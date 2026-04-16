use std::env;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{interval, sleep, Duration};

const MAGIC: &[u8] = b"i3-ipc";
const TYPE_SUBSCRIBE: u32 = 2;
const TYPE_GET_INPUTS: u32 = 4;
const EVENT_INPUT: u32 = 0x80000015;

fn layout_to_code(name: &str) -> String {
    if name.contains("English (US)") {
        "us".into()
    } else if name.contains("Russian") {
        "ru".into()
    } else if name.contains("Ukrainian") {
        "ua".into()
    } else {
        name.chars().take(3).collect::<String>().to_lowercase()
    }
}

fn publish(tx: &watch::Sender<String>, next: String) {
    let _ = tx.send_if_modified(|current| {
        if *current == next {
            false
        } else {
            *current = next.clone();
            true
        }
    });
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
    stream.write_all(MAGIC).await?;
    stream
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await?;
    stream.write_all(&typ.to_le_bytes()).await?;
    stream.write_all(payload).await?;
    Ok(())
}

async fn recv_msg(stream: &mut UnixStream) -> std::io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 14];
    stream.read_exact(&mut header).await?;
    let len = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
    let typ = u32::from_le_bytes(header[10..14].try_into().unwrap());
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok((typ, body))
}

fn parse_sway_layout(json: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(json).ok()?;

    let inputs = if value.is_array() {
        value
    } else {
        let input = value.get("input")?;
        serde_json::Value::Array(vec![input.clone()])
    };

    for item in inputs.as_array()? {
        let device_type = item.get("type")?.as_str().unwrap_or("");
        if device_type != "keyboard" {
            continue;
        }

        let name = item
            .get("xkb_active_layout_name")
            .and_then(|field| field.as_str())
            .unwrap_or("");

        if !name.is_empty() {
            return Some(layout_to_code(name));
        }
    }

    None
}

async fn sync_initial_sway_layout(stream: &mut UnixStream, tx: &watch::Sender<String>) {
    if send_msg(stream, TYPE_GET_INPUTS, b"").await.is_err() {
        return;
    }

    if let Ok((_, body)) = recv_msg(stream).await {
        if let Some(code) = parse_sway_layout(&body) {
            publish(tx, code);
        }
    }
}

async fn subscribe_to_sway_inputs(stream: &mut UnixStream) -> bool {
    if send_msg(stream, TYPE_SUBSCRIBE, br#"["input"]"#)
        .await
        .is_err()
    {
        return false;
    }

    recv_msg(stream).await.is_ok()
}

async fn watch_sway_connection(mut stream: UnixStream, tx: &watch::Sender<String>) {
    sync_initial_sway_layout(&mut stream, tx).await;

    if !subscribe_to_sway_inputs(&mut stream).await {
        return;
    }

    loop {
        let Ok((typ, body)) = recv_msg(&mut stream).await else {
            return;
        };

        if typ != EVENT_INPUT {
            continue;
        }

        let change = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("change")
                    .and_then(|field| field.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_default();

        if change != "xkb_layout" && !change.is_empty() {
            continue;
        }

        if let Some(code) = parse_sway_layout(&body) {
            publish(tx, code);
        }
    }
}

async fn watch_sway(sock_path: String, tx: watch::Sender<String>) {
    loop {
        match UnixStream::connect(&sock_path).await {
            Ok(stream) => {
                watch_sway_connection(stream, &tx).await;
            }
            Err(error) => {
                eprintln!("i3status-dumb: sway IPC connect failed: {error}");
            }
        }

        sleep(Duration::from_secs(1)).await;
    }
}

fn parse_x11_layout(output: &str) -> Option<String> {
    let line = output
        .lines()
        .find(|line| line.trim_start().starts_with("layout:"))?;

    let raw = line.split_once(':')?.1.trim();
    let first = raw.split(',').next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_lowercase())
    }
}

async fn read_x11_layout() -> Option<String> {
    let output = Command::new("setxkbmap").arg("-query").output().await.ok()?;
    if !output.status.success() {
        return None;
    }

    parse_x11_layout(&String::from_utf8_lossy(&output.stdout))
}

async fn watch_x11(tx: watch::Sender<String>) {
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

pub fn spawn(tx: watch::Sender<String>) {
    tokio::spawn(async move {
        match detect_backend() {
            LayoutBackend::Sway(sock_path) => watch_sway(sock_path, tx).await,
            LayoutBackend::X11 => watch_x11(tx).await,
        }
    });
}
