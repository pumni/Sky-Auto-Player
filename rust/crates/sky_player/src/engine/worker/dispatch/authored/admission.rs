use super::super::super::{
    FinalControlSignals, FinalDownVerdict, FinalTargetSignals, PlaybackClockState, QpcClock,
    TargetStamp, WorkerRuntime, apply_final_down_rejection,
};
use super::super::{DispatchStep, DownBoundaryAdmission};
use super::AdmissionOutcome;
use crate::engine::shared::{SessionTarget, SharedProgressClock};
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_win32::clock::QpcTicks;
use std::sync::atomic::AtomicBool;

pub(super) fn build_final_target_signals<'a>(
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &'a AtomicBool,
    target: &'a SessionTarget,
    #[cfg(any(test, feature = "test-support"))] post_focus_race_hook: Option<
        &'a super::super::super::config::FinalGateRaceHook,
    >,
    #[cfg(any(test, feature = "test-support"))] post_focus_control_signals: FinalControlSignals<'a>,
) -> FinalTargetSignals<'a> {
    FinalTargetSignals {
        expected,
        require_focus,
        focus_active,
        target,
        #[cfg(any(test, feature = "test-support"))]
        post_focus_race_hook,
        #[cfg(any(test, feature = "test-support"))]
        post_focus_control_signals: Some(post_focus_control_signals),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reject_authored_target_if_needed(
    verdict: FinalDownVerdict,
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    progress_clock: &SharedProgressClock,
) -> Result<Option<AdmissionOutcome>, DispatchStep> {
    if verdict == FinalDownVerdict::Allowed {
        return Ok(None);
    }
    apply_final_down_rejection(verdict, runtime, local_metrics);
    super::authored_target_rejection_outcome(
        verdict,
        qpc_clock,
        clock_state,
        runtime,
        progress_clock,
    )
    .map(Some)
}

pub(super) fn final_policy_boundary_qpc(
    qpc_clock: QpcClock,
    physical_target_qpc: QpcTicks,
    #[cfg(any(test, feature = "test-support"))] test_direct_boundary: bool,
) -> Result<QpcTicks, DispatchStep> {
    #[cfg(any(test, feature = "test-support"))]
    if test_direct_boundary {
        return Ok(physical_target_qpc);
    }
    qpc_clock.now().map_err(|error| {
        DispatchStep::Terminate(format!("QPC final policy boundary failure: {error:?}"))
    })
}

pub(super) fn resolve_target_crossing(
    down_admission: DownBoundaryAdmission,
    boundary_crossing_qpc: Option<QpcTicks>,
    physical_target_qpc: QpcTicks,
    #[cfg(any(test, feature = "test-support"))] test_direct_boundary: bool,
) -> Result<Option<Option<QpcTicks>>, DispatchStep> {
    match super::resolve_target_crossing_qpc(
        down_admission,
        boundary_crossing_qpc,
        physical_target_qpc,
        #[cfg(any(test, feature = "test-support"))]
        test_direct_boundary,
    ) {
        Ok(value) => Ok(Some(value)),
        Err(DispatchStep::Continue) => Ok(None),
        Err(step) => Err(step),
    }
}
