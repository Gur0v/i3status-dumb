use futures_util::StreamExt;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};

use swayipc_async::{Connection, Event, EventType, Input};

use crate::status::LayoutState;

const RETRY_DELAY: Duration = Duration::from_secs(1);

fn publish(tx: &watch::Sender<LayoutState>, next: LayoutState) {
    let _ = tx.send_if_modified(|current| {
        if *current == next {
            false
        } else {
            *current = next;
            true
        }
    });
}

fn layout_from_input(input: &Input) -> Option<LayoutState> {
    if input.input_type != "keyboard" {
        return None;
    }

    if let Some(name) = input
        .xkb_active_layout_name
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        return Some(LayoutState::from_name(name));
    }

    let index = usize::try_from(input.xkb_active_layout_index.unwrap_or(0)).unwrap_or(0);
    input.xkb_layout_names
        .get(index)
        .or_else(|| input.xkb_layout_names.first())
        .filter(|name| !name.is_empty())
        .map(|name| LayoutState::from_name(name))
}

fn layout_from_inputs(inputs: &[Input]) -> Option<LayoutState> {
    inputs.iter().find_map(layout_from_input)
}

async fn connect_and_subscribe(
    tx: &watch::Sender<LayoutState>,
) -> swayipc_async::Fallible<swayipc_async::EventStream> {
    let mut connection = Connection::new().await?;
    let inputs = connection.get_inputs().await?;
    if let Some(code) = layout_from_inputs(&inputs) {
        publish(tx, code);
    }
    connection.subscribe([EventType::Input]).await
}

pub fn spawn(tx: watch::Sender<LayoutState>) {
    tokio::spawn(async move {
        loop {
            let mut events = match connect_and_subscribe(&tx).await {
                Ok(events) => events,
                Err(error) => {
                    eprintln!("i3status-dumb: sway watcher setup failed: {error}");
                    sleep(RETRY_DELAY).await;
                    continue;
                }
            };

            while let Some(event) = events.next().await {
                match event {
                    Ok(Event::Input(event)) => {
                        if let Some(code) = layout_from_input(&event.input) {
                            publish(&tx, code);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("i3status-dumb: sway event stream failed: {error}");
                        break;
                    }
                }
            }

            sleep(RETRY_DELAY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::status::LayoutState;

    fn layout_name(
        device_type: &str,
        active_name: Option<&str>,
        layout_names: &[&str],
        active_index: Option<i32>,
    ) -> Option<LayoutState> {
        if device_type != "keyboard" {
            return None;
        }

        if let Some(name) = active_name.filter(|name| !name.is_empty()) {
            return Some(LayoutState::from_name(name));
        }

        let index = usize::try_from(active_index.unwrap_or(0)).unwrap_or(0);
        layout_names
            .get(index)
            .or_else(|| layout_names.first())
            .copied()
            .filter(|name| !name.is_empty())
            .map(LayoutState::from_name)
    }

    #[test]
    fn reads_first_keyboard_layout_from_sway_inputs() {
        assert_eq!(
            layout_name("keyboard", Some("English (US)"), &[], None),
            Some(LayoutState::from_ascii("us"))
        );
    }

    #[test]
    fn uses_layout_list_when_active_name_is_missing() {
        assert_eq!(
            layout_name(
                "keyboard",
                None,
                &["English (US)", "Ukrainian"],
                Some(1)
            ),
            Some(LayoutState::from_ascii("ua"))
        );
    }
}
