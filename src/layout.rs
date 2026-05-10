use futures_util::StreamExt;
use tokio::sync::watch;

use swayipc_async::{Connection, Event, EventType, Input};

use crate::status::LayoutState;
use crate::util::{log_error, publish_if_changed, RetryBackoff};

fn layout_name(
    input_type: &str,
    active_name: Option<&str>,
    layout_names: &[String],
    active_index: Option<i32>,
) -> Option<LayoutState> {
    if input_type != "keyboard" {
        return None;
    }

    if let Some(name) = active_name.filter(|name| !name.is_empty()) {
        return Some(LayoutState::from_name(name));
    }

    let index = usize::try_from(active_index.unwrap_or(0)).unwrap_or(0);
    layout_names
        .get(index)
        .or_else(|| layout_names.first())
        .filter(|name| !name.is_empty())
        .map(|name| LayoutState::from_name(name))
}

fn layout_from_input(input: &Input) -> Option<LayoutState> {
    layout_name(
        &input.input_type,
        input.xkb_active_layout_name.as_deref(),
        &input.xkb_layout_names,
        input.xkb_active_layout_index,
    )
}

fn layout_from_inputs(inputs: &[Input]) -> Option<LayoutState> {
    inputs.iter().find_map(layout_from_input)
}

pub async fn current() -> LayoutState {
    match Connection::new().await {
        Ok(mut connection) => match connection.get_inputs().await {
            Ok(inputs) => layout_from_inputs(&inputs).unwrap_or(LayoutState::UNKNOWN),
            Err(error) => {
                log_error("sway input query failed", error);
                LayoutState::UNKNOWN
            }
        },
        Err(error) => {
            log_error("sway connection failed", error);
            LayoutState::UNKNOWN
        }
    }
}

async fn connect_and_subscribe(
    tx: &watch::Sender<LayoutState>,
) -> swayipc_async::Fallible<swayipc_async::EventStream> {
    let mut connection = Connection::new().await?;
    let inputs = connection.get_inputs().await?;
    if let Some(code) = layout_from_inputs(&inputs) {
        publish_if_changed(tx, code);
    }
    connection.subscribe([EventType::Input]).await
}

pub fn spawn(tx: watch::Sender<LayoutState>) {
    tokio::spawn(async move {
        let mut retry = RetryBackoff::new();

        loop {
            let mut events = match connect_and_subscribe(&tx).await {
                Ok(events) => {
                    retry.reset();
                    events
                }
                Err(error) => {
                    publish_if_changed(&tx, LayoutState::UNKNOWN);
                    log_error("sway watcher setup failed", error);
                    retry.wait().await;
                    continue;
                }
            };

            while let Some(event) = events.next().await {
                match event {
                    Ok(Event::Input(event)) => {
                        if let Some(code) = layout_from_input(&event.input) {
                            publish_if_changed(&tx, code);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        publish_if_changed(&tx, LayoutState::UNKNOWN);
                        log_error("sway event stream failed", error);
                        break;
                    }
                }
            }

            retry.wait().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::layout_name;
    use crate::status::LayoutState;

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
                &["English (US)".into(), "Ukrainian".into()],
                Some(1)
            ),
            Some(LayoutState::from_ascii("ua"))
        );
    }
}
