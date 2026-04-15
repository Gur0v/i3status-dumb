mod clock;
mod status;
mod sway;
mod volume;

use tokio::sync::watch;

#[tokio::main]
async fn main() {
    let (vol_tx, vol_rx) = watch::channel(String::from("??%"));
    let (layout_tx, layout_rx) = watch::channel(String::from("??"));
    let (time_tx, time_rx) = watch::channel(clock::now_string());

    clock::spawn(time_tx);
    volume::spawn(vol_tx);
    sway::spawn(layout_tx);

    let mut vol_rx = vol_rx;
    let mut layout_rx = layout_rx;
    let mut time_rx = time_rx;

    println!(
        "{}",
        status::render(&vol_rx.borrow(), &layout_rx.borrow(), &time_rx.borrow())
    );

    loop {
        tokio::select! {
            Ok(_) = vol_rx.changed()    => {}
            Ok(_) = layout_rx.changed() => {}
            Ok(_) = time_rx.changed()   => {}
        }
        println!(
            "{}",
            status::render(
                &vol_rx.borrow_and_update(),
                &layout_rx.borrow_and_update(),
                &time_rx.borrow_and_update(),
            )
        );
    }
}
