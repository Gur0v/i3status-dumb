use chrono::Local;
use tokio::sync::watch;
use tokio::time::{interval, Duration};

pub fn now_string() -> String {
    Local::now().format("%Y-%m-%d %I:%M:%S %p").to_string()
}

pub fn spawn(tx: watch::Sender<String>) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(1));

        loop {
            tick.tick().await;
            let next = now_string();
            let _ = tx.send_if_modified(|current| {
                if *current == next {
                    false
                } else {
                    *current = next.clone();
                    true
                }
            });
        }
    });
}
