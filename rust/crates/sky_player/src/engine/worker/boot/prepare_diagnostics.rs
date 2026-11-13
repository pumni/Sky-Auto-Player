use sky_dispatch_core::model::{ActionKind, NO_GENERATION_ID, RuntimeSchedule};

pub(super) fn record_unmatched_up_prepare_diagnostics(
    schedule: &RuntimeSchedule,
    qpc_clock: super::super::super::QpcClock,
    telemetry_output: &std::sync::Arc<
        parking_lot::Mutex<crate::engine::telemetry::TelemetryCollector>,
    >,
    mut checkpoint: impl FnMut() -> Result<(), String>,
    #[cfg(any(test, feature = "test-support"))] startup_hook: Option<
        &super::super::super::config::StartupOrderingHook,
    >,
) -> Result<(), String> {
    const CHECKPOINT_INTERVAL: usize = 32;
    let mut telemetry = telemetry_output.lock();
    let mut diagnostics_since_checkpoint = 0usize;
    for batch in schedule
        .batches
        .iter()
        .filter(|batch| batch.kind == ActionKind::Up)
    {
        let unmatched_count = schedule
            .intent_slice(batch)
            .iter()
            .filter(|intent| intent.generation_id() == NO_GENERATION_ID)
            .count();
        if unmatched_count == 0 {
            continue;
        }
        let effective_scheduled_ticks = qpc_clock
            .timeline_from_us(batch.scheduled_us)
            .map_err(|error| format!("timestamp conversion failed: {error:?}"))?;
        super::super::dispatch::observer_trace::drain_stale_metadata_observation(
            &super::super::dispatch::observation::StaleMetadataObservation {
                source_action_index: batch.source_action_index,
                effective_scheduled_ticks,
                suppressed_intent_count: unmatched_count,
            },
            &mut telemetry,
        )
        .map_err(|step| format!("trace admission failed: {step:?}"))?;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = startup_hook {
            hook.record_unmatched_up_prepare(unmatched_count);
        }
        diagnostics_since_checkpoint += unmatched_count;
        if diagnostics_since_checkpoint >= CHECKPOINT_INTERVAL {
            drop(telemetry);
            checkpoint()?;
            telemetry = telemetry_output.lock();
            diagnostics_since_checkpoint = 0;
        }
    }
    drop(telemetry);
    if diagnostics_since_checkpoint != 0 {
        checkpoint()?;
    }
    Ok(())
}
