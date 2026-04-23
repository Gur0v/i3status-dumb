mod clock;
mod layout;
mod status;
mod volume;

use std::io::{self, BufWriter, Write};

use status::{LayoutState, VolumeState};
use tokio::sync::watch;

fn write_line(stdout: &mut BufWriter<io::StdoutLock<'_>>, line: &str) -> io::Result<()> {
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()
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
    let mut stdout = BufWriter::new(stdout.lock());
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

        if write_line(
            &mut stdout,
            status::render_into(
                &mut line,
                *vol_rx.borrow_and_update(),
                *layout_rx.borrow_and_update(),
                *time_rx.borrow_and_update(),
            ),
        )
        .is_err()
        {
            break;
        }
    }
}
