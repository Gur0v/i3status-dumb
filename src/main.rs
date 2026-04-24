mod clock;
mod layout;
mod status;
mod volume;

use std::io::{self, LineWriter, Write};

use status::{LayoutState, VolumeState};
use tokio::sync::watch;

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

#[tokio::main]
async fn main() {
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

        if write_line(&mut stdout, status::render_into(&mut line, volume, layout, time)).is_err() {
            break;
        }
    }
}
