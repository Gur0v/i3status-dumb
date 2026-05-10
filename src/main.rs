mod clock;
mod layout;
mod status;
mod util;
mod volume;

use std::env;
use std::ffi::OsString;
use std::io::{self, LineWriter, Write};
use std::process;

use status::{LayoutState, VolumeState};
use tokio::sync::watch;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn write_line(stdout: &mut LineWriter<io::StdoutLock<'_>>, line: &str) -> io::Result<()> {
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\n")
}

fn sync_receivers(
    vol_rx: &mut watch::Receiver<VolumeState>,
    layout_rx: &mut watch::Receiver<LayoutState>,
    time_rx: &mut watch::Receiver<status::ClockState>,
) -> (VolumeState, LayoutState, status::ClockState) {
    let volume = *vol_rx.borrow_and_update();
    while vol_rx.has_changed().unwrap_or(false) {
        vol_rx.borrow_and_update();
    }

    let layout = *layout_rx.borrow_and_update();
    while layout_rx.has_changed().unwrap_or(false) {
        layout_rx.borrow_and_update();
    }

    let time = *time_rx.borrow_and_update();
    while time_rx.has_changed().unwrap_or(false) {
        time_rx.borrow_and_update();
    }

    (volume, layout, time)
}

enum Mode {
    Watch,
    Once,
    Version,
}

fn mode_from_args(mut args: impl Iterator<Item = OsString>) -> Result<Mode, String> {
    let _program = args.next();

    match args.next() {
        None => Ok(Mode::Watch),
        Some(arg) if arg == "--once" => {
            if args.next().is_some() {
                Err(String::from("too many arguments"))
            } else {
                Ok(Mode::Once)
            }
        }
        Some(arg) if arg == "--version" => {
            if args.next().is_some() {
                Err(String::from("too many arguments"))
            } else {
                Ok(Mode::Version)
            }
        }
        Some(arg) => Err(format!("unknown argument: {}", arg.to_string_lossy())),
    }
}

async fn run_once() -> io::Result<()> {
    let volume = tokio::task::spawn_blocking(volume::current)
        .await
        .unwrap_or(VolumeState::UNKNOWN);
    let layout = layout::current().await;
    let time = clock::now();

    let stdout = io::stdout();
    let mut stdout = LineWriter::new(stdout.lock());
    let mut line = String::with_capacity(32);
    write_line(
        &mut stdout,
        status::render_into(&mut line, volume, layout, time),
    )
}

async fn run_watch() {
    let (vol_tx, vol_rx) = watch::channel(VolumeState::UNKNOWN);
    let (layout_tx, layout_rx) = watch::channel(LayoutState::UNKNOWN);
    let (time_tx, time_rx) = watch::channel(clock::now());

    clock::spawn(time_tx);
    volume::spawn(vol_tx);
    layout::spawn(layout_tx);

    let mut vol_rx = vol_rx;
    let mut layout_rx = layout_rx;
    let mut time_rx = time_rx;
    let stdout = io::stdout();
    let mut stdout = LineWriter::new(stdout.lock());
    let mut line = String::with_capacity(32);

    if write_line(
        &mut stdout,
        status::render_into(
            &mut line,
            *vol_rx.borrow(),
            *layout_rx.borrow(),
            *time_rx.borrow(),
        ),
    )
    .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            changed = vol_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            changed = layout_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            changed = time_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }

        let (volume, layout, time) = sync_receivers(&mut vol_rx, &mut layout_rx, &mut time_rx);

        if write_line(
            &mut stdout,
            status::render_into(&mut line, volume, layout, time),
        )
        .is_err()
        {
            break;
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    match mode_from_args(env::args_os()) {
        Ok(Mode::Watch) => run_watch().await,
        Ok(Mode::Once) => {
            if run_once().await.is_err() {
                process::exit(1);
            }
        }
        Ok(Mode::Version) => println!("i3status-dumb {VERSION}"),
        Err(error) => {
            eprintln!("i3status-dumb: {}", error.to_lowercase());
            process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mode_from_args, Mode};
    use std::ffi::OsString;

    fn mode(args: &[&str]) -> Result<Mode, String> {
        mode_from_args(args.iter().map(OsString::from))
    }

    #[test]
    fn defaults_to_watch_mode() {
        assert!(matches!(mode(&["i3status-dumb"]), Ok(Mode::Watch)));
    }

    #[test]
    fn parses_once_mode() {
        assert!(matches!(mode(&["i3status-dumb", "--once"]), Ok(Mode::Once)));
    }

    #[test]
    fn parses_version_mode() {
        assert!(matches!(
            mode(&["i3status-dumb", "--version"]),
            Ok(Mode::Version)
        ));
    }

    #[test]
    fn rejects_unknown_argument() {
        assert!(mode(&["i3status-dumb", "--wat"]).is_err());
    }
}
