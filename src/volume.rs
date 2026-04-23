use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{sleep, timeout, Duration};

use crate::status::VolumeState;

const COMMAND_TIMEOUT: Duration = Duration::from_millis(750);
const RETRY_DELAY: Duration = Duration::from_secs(1);

fn parse_pactl_volume(output: &[u8]) -> Option<u16> {
    let text = std::str::from_utf8(output).ok()?;
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        let start = index;
        let mut value = 0u16;

        while index < bytes.len() && bytes[index].is_ascii_digit() {
            value = value
                .saturating_mul(10)
                .saturating_add((bytes[index] - b'0') as u16);
            index += 1;
        }

        if index < bytes.len()
            && bytes[index] == b'%'
            && start > 0
            && bytes[start - 1].is_ascii_whitespace()
        {
            return Some(value.min(999));
        }
    }

    None
}

fn parse_pactl_mute(output: &[u8]) -> Option<bool> {
    let text = std::str::from_utf8(output).ok()?;
    let (_, state) = text.split_once(':')?;
    match state.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

async fn command_output(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    let output = timeout(COMMAND_TIMEOUT, Command::new(program).args(args).output())
        .await
        .ok()?
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(output.stdout)
}

async fn read_volume() -> Option<VolumeState> {
    let volume_output = command_output("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]).await?;
    let mute_output = command_output("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]).await?;

    Some(VolumeState::new(
        parse_pactl_volume(&volume_output)?,
        parse_pactl_mute(&mute_output)?,
    ))
}

fn publish(tx: &watch::Sender<VolumeState>, next: VolumeState) {
    let _ = tx.send_if_modified(|current| {
        if *current == next {
            false
        } else {
            *current = next;
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

async fn forward_current_volume(tx: &watch::Sender<VolumeState>) {
    if let Some(volume) = read_volume().await {
        publish(tx, volume);
    }
}

async fn watch_subscription(tx: &watch::Sender<VolumeState>) -> std::io::Result<()> {
    let mut child = Command::new("pactl")
        .arg("subscribe")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill().await;
        return Err(std::io::Error::other("missing pactl stdout"));
    };

    let mut lines = BufReader::new(stdout).lines();
    forward_current_volume(tx).await;

    while let Some(line) = lines.next_line().await? {
        if should_refresh(&line) {
            forward_current_volume(tx).await;
        }
    }

    match child.wait().await {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            format!("pactl subscribe exited with status {status}"),
        )),
        Err(error) => Err(error),
    }
}

pub fn spawn(tx: watch::Sender<VolumeState>) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = watch_subscription(&tx).await {
                eprintln!("i3status-dumb: volume watcher failed: {error}");
            }

            sleep(RETRY_DELAY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{parse_pactl_mute, parse_pactl_volume};

    #[test]
    fn parses_first_percentage_token() {
        assert_eq!(
            parse_pactl_volume(b"Volume: front-left: 32768 /  50% / -18.06 dB"),
            Some(50)
        );
    }

    #[test]
    fn parses_large_values_without_allocating() {
        assert_eq!(
            parse_pactl_volume(b"Volume: mono: 65536 / 150% / 0.00 dB"),
            Some(150)
        );
    }

    #[test]
    fn parses_muted_variants() {
        assert_eq!(parse_pactl_mute(b"Mute: yes"), Some(true));
        assert_eq!(parse_pactl_mute(b"Mute: false"), Some(false));
    }
}
