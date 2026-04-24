use std::sync::mpsc;

use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::introspect::SinkInfo;
use pulse::context::subscribe::{Facility, InterestMaskSet, Operation};
use pulse::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::proplist::Proplist;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};

use crate::status::VolumeState;

const RETRY_DELAY: Duration = Duration::from_secs(1);

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

fn percent_from_raw(avg: u32, normal: u32) -> u16 {
    let normal = normal.max(1);
    let rounded = avg
        .saturating_mul(100)
        .saturating_add(normal / 2)
        / normal;
    rounded.min(999) as u16
}

fn volume_from_sink(info: &SinkInfo<'_>) -> VolumeState {
    let avg = info.volume.avg().0;
    let normal = pulse::volume::Volume::NORMAL.0;
    VolumeState::new(percent_from_raw(avg, normal), info.mute)
}

fn iterate(mainloop: &mut Mainloop) -> Result<(), String> {
    match mainloop.iterate(true) {
        IterateResult::Quit(_) | IterateResult::Err(_) => Err(String::from("pulse mainloop stopped")),
        IterateResult::Success(_) => Ok(()),
    }
}

fn wait_for_context_ready(mainloop: &mut Mainloop, context: &Context) -> Result<(), String> {
    loop {
        match context.get_state() {
            ContextState::Ready => return Ok(()),
            ContextState::Failed | ContextState::Terminated => {
                return Err(String::from("pulse context failed"))
            }
            _ => iterate(mainloop)?,
        }
    }
}

fn wait_for_operation<T: ?Sized>(
    mainloop: &mut Mainloop,
    operation: pulse::operation::Operation<T>,
) -> Result<(), String> {
    while operation.get_state() == pulse::operation::State::Running {
        iterate(mainloop)?;
    }
    Ok(())
}

fn request_default_sink_volume(
    context: &Context,
    mainloop: &mut Mainloop,
    out: &watch::Sender<VolumeState>,
) -> Result<(), String> {
    let (server_tx, server_rx) = mpsc::channel();
    let operation = context.introspect().get_server_info(move |info| {
        let default = info.default_sink_name.as_ref().map(|name| name.to_string());
        let _ = server_tx.send(default);
    });
    wait_for_operation(mainloop, operation)?;

    let Some(default_sink) = server_rx.recv().map_err(|error| error.to_string())? else {
        return Ok(());
    };

    let (sink_tx, sink_rx) = mpsc::channel();
    let operation = context
        .introspect()
        .get_sink_info_by_name(&default_sink, move |result| match result {
            ListResult::Item(info) => {
                let _ = sink_tx.send(volume_from_sink(info));
            }
            ListResult::End | ListResult::Error => {}
        });
    wait_for_operation(mainloop, operation)?;

    if let Some(volume) = sink_rx.try_iter().last() {
        publish(out, volume);
    }

    Ok(())
}

fn should_refresh(facility: Option<Facility>, _operation: Option<Operation>) -> bool {
    matches!(
        facility,
        Some(Facility::Sink) | Some(Facility::Server) | Some(Facility::Card)
    )
}

fn run_pulse_loop(out: watch::Sender<VolumeState>) -> Result<(), String> {
    let mut proplist = Proplist::new().ok_or_else(|| String::from("failed to create proplist"))?;
    proplist
        .set_str(pulse::proplist::properties::APPLICATION_NAME, "i3status-dumb")
        .map_err(|_| String::from("failed to set pulse application name"))?;

    let mut mainloop =
        Mainloop::new().ok_or_else(|| String::from("failed to create pulse mainloop"))?;
    let mut context = Context::new_with_proplist(&mainloop, "i3status-dumb", &proplist)
        .ok_or_else(|| String::from("failed to create pulse context"))?;

    context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .map_err(|error| format!("{error:?}"))?;
    wait_for_context_ready(&mut mainloop, &context)?;
    request_default_sink_volume(&context, &mut mainloop, &out)?;

    let (event_tx, event_rx) = mpsc::channel::<()>();
    context.set_subscribe_callback(Some(Box::new(move |facility, operation, _| {
        if should_refresh(facility, operation) {
            let _ = event_tx.send(());
        }
    })));

    let operation = context.subscribe(
        InterestMaskSet::SINK | InterestMaskSet::SERVER | InterestMaskSet::CARD,
        |_| {},
    );
    wait_for_operation(&mut mainloop, operation)?;

    loop {
        iterate(&mut mainloop)?;
        while event_rx.try_recv().is_ok() {
            request_default_sink_volume(&context, &mut mainloop, &out)?;
        }
    }
}

pub fn spawn(tx: watch::Sender<VolumeState>) {
    tokio::spawn(async move {
        loop {
            let join = tokio::task::spawn_blocking({
                let tx = tx.clone();
                move || run_pulse_loop(tx)
            })
            .await;

            match join {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("i3status-dumb: pulse watcher failed: {error}"),
                Err(error) => eprintln!("i3status-dumb: pulse watcher crashed: {error}"),
            }

            sleep(RETRY_DELAY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{percent_from_raw, should_refresh};
    use libpulse_binding::context::subscribe::{Facility, Operation};
    use libpulse_binding::volume::Volume;

    #[test]
    fn rounds_to_nearest_percent() {
        assert_eq!(percent_from_raw(32_768, Volume::NORMAL.0), 50);
    }

    #[test]
    fn refreshes_on_sink_changes() {
        assert!(should_refresh(
            Some(Facility::Sink),
            Some(Operation::Changed)
        ));
    }

    #[test]
    fn ignores_unrelated_events() {
        assert!(!should_refresh(
            Some(Facility::Client),
            Some(Operation::Removed)
        ));
    }
}
