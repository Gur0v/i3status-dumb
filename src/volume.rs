use std::sync::mpsc;

use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::introspect::SinkInfo;
use pulse::context::subscribe::{Facility, InterestMaskSet, Operation};
use pulse::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::proplist::Proplist;
use tokio::sync::watch;

use crate::status::VolumeState;
use crate::util::{log_error, publish_if_changed, RetryBackoff};

fn percent_from_raw(avg: u32, normal: u32) -> u16 {
    let normal = normal.max(1);
    let rounded = avg.saturating_mul(100).saturating_add(normal / 2) / normal;
    rounded.min(999) as u16
}

fn volume_from_sink(info: &SinkInfo<'_>) -> VolumeState {
    let avg = info.volume.avg().0;
    let normal = pulse::volume::Volume::NORMAL.0;
    VolumeState::new(percent_from_raw(avg, normal), info.mute)
}

fn iterate(mainloop: &mut Mainloop) -> Result<(), String> {
    match mainloop.iterate(true) {
        IterateResult::Quit(_) | IterateResult::Err(_) => {
            Err(String::from("pulse mainloop stopped"))
        }
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
        publish_if_changed(out, VolumeState::UNKNOWN);
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
        publish_if_changed(out, volume);
    } else {
        publish_if_changed(out, VolumeState::UNKNOWN);
    }

    Ok(())
}

fn should_refresh(facility: Option<Facility>, operation: Option<Operation>) -> bool {
    matches!(
        (facility, operation),
        (
            Some(Facility::Sink),
            Some(Operation::New | Operation::Changed | Operation::Removed)
        ) | (Some(Facility::Server), Some(Operation::Changed))
            | (Some(Facility::Card), Some(Operation::Changed))
    )
}

fn connect_pulse() -> Result<(Mainloop, Context), String> {
    let mut proplist = Proplist::new().ok_or_else(|| String::from("failed to create proplist"))?;
    proplist
        .set_str(
            pulse::proplist::properties::APPLICATION_NAME,
            "i3status-dumb",
        )
        .map_err(|_| String::from("failed to set pulse application name"))?;

    let mut mainloop =
        Mainloop::new().ok_or_else(|| String::from("failed to create pulse mainloop"))?;
    let mut context = Context::new_with_proplist(&mainloop, "i3status-dumb", &proplist)
        .ok_or_else(|| String::from("failed to create pulse context"))?;

    context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .map_err(|error| format!("{error:?}"))?;
    wait_for_context_ready(&mut mainloop, &context)?;

    Ok((mainloop, context))
}

fn run_pulse_once() -> Result<VolumeState, String> {
    let (mut mainloop, context) = connect_pulse()?;
    let (tx, rx) = watch::channel(VolumeState::UNKNOWN);
    request_default_sink_volume(&context, &mut mainloop, &tx)?;
    let volume = *rx.borrow();
    Ok(volume)
}

pub fn current() -> VolumeState {
    match run_pulse_once() {
        Ok(volume) => volume,
        Err(error) => {
            log_error("pulse query failed", error);
            VolumeState::UNKNOWN
        }
    }
}

fn run_pulse_loop(out: watch::Sender<VolumeState>) -> Result<(), String> {
    let (mut mainloop, mut context) = connect_pulse()?;
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
        let mut retry = RetryBackoff::new();

        loop {
            let join = tokio::task::spawn_blocking({
                let tx = tx.clone();
                move || run_pulse_loop(tx)
            })
            .await;

            match join {
                Ok(Ok(())) => retry.reset(),
                Ok(Err(error)) => {
                    publish_if_changed(&tx, VolumeState::UNKNOWN);
                    log_error("pulse watcher failed", error);
                }
                Err(error) => {
                    publish_if_changed(&tx, VolumeState::UNKNOWN);
                    log_error("pulse watcher crashed", error);
                }
            }

            retry.wait().await;
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
    fn refreshes_on_default_sink_changes() {
        assert!(should_refresh(
            Some(Facility::Server),
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

    #[test]
    fn ignores_unrelated_sink_operations() {
        assert!(!should_refresh(Some(Facility::Sink), None));
    }
}
