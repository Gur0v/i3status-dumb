use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};

fn parse_default_sink(output: &str) -> Option<String> {
    let sink = output.trim();
    if sink.is_empty() {
        None
    } else {
        Some(sink.to_owned())
    }
}

fn parse_pactl_volume(output: &str) -> Option<u32> {
    output
        .split('/')
        .nth(1)?
        .trim()
        .strip_suffix('%')?
        .trim()
        .parse::<u32>()
        .ok()
}

fn parse_pactl_mute(output: &str) -> Option<bool> {
    let state = output.split_once(':')?.1.trim();
    match state {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

async fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn read_default_sink() -> Option<String> {
    let output = command_output("pactl", &["get-default-sink"]).await?;
    parse_default_sink(&output)
}

async fn read_volume() -> Option<String> {
    let sink = read_default_sink().await?;
    let volume_output = command_output("pactl", &["get-sink-volume", &sink]).await?;
    let mute_output = command_output("pactl", &["get-sink-mute", &sink]).await?;

    let muted = parse_pactl_mute(&mute_output)?;
    let volume = parse_pactl_volume(&volume_output)?;

    if muted {
        Some(String::from("0%"))
    } else {
        Some(format!("{volume}%"))
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
