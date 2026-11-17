use sky_dispatch_core::executor::PreparedDispatchSchedule;

pub(super) fn record_unmatched_up_prepare_diagnostics(
    schedule: &PreparedDispatchSchedule,
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
    for metadata in schedule.unmatched_up_metadata() {
        super::super::dispatch::observer_trace::drain_stale_metadata_observation(
            &super::super::dispatch::observation::StaleMetadataObservation {
                source_action_index: metadata.source_action_index,
                effective_scheduled_ticks: metadata.scheduled_ticks,
                suppressed_intent_count: metadata.suppressed_intent_count,
            },
            &mut telemetry,
        )
        .map_err(|step| format!("trace admission failed: {step:?}"))?;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = startup_hook {
            hook.record_unmatched_up_prepare(metadata.suppressed_intent_count);
        }
        diagnostics_since_checkpoint += metadata.suppressed_intent_count;
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
