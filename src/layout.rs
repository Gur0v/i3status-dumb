use std::env;
use std::io;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{interval, sleep, timeout, Duration};

use crate::status::LayoutState;

const MAGIC: &[u8] = b"i3-ipc";
const TYPE_SUBSCRIBE: u32 = 2;
const TYPE_GET_INPUTS: u32 = 4;
const EVENT_INPUT: u32 = 0x80000015;
const IPC_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_TIMEOUT: Duration = Duration::from_millis(750);
const RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_IPC_PAYLOAD_LEN: usize = 1024 * 1024;

#[derive(Deserialize)]
struct SwayInput {
    #[serde(rename = "type")]
    device_type: String,
    xkb_active_layout_name: Option<String>,
    xkb_layout_names: Option<Vec<String>>,
    xkb_active_layout_index: Option<usize>,
}

#[derive(Deserialize)]
struct SwayInputEvent {
    change: String,
    input: SwayInput,
}

fn publish(tx: &watch::Sender<LayoutState>, next: LayoutState) {
    let _ = tx.send_if_modified(|current| {
        if *current == next {
            false
        } else {
            *current = next;
            true
        }
    });
}

fn needs_retry(tx: &watch::Sender<LayoutState>) -> bool {
    tx.borrow().is_unknown()
}

fn detect_backend() -> LayoutBackend {
    let has_wayland = env::var_os("WAYLAND_DISPLAY").is_some();
    let has_x11 = env::var_os("DISPLAY").is_some();

    if has_wayland {
        if let Ok(path) = env::var("SWAYSOCK") {
            return LayoutBackend::Sway(path);
        }
    }

    if has_x11 {
        return LayoutBackend::X11;
    }

    if let Ok(path) = env::var("SWAYSOCK") {
        return LayoutBackend::Sway(path);
    }

    LayoutBackend::X11
}

async fn send_msg(stream: &mut UnixStream, typ: u32, payload: &[u8]) -> std::io::Result<()> {
    timeout(IPC_TIMEOUT, async {
        stream.write_all(MAGIC).await?;
        stream
            .write_all(&(payload.len() as u32).to_le_bytes())
            .await?;
        stream.write_all(&typ.to_le_bytes()).await?;
        stream.write_all(payload).await?;
        Ok(())
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "sway IPC write timed out"))?
}

async fn recv_msg(stream: &mut UnixStream) -> std::io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 14];
    timeout(IPC_TIMEOUT, stream.read_exact(&mut header))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "sway IPC read timed out"))??;

    if &header[..6] != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid sway IPC magic",
        ));
    }

    let len = u32::from_le_bytes([header[6], header[7], header[8], header[9]]) as usize;
    if len > MAX_IPC_PAYLOAD_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sway IPC payload too large",
        ));
    }

    let typ = u32::from_le_bytes([header[10], header[11], header[12], header[13]]);
    let mut body = vec![0u8; len];
    timeout(IPC_TIMEOUT, stream.read_exact(&mut body))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "sway IPC body read timed out"))??;
    Ok((typ, body))
}

fn layout_from_input(input: &SwayInput) -> Option<LayoutState> {
    if input.device_type != "keyboard" {
        return None;
    }

    if let Some(name) = input
        .xkb_active_layout_name
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        return Some(LayoutState::from_name(name));
    }

    let names = input.xkb_layout_names.as_ref()?;
    let index = input.xkb_active_layout_index.unwrap_or(0);
    names
        .get(index)
        .or_else(|| names.first())
        .filter(|name| !name.is_empty())
        .map(|name| LayoutState::from_name(name))
}

fn parse_sway_layouts(json: &[u8]) -> Option<LayoutState> {
    let inputs = serde_json::from_slice::<Vec<SwayInput>>(json).ok()?;
    inputs.iter().find_map(layout_from_input)
}

fn parse_sway_event(json: &[u8]) -> Option<LayoutState> {
    let event = serde_json::from_slice::<SwayInputEvent>(json).ok()?;

    if event.change != "xkb_layout" && !event.change.is_empty() {
        return None;
    }

    layout_from_input(&event.input)
}

async fn sync_initial_sway_layout(stream: &mut UnixStream, tx: &watch::Sender<LayoutState>) {
    if send_msg(stream, TYPE_GET_INPUTS, b"").await.is_err() {
        if let Some(code) = read_sway_layout_via_command().await {
            publish(tx, code);
        }
        return;
    }

    if let Ok((_, body)) = recv_msg(stream).await {
        if let Some(code) = parse_sway_layouts(&body) {
            publish(tx, code);
            return;
        }
    }

    if let Some(code) = read_sway_layout_via_command().await {
        publish(tx, code);
    }
}

async fn subscribe_to_sway_inputs(stream: &mut UnixStream) -> bool {
    if send_msg(stream, TYPE_SUBSCRIBE, br#"["input"]"#)
        .await
        .is_err()
    {
        return false;
    }

    matches!(
        recv_msg(stream).await,
        Ok((_, body)) if body == br#"{"success":true}"#
    )
}

async fn watch_sway_connection(mut stream: UnixStream, tx: &watch::Sender<LayoutState>) {
    let mut retry_tick = interval(RETRY_DELAY);
    sync_initial_sway_layout(&mut stream, tx).await;

    if !subscribe_to_sway_inputs(&mut stream).await {
        return;
    }

    loop {
        tokio::select! {
            result = recv_msg(&mut stream) => {
                let Ok((typ, body)) = result else {
                    return;
                };

                if typ != EVENT_INPUT {
                    continue;
                }

                if let Some(code) = parse_sway_event(&body) {
                    publish(tx, code);
                }
            }
            _ = retry_tick.tick() => {
                if needs_retry(tx) {
                    sync_initial_sway_layout(&mut stream, tx).await;
                }
            }
        }
    }
}

async fn watch_sway(sock_path: String, tx: watch::Sender<LayoutState>) {
    loop {
        match timeout(IPC_TIMEOUT, UnixStream::connect(&sock_path)).await {
            Ok(Ok(stream)) => {
                watch_sway_connection(stream, &tx).await;
            }
            Ok(Err(error)) => {
                eprintln!("i3status-dumb: sway IPC connect failed: {error}");
            }
            Err(_) => {
                eprintln!("i3status-dumb: sway IPC connect timed out");
            }
        }

        sleep(RETRY_DELAY).await;
    }
}

fn parse_x11_layout(output: &[u8]) -> Option<LayoutState> {
    let text = std::str::from_utf8(output).ok()?;
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("layout:"))?;

    let raw = line.split_once(':')?.1.trim();
    let first = raw.split(',').next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(LayoutState::from_name(first))
    }
}

async fn read_x11_layout() -> Option<LayoutState> {
    let output = timeout(
        COMMAND_TIMEOUT,
        Command::new("setxkbmap").arg("-query").output(),
    )
    .await
    .ok()?
    .ok()?;

    if !output.status.success() {
        return None;
    }

    parse_x11_layout(&output.stdout)
}

async fn read_sway_layout_via_command() -> Option<LayoutState> {
    let output = timeout(
        COMMAND_TIMEOUT,
        Command::new("swaymsg")
            .args(["-r", "-t", "get_inputs"])
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    if !output.status.success() {
        return None;
    }

    parse_sway_layouts(&output.stdout)
}

async fn watch_x11(tx: watch::Sender<LayoutState>) {
    let mut tick = interval(Duration::from_secs(1));

    loop {
        tick.tick().await;

        if let Some(layout) = read_x11_layout().await {
            publish(&tx, layout);
        }
    }
}

enum LayoutBackend {
    Sway(String),
    X11,
}

pub fn spawn(tx: watch::Sender<LayoutState>) {
    tokio::spawn(async move {
        match detect_backend() {
            LayoutBackend::Sway(sock_path) => watch_sway(sock_path, tx).await,
            LayoutBackend::X11 => watch_x11(tx).await,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{parse_sway_event, parse_sway_layouts, parse_x11_layout, MAGIC};
    use crate::status::LayoutState;

    #[test]
    fn parses_x11_layout_line() {
        let output = b"rules:      evdev\nlayout:     us,ru\nvariant:    ,\n";
        assert_eq!(
            parse_x11_layout(output),
            Some(LayoutState::from_ascii("us"))
        );
    }

    #[test]
    fn parses_sway_input_payloads() {
        let json = br#"[
            {"type":"pointer","xkb_active_layout_name":"ignored"},
            {"type":"keyboard","xkb_active_layout_name":"English (US)"}
        ]"#;

        assert_eq!(
            parse_sway_layouts(json),
            Some(LayoutState::from_ascii("us"))
        );
    }

    #[test]
    fn parses_sway_get_inputs_fallback_fields() {
        let json = br#"[
            {
                "type":"keyboard",
                "xkb_active_layout_index":1,
                "xkb_layout_names":["English (US)","Ukrainian"]
            }
        ]"#;

        assert_eq!(
            parse_sway_layouts(json),
            Some(LayoutState::from_ascii("ua"))
        );
    }

    #[test]
    fn parses_sway_events() {
        let json = br#"{
            "change":"xkb_layout",
            "input":{"type":"keyboard","xkb_active_layout_name":"Russian"}
        }"#;

        assert_eq!(parse_sway_event(json), Some(LayoutState::from_ascii("ru")));
    }

    #[test]
    fn ipc_magic_constant_stays_expected() {
        assert_eq!(MAGIC, b"i3-ipc");
    }
}
