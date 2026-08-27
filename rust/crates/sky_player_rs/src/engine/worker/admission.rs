use super::super::{PlaybackClockState, QpcClock, RuntimeDispatchCoordinator};
use super::dispatch::DispatchStep;
use super::{TrackedKeyState, focus_gate_matches};
use crate::engine::shared::SharedProgressClock;
use crate::engine::telemetry::{
    TRACE_KIND_DOWN, TRACE_KIND_MIXED, TRACE_KIND_UP, WorkerMetricsLocal,
};
use sky_dispatch_core::clock::PauseReason;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcError, QpcTicks};
use sky_dispatch_win32::input::PhysicalKeyPreflightError;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TargetStamp {
    pub(crate) hwnd: isize,
    pub(crate) generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FinalGateRejection {
    Control,
    Target,
    Focus,
    Lease,
}

#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn invoke_final_gate_race_hook(
    hook: Option<&super::super::config::FinalGateRaceHook>,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
) {
    if let Some(hook) = hook {
        hook(
            focus_active,
            target_hwnd,
            target_generation,
            quit_requested,
            skip_requested,
            panic_requested,
            desired_pause,
        );
    }
}

pub(crate) fn record_final_gate_rejection(
    local_metrics: &mut WorkerMetricsLocal,
    reason: FinalGateRejection,
) {
    match reason {
        FinalGateRejection::Control => {
            local_metrics.final_gate_control_rejections = local_metrics
                .final_gate_control_rejections
                .saturating_add(1)
        }
        FinalGateRejection::Target => {
            local_metrics.final_gate_target_changes =
                local_metrics.final_gate_target_changes.saturating_add(1)
        }
        FinalGateRejection::Focus => {
            local_metrics.final_gate_focus_losses =
                local_metrics.final_gate_focus_losses.saturating_add(1)
        }
        FinalGateRejection::Lease => {
            local_metrics.final_gate_lease_expirations =
                local_metrics.final_gate_lease_expirations.saturating_add(1)
        }
    }
}

pub(crate) fn trace_kind_for_packet_kind(
    packet_kind: sky_dispatch_core::model::PhysicalPacketKind,
) -> u8 {
    match packet_kind {
        sky_dispatch_core::model::PhysicalPacketKind::UpOnly => TRACE_KIND_UP,
        sky_dispatch_core::model::PhysicalPacketKind::DownOnly => TRACE_KIND_DOWN,
        sky_dispatch_core::model::PhysicalPacketKind::Mixed => TRACE_KIND_MIXED,
    }
}

pub(crate) fn load_target_stamp(
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
) -> TargetStamp {
    TargetStamp {
        hwnd: target_hwnd.load(Ordering::Acquire),
        generation: target_generation.load(Ordering::Acquire),
    }
}

pub(crate) fn focus_matches_hwnd(
    require_focus: bool,
    focus_active: &AtomicBool,
    expected_hwnd: isize,
) -> bool {
    if !require_focus {
        return true;
    }
    let validated_focus_active = focus_active.load(Ordering::Acquire);
    let foreground_matches =
        expected_hwnd == 0 || sky_dispatch_win32::focus::foreground_window_matches(expected_hwnd);
    focus_gate_matches(
        require_focus,
        validated_focus_active,
        expected_hwnd,
        foreground_matches,
    )
}

pub(crate) fn focus_matches(require_focus: bool, focus_active: &AtomicBool) -> bool {
    !require_focus || focus_active.load(Ordering::Acquire)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DownAdmission {
    Allowed,
    TargetChanged,
    FocusLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FinalControlAdmission {
    Allowed,
    PanicRequested,
    QuitRequested,
    SkipRequested,
    PauseRequested,
    LeaseExpired,
}

#[derive(Clone, Copy)]
pub(crate) struct FinalControlSignals<'a> {
    pub(crate) quit_requested: &'a AtomicBool,
    pub(crate) skip_requested: &'a AtomicBool,
    pub(crate) panic_requested: &'a AtomicBool,
    pub(crate) desired_pause: &'a AtomicBool,
    pub(crate) supervisor_heartbeat_ticks: &'a AtomicU64,
}

pub(crate) struct FinalTargetSignals<'a> {
    pub(crate) expected: TargetStamp,
    pub(crate) require_focus: bool,
    pub(crate) focus_active: &'a AtomicBool,
    pub(crate) target_hwnd: &'a AtomicIsize,
    pub(crate) target_generation: &'a AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) post_focus_race_hook: Option<&'a super::super::config::FinalGateRaceHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) post_focus_control_signals: Option<FinalControlSignals<'a>>,
}

/// Classify lease state from the one authoritative start sample.
fn classify_final_control(
    now_qpc: QpcTicks,
    lease_timeout_ticks: DurationTicks,
    signals: FinalControlSignals<'_>,
) -> Result<FinalControlAdmission, QpcError> {
    if super::supervisor_lease_expired(
        now_qpc,
        lease_timeout_ticks,
        signals.supervisor_heartbeat_ticks,
    )? {
        return Ok(FinalControlAdmission::LeaseExpired);
    }
    Ok(FinalControlAdmission::Allowed)
}

pub(crate) fn final_control_precheck(signals: FinalControlSignals<'_>) -> FinalControlAdmission {
    if signals.panic_requested.load(Ordering::Acquire) {
        return FinalControlAdmission::PanicRequested;
    }
    if signals.quit_requested.load(Ordering::Acquire) {
        return FinalControlAdmission::QuitRequested;
    }
    if signals.skip_requested.load(Ordering::Acquire) {
        return FinalControlAdmission::SkipRequested;
    }
    if signals.desired_pause.load(Ordering::Acquire) {
        return FinalControlAdmission::PauseRequested;
    }
    FinalControlAdmission::Allowed
}

pub(crate) fn final_control_admission_at(
    final_policy_qpc: QpcTicks,
    lease_timeout_ticks: DurationTicks,
    signals: FinalControlSignals<'_>,
) -> Result<FinalControlAdmission, QpcError> {
    classify_final_control(final_policy_qpc, lease_timeout_ticks, signals)
}

/// Compatibility wrapper for test seams and non-physical callers. Production
/// dispatch uses `final_control_precheck` followed by one caller-owned QPC
/// sample and `final_control_admission_at`.
#[cfg(test)]
pub(crate) fn final_control_admission_with_lease(
    qpc_clock: QpcClock,
    lease_timeout_ticks: DurationTicks,
    signals: FinalControlSignals<'_>,
) -> Result<(FinalControlAdmission, Option<QpcTicks>), QpcError> {
    let precheck = final_control_precheck(FinalControlSignals {
        quit_requested: signals.quit_requested,
        skip_requested: signals.skip_requested,
        panic_requested: signals.panic_requested,
        desired_pause: signals.desired_pause,
        supervisor_heartbeat_ticks: signals.supervisor_heartbeat_ticks,
    });
    if !matches!(precheck, FinalControlAdmission::Allowed) {
        return Ok((precheck, None));
    }
    let now_qpc = qpc_clock.now()?;
    let admission = final_control_admission_at(now_qpc, lease_timeout_ticks, signals)?;
    Ok((admission, Some(now_qpc)))
}

/// Authoritative target/focus gate for Down-bearing traffic.  Control and
/// lease decisions are intentionally kept in the shared control gate so an
/// UpOnly/release send never acquires a focus dependency.
pub(crate) fn final_down_target_admission(target: FinalTargetSignals<'_>) -> DownAdmission {
    if !target_stamp_still_current(
        target.target_hwnd,
        target.target_generation,
        target.expected,
    ) {
        return DownAdmission::TargetChanged;
    }
    if !focus_matches_hwnd(
        target.require_focus,
        target.focus_active,
        target.expected.hwnd,
    ) {
        return DownAdmission::FocusLost;
    }
    #[cfg(any(test, feature = "test-support"))]
    if let (Some(hook), Some(control)) = (
        target.post_focus_race_hook,
        target.post_focus_control_signals,
    ) {
        hook(
            target.focus_active,
            target.target_hwnd,
            target.target_generation,
            control.quit_requested,
            control.skip_requested,
            control.panic_requested,
            control.desired_pause,
        );
    }
    if !target_stamp_still_current(
        target.target_hwnd,
        target.target_generation,
        target.expected,
    ) {
        return DownAdmission::TargetChanged;
    }
    if !focus_matches(target.require_focus, target.focus_active) {
        return DownAdmission::FocusLost;
    }
    DownAdmission::Allowed
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_final_focus_loss(
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    runtime: &mut super::WorkerRuntime,
    target_hwnd: &AtomicIsize,
    progress_clock: &SharedProgressClock,
) -> Result<(), DispatchStep> {
    runtime.verified_target = None;
    if !runtime.musical_physical_commit_started {
        return Err(DispatchStep::TerminateStatic("focus_lost_during_preroll"));
    }
    let focus_ticks = qpc_clock
        .now()
        .map_err(|error| DispatchStep::Terminate(format!("QPC failure: {error:?}")))?;
    super::suspend_live_input(backend, coordinator, target_hwnd.load(Ordering::Acquire))
        .map_err(|error| DispatchStep::Terminate(format!("focus suspension failed: {error}")))?;
    clock_state
        .enter_pause(PauseReason::Focus, focus_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "playback clock failure after final focus check: {error}"
            ))
        })?;
    progress_clock.publish(clock_state);
    runtime.focus_restore_started_ticks = None;
    Ok(())
}

pub(crate) fn ensure_preflight_for_target(
    backend: &TrackedKeyState,
    current: TargetStamp,
    verified_target: &mut Option<TargetStamp>,
) -> Result<(), PhysicalKeyPreflightError> {
    if *verified_target == Some(current) {
        return Ok(());
    }
    *verified_target = None;
    backend.ensure_instrument_keys_physically_up(current.hwnd)?;
    *verified_target = Some(current);
    Ok(())
}

pub(crate) fn target_stamp_still_current(
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    expected: TargetStamp,
) -> bool {
    target_generation.load(Ordering::Acquire) == expected.generation
        && target_hwnd.load(Ordering::Acquire) == expected.hwnd
}
