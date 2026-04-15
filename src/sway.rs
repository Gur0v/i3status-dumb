use std::env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};

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

fn parse_layout(json: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;

    let arr = if v.is_array() {
        v
    } else {
        let input = v.get("input")?;
        serde_json::Value::Array(vec![input.clone()])
    };

    for item in arr.as_array()? {
        let typ = item.get("type")?.as_str().unwrap_or("");
        if typ != "keyboard" {
            continue;
        }
        let name = item
            .get("xkb_active_layout_name")
            .and_then(|n| n.as_str())
            .unwrap_or("");
        if !name.is_empty() {
            return Some(layout_to_code(name));
        }
    }
    None
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

async fn sync_initial_layout(stream: &mut UnixStream, tx: &watch::Sender<String>) {
    if send_msg(stream, TYPE_GET_INPUTS, b"").await.is_err() {
        return;
    }

    if let Ok((_, body)) = recv_msg(stream).await {
        if let Some(code) = parse_layout(&body) {
            publish(tx, code);
        }
    }
}

async fn subscribe_to_inputs(stream: &mut UnixStream) -> bool {
    if send_msg(stream, TYPE_SUBSCRIBE, br#"["input"]"#)
        .await
        .is_err()
    {
        return false;
    }

    recv_msg(stream).await.is_ok()
}

async fn watch_connection(mut stream: UnixStream, tx: &watch::Sender<String>) {
    sync_initial_layout(&mut stream, tx).await;

    if !subscribe_to_inputs(&mut stream).await {
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

        if let Some(code) = parse_layout(&body) {
            publish(tx, code);
        }
    }
}

pub fn spawn(tx: watch::Sender<String>) {
    tokio::spawn(async move {
        let sock_path = match env::var("SWAYSOCK") {
            Ok(p) => p,
            Err(_) => {
                eprintln!("i3status-dumb: SWAYSOCK not set");
                return;
            }
        };

        loop {
            match UnixStream::connect(&sock_path).await {
                Ok(stream) => {
                    watch_connection(stream, &tx).await;
                }
                Err(e) => {
                    eprintln!("i3status-dumb: sway IPC connect failed: {e}");
                }
            }
            sleep(Duration::from_secs(1)).await;
        }
    });
}
