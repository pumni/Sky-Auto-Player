use super::super::{PlaybackClockState, QpcClock};
use super::dispatch::DispatchStep;
use super::{TrackedKeyState, WorkerConfig, WorkerRuntime, focus_gate_matches};
#[cfg(any(test, feature = "test-support"))]
use crate::engine::shared::SystemPowerState;
use crate::engine::shared::{SessionTarget, SharedProgressClock, SupervisorLeaseState};
use crate::engine::target::OwnerIdentityStatus;
use crate::engine::telemetry::{
    TRACE_KIND_DOWN, TRACE_KIND_MIXED, TRACE_KIND_UP, WorkerMetricsLocal,
};
use sky_dispatch_core::clock::PauseReason;
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::input::{ModifierKeyObservation, ModifierMask, PhysicalKeyPreflightError};
use std::sync::atomic::{AtomicBool, Ordering};

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
}

#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn invoke_final_gate_race_hook(
    hook: Option<&super::super::config::FinalGateRaceHook>,
    focus_active: &AtomicBool,
    target: &SessionTarget,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    system_power: &super::super::SystemPowerState,
) {
    if let Some(hook) = hook {
        hook(
            focus_active,
            target,
            quit_requested,
            skip_requested,
            panic_requested,
            desired_pause,
            system_power,
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

pub(crate) fn load_target_stamp(target: &SessionTarget) -> Option<TargetStamp> {
    target
        .load_stable()
        .map(|(hwnd, generation)| TargetStamp { hwnd, generation })
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
    OwnerMismatch,
    OwnerQueryUnavailable,
    OwnerIdentityAbsent,
    OwnerIdentityStale,
    OwnerProcessTerminated,
    OwnerIdentityDrift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FinalControlAdmission {
    Allowed,
    PanicRequested,
    QuitRequested,
    SkipRequested,
    PauseRequested,
    SystemSuspendRequested,
}

#[derive(Clone, Copy)]
pub(crate) struct FinalControlSignals<'a> {
    pub(crate) quit_requested: &'a AtomicBool,
    pub(crate) skip_requested: &'a AtomicBool,
    pub(crate) panic_requested: &'a AtomicBool,
    pub(crate) desired_pause: &'a AtomicBool,
    pub(crate) supervisor_expired: &'a SupervisorLeaseState,
    pub(crate) system_power: Option<&'a super::super::shared::SystemPowerState>,
}

pub(crate) struct FinalTargetSignals<'a> {
    pub(crate) expected: TargetStamp,
    pub(crate) require_focus: bool,
    pub(crate) focus_active: &'a AtomicBool,
    pub(crate) target: &'a SessionTarget,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) post_focus_race_hook: Option<&'a super::super::config::FinalGateRaceHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) post_focus_control_signals: Option<FinalControlSignals<'a>>,
}

pub(crate) fn final_control_precheck(signals: FinalControlSignals<'_>) -> FinalControlAdmission {
    if signals.supervisor_expired.is_expired() || signals.panic_requested.load(Ordering::Acquire) {
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
    if signals
        .system_power
        .is_some_and(super::super::shared::SystemPowerState::down_blocked)
    {
        return FinalControlAdmission::SystemSuspendRequested;
    }
    FinalControlAdmission::Allowed
}

/// Final target/focus gate for Down-bearing traffic at the precision boundary.
/// Control and lease decisions are intentionally kept in the shared control
/// gate so an UpOnly/release send never acquires a focus dependency.  The
/// published focus hint is a cheap early rejection; a Down that remains
/// eligible then receives one fresh foreground proof before the final atomic
/// revalidation.
pub(crate) fn final_down_target_admission(target: FinalTargetSignals<'_>) -> DownAdmission {
    if !target_stamp_still_current(target.target, target.expected) {
        return DownAdmission::TargetChanged;
    }
    if !focus_matches(target.require_focus, target.focus_active) {
        return DownAdmission::FocusLost;
    }
    if target.require_focus {
        if target.expected.hwnd == 0 {
            return DownAdmission::FocusLost;
        }
        let owner_pid = match target
            .target
            .owner_identity_status(target.expected.generation)
        {
            OwnerIdentityStatus::Bound(owner_pid) => owner_pid,
            OwnerIdentityStatus::NotRequired | OwnerIdentityStatus::Missing => {
                return DownAdmission::OwnerIdentityAbsent;
            }
            OwnerIdentityStatus::Stale => return DownAdmission::OwnerIdentityStale,
            OwnerIdentityStatus::ProcessTerminated => {
                return DownAdmission::OwnerProcessTerminated;
            }
            OwnerIdentityStatus::OwnerMismatch => return DownAdmission::OwnerMismatch,
            OwnerIdentityStatus::QueryUnavailable => {
                return DownAdmission::OwnerQueryUnavailable;
            }
            OwnerIdentityStatus::WindowUnavailable
            | OwnerIdentityStatus::ProcessIdentityMismatch => {
                return DownAdmission::OwnerIdentityDrift;
            }
        };
        match sky_dispatch_win32::focus::foreground_window_owner_matches(
            target.expected.hwnd,
            owner_pid,
        ) {
            sky_dispatch_win32::focus::ForegroundOwnerMatch::Match => {}
            sky_dispatch_win32::focus::ForegroundOwnerMatch::NotForeground => {
                return DownAdmission::FocusLost;
            }
            sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerMismatch => {
                return DownAdmission::OwnerMismatch;
            }
            sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerQueryUnavailable => {
                return DownAdmission::OwnerQueryUnavailable;
            }
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    if let (Some(hook), Some(control), Some(system_power)) = (
        target.post_focus_race_hook,
        target.post_focus_control_signals,
        target
            .post_focus_control_signals
            .and_then(|signals| signals.system_power),
    ) {
        hook(
            target.focus_active,
            target.target,
            control.quit_requested,
            control.skip_requested,
            control.panic_requested,
            control.desired_pause,
            system_power,
        );
    }
    final_down_atomic_revalidation(
        target.target,
        target.expected,
        target.require_focus,
        target.focus_active,
    )
}

/// Recheck only bounded atomic target, focus, and owner state after the
/// modifier queries. The fresh foreground/process proof remains owned by
/// `final_down_target_admission` immediately before this final suffix.
#[inline(always)]
pub(crate) fn final_down_atomic_revalidation(
    target: &SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
) -> DownAdmission {
    if !target_stamp_still_current(target, expected) {
        return DownAdmission::TargetChanged;
    }
    if !focus_matches(require_focus, focus_active) {
        return DownAdmission::FocusLost;
    }
    if require_focus {
        match target.owner_identity_status(expected.generation) {
            OwnerIdentityStatus::Bound(owner_pid) if owner_pid != 0 => {}
            OwnerIdentityStatus::NotRequired | OwnerIdentityStatus::Missing => {
                return DownAdmission::OwnerIdentityAbsent;
            }
            OwnerIdentityStatus::Stale => return DownAdmission::OwnerIdentityStale,
            OwnerIdentityStatus::ProcessTerminated => {
                return DownAdmission::OwnerProcessTerminated;
            }
            OwnerIdentityStatus::OwnerMismatch => return DownAdmission::OwnerMismatch,
            OwnerIdentityStatus::QueryUnavailable => {
                return DownAdmission::OwnerQueryUnavailable;
            }
            OwnerIdentityStatus::WindowUnavailable
            | OwnerIdentityStatus::ProcessIdentityMismatch => {
                return DownAdmission::OwnerIdentityDrift;
            }
            OwnerIdentityStatus::Bound(_) => return DownAdmission::OwnerIdentityAbsent,
        }
    }
    DownAdmission::Allowed
}

#[allow(clippy::too_many_arguments)]
pub(super) fn modifier_guard_and_final_revalidation(
    has_down: bool,
    backend: &TrackedKeyState,
    preflight_target: Option<TargetStamp>,
    config: &WorkerConfig,
    focus_active: &AtomicBool,
    target: &SessionTarget,
    control_signals: FinalControlSignals<'_>,
    #[cfg(any(test, feature = "test-support"))] quit_requested: &AtomicBool,
    #[cfg(any(test, feature = "test-support"))] skip_requested: &AtomicBool,
    #[cfg(any(test, feature = "test-support"))] panic_requested: &AtomicBool,
    #[cfg(any(test, feature = "test-support"))] desired_pause: &AtomicBool,
    #[cfg(any(test, feature = "test-support"))] system_power: &SystemPowerState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    progress_clock: &SharedProgressClock,
) -> Result<Option<super::dispatch::AdmissionOutcome>, DispatchStep> {
    if !has_down {
        return Ok(None);
    }
    match backend.observe_modifiers_before_down() {
        ModifierKeyObservation::NoHeldObserved => {}
        ModifierKeyObservation::HeldObserved(mask) => {
            return Err(modifier_guard_rejection_step(mask));
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    invoke_final_gate_race_hook(
        runtime.final_gate_post_modifier_race_hook.as_ref(),
        focus_active,
        target,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        system_power,
    );
    let Some(expected) = preflight_target else {
        return Err(DispatchStep::TerminateStatic(
            "modifier guard reached without frozen target proof",
        ));
    };
    match final_down_atomic_revalidation(target, expected, config.focus.require_focus, focus_active)
    {
        DownAdmission::Allowed => {}
        DownAdmission::TargetChanged => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            record_final_gate_rejection(local_metrics, FinalGateRejection::Target);
            return Ok(Some(super::dispatch::AdmissionOutcome::TargetChanged));
        }
        DownAdmission::FocusLost => {
            record_final_gate_rejection(local_metrics, FinalGateRejection::Focus);
            handle_final_focus_loss(qpc_clock, clock_state, runtime, progress_clock)?;
            return Ok(Some(super::dispatch::AdmissionOutcome::FocusLost));
        }
        DownAdmission::OwnerMismatch
        | DownAdmission::OwnerQueryUnavailable
        | DownAdmission::OwnerIdentityAbsent
        | DownAdmission::OwnerIdentityStale
        | DownAdmission::OwnerProcessTerminated
        | DownAdmission::OwnerIdentityDrift => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            return Err(DispatchStep::TerminateStatic(
                "authored_down_owner_changed_after_modifier_guard",
            ));
        }
    }
    if !matches!(
        final_control_precheck(control_signals),
        FinalControlAdmission::Allowed
    ) {
        runtime.verified_target = None;
        record_final_gate_rejection(local_metrics, FinalGateRejection::Control);
        return Ok(Some(super::dispatch::AdmissionOutcome::ControlRejected));
    }
    Ok(None)
}

pub(crate) fn modifier_guard_rejection_step(mask: ModifierMask) -> DispatchStep {
    DispatchStep::Terminate(format!("physical_modifier_held:{:02X}", mask.bits()))
}

pub(crate) fn enter_focus_pause(
    clock_state: &mut PlaybackClockState,
    runtime: &mut super::WorkerRuntime,
    focus_ticks: QpcTicks,
    progress_clock: &SharedProgressClock,
) -> Result<bool, String> {
    runtime.invalidate_down_authorization();
    runtime.verified_target = None;
    runtime.focus_restore_started_ticks = None;
    if clock_state.has_pause_reason(PauseReason::Focus) {
        return Ok(false);
    }
    clock_state
        .enter_pause(PauseReason::Focus, focus_ticks)
        .map_err(|error| format!("playback clock failure: {error}"))?;
    progress_clock.publish(clock_state);
    Ok(true)
}

pub(crate) fn handle_final_focus_loss(
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    runtime: &mut super::WorkerRuntime,
    progress_clock: &SharedProgressClock,
) -> Result<(), DispatchStep> {
    if !runtime.musical_physical_commit_started {
        return Err(DispatchStep::TerminateStatic("focus_lost_during_preroll"));
    }
    let focus_ticks = qpc_clock
        .now()
        .map_err(|error| DispatchStep::Terminate(format!("QPC failure: {error:?}")))?;
    enter_focus_pause(clock_state, runtime, focus_ticks, progress_clock)
        .map(|_| ())
        .map_err(DispatchStep::Terminate)?;
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

pub(crate) fn target_stamp_still_current(target: &SessionTarget, expected: TargetStamp) -> bool {
    target.is_current(expected.hwnd, expected.generation)
}
