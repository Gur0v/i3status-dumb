use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};

const DEFAULT_SINK: &str = "@DEFAULT_AUDIO_SINK@";

fn parse_wpctl_output(output: &str) -> Option<String> {
    let muted = output.contains("[MUTED]");
    let volume = output
        .split_whitespace()
        .find_map(|part| part.parse::<f32>().ok())?;

    if muted {
        Some(String::from("0%"))
    } else {
        Some(format!("{}%", (volume * 100.0).round() as u32))
    }
}

async fn read_volume() -> Option<String> {
    let output = Command::new("wpctl")
        .args(["get-volume", DEFAULT_SINK])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    parse_wpctl_output(&String::from_utf8_lossy(&output.stdout))
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

fn should_refresh(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("on sink")
        || lower.contains("on server")
        || lower.contains("on card")
        || lower.contains("on source")
}

async fn forward_current_volume(tx: &watch::Sender<String>) {
    if let Some(volume) = read_volume().await {
        publish(tx, volume);
    }
}

async fn watch_subscription(tx: &watch::Sender<String>) -> std::io::Result<()> {
    let mut child = Command::new("pactl")
        .arg("subscribe")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill().await;
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "missing pactl stdout",
        ));
    };

    let mut lines = BufReader::new(stdout).lines();
    forward_current_volume(tx).await;

    while let Some(line) = lines.next_line().await? {
        if should_refresh(&line) {
            forward_current_volume(tx).await;
        }
    }

    let _ = child.wait().await;
    Ok(())
}

pub fn spawn(tx: watch::Sender<String>) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = watch_subscription(&tx).await {
                eprintln!("i3status-dumb: volume watcher failed: {error}");
            }

            sleep(Duration::from_secs(1)).await;
        }
    });
}
