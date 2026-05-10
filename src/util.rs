use std::fmt;

use tokio::sync::watch;
use tokio::time::{sleep, Duration};

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

pub fn publish_if_changed<T>(tx: &watch::Sender<T>, next: T)
where
    T: Copy + PartialEq,
{
    let _ = tx.send_if_modified(|current| {
        if *current == next {
            false
        } else {
            *current = next;
            true
        }
    });
}

pub fn log_error(context: &str, error: impl fmt::Display) {
    eprintln!(
        "i3status-dumb: {context}: {}",
        error.to_string().to_lowercase()
    );
}

pub struct RetryBackoff {
    next: Duration,
}

impl RetryBackoff {
    pub const fn new() -> Self {
        Self {
            next: INITIAL_RETRY_DELAY,
        }
    }

    pub fn reset(&mut self) {
        self.next = INITIAL_RETRY_DELAY;
    }

    pub async fn wait(&mut self) {
        sleep(self.next).await;
        self.next = (self.next * 2).min(MAX_RETRY_DELAY);
    }
}
