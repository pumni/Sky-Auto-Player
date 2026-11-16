use super::super::{PlaybackClockState, QpcClock};
use super::dispatch::DispatchStep;
use super::{TrackedKeyState, WorkerRuntime, focus_gate_matches};
use crate::engine::shared::{
    SessionTarget, SharedProgressClock, SupervisorLeaseAdmission, SupervisorLeaseAgeProof,
    SupervisorLeaseState,
};
use crate::engine::target::{
    OwnerAuthorityReadError, OwnerIdentityStatus, ReleaseOwnerPermitError,
};
use crate::engine::telemetry::WorkerMetricsLocal;
use parking_lot::MutexGuard;
use sky_dispatch_core::clock::PauseReason;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcError, QpcTicks};
use sky_dispatch_win32::input::{
    ModifierKeyObservation, ModifierMask, PhysicalKeyPreflightError, ReleaseOwnerProof,
};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TargetStamp {
    pub(crate) hwnd: isize,
    pub(crate) generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FinalGateRejection {
    Control,
    Modifier,
    Target,
    Owner,
    Focus,
    LeaseExpired,
    LeaseUnproven,
}

/// Take the single enabled-lease age sample for eligible Down traffic and
/// apply the bounded supervisor proof. A disabled timeout preserves the
/// existing diagnostic-only behavior without reading QPC or lease state.
#[inline(always)]
pub(crate) fn final_down_lease_age_proof(
    lease: &SupervisorLeaseState,
    timeout_ticks: DurationTicks,
    qpc_clock: QpcClock,
    #[cfg(test)] qpc_sample_count: &mut u64,
    #[cfg(test)] qpc_sample_override: &mut Option<Result<QpcTicks, QpcError>>,
) -> Result<SupervisorLeaseAgeProof, QpcError> {
    if timeout_ticks == DurationTicks::ZERO {
        return Ok(lease.prove_down_age(QpcTicks::from_raw(0), timeout_ticks));
    }

    #[cfg(test)]
    let now = {
        *qpc_sample_count = qpc_sample_count.saturating_add(1);
        qpc_sample_override
            .take()
            .unwrap_or_else(|| qpc_clock.now())?
    };
    #[cfg(not(test))]
    let now = qpc_clock.now()?;

    Ok(lease.prove_down_age(now, timeout_ticks))
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
        FinalGateRejection::Modifier => {
            local_metrics.final_gate_control_rejections = local_metrics
                .final_gate_control_rejections
                .saturating_add(1)
        }
        FinalGateRejection::Target => {
            local_metrics.final_gate_target_changes =
                local_metrics.final_gate_target_changes.saturating_add(1)
        }
        FinalGateRejection::Owner => {
            local_metrics.final_gate_target_changes =
                local_metrics.final_gate_target_changes.saturating_add(1)
        }
        FinalGateRejection::Focus => {
            local_metrics.final_gate_focus_losses =
                local_metrics.final_gate_focus_losses.saturating_add(1)
        }
        FinalGateRejection::LeaseExpired => {
            local_metrics.final_gate_lease_expirations =
                local_metrics.final_gate_lease_expirations.saturating_add(1)
        }
        FinalGateRejection::LeaseUnproven => {
            local_metrics.final_gate_lease_expirations =
                local_metrics.final_gate_lease_expirations.saturating_add(1)
        }
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
pub(crate) enum OwnerAdmissionFailure {
    Mismatch,
    QueryUnavailable,
    ProofBusy,
    IdentityAbsent,
    IdentityStale,
    ProcessTerminated,
    IdentityDrift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FinalDownVerdict {
    Allowed,
    ControlRejected,
    ModifierHeld(ModifierMask),
    TargetChanged,
    FocusLost,
    OwnerRejected(OwnerAdmissionFailure),
    LeaseExpired,
    LeaseUnproven,
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

#[derive(Clone, Copy)]
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
    if signals
        .system_power
        .is_some_and(super::super::shared::SystemPowerState::down_blocked)
    {
        return FinalControlAdmission::SystemSuspendRequested;
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

#[inline(always)]
pub(crate) fn final_down_control_verdict(signals: FinalControlSignals<'_>) -> FinalDownVerdict {
    match final_control_precheck(signals) {
        FinalControlAdmission::Allowed => FinalDownVerdict::Allowed,
        FinalControlAdmission::PanicRequested
        | FinalControlAdmission::QuitRequested
        | FinalControlAdmission::SkipRequested
        | FinalControlAdmission::PauseRequested
        | FinalControlAdmission::SystemSuspendRequested => FinalDownVerdict::ControlRejected,
    }
}

/// Cheap target and published-focus hints used before modifier observation.
#[inline(always)]
pub(crate) fn final_down_target_hint(target: &FinalTargetSignals<'_>) -> FinalDownVerdict {
    if !target_stamp_still_current(target.target, target.expected) {
        return FinalDownVerdict::TargetChanged;
    }
    if !focus_matches(target.require_focus, target.focus_active) {
        return FinalDownVerdict::FocusLost;
    }
    FinalDownVerdict::Allowed
}

/// Fresh foreground/process proof and coherent target-owner revalidation for
/// Down-bearing traffic. Call after modifier observation, immediately before
/// the shared lease and final-control suffix.
pub(crate) fn final_down_fresh_target_admission(
    target: FinalTargetSignals<'_>,
) -> FinalDownVerdict {
    let owner_pid = if target.require_focus {
        if target.expected.hwnd == 0 {
            return FinalDownVerdict::FocusLost;
        }
        match target
            .target
            .owner_identity_status(target.expected.generation)
        {
            OwnerIdentityStatus::Bound(owner_pid) if owner_pid != 0 => owner_pid,
            OwnerIdentityStatus::NotRequired | OwnerIdentityStatus::Missing => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityAbsent);
            }
            OwnerIdentityStatus::Stale => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityStale);
            }
            OwnerIdentityStatus::ProcessTerminated => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::ProcessTerminated);
            }
            OwnerIdentityStatus::OwnerMismatch => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::Mismatch);
            }
            OwnerIdentityStatus::QueryUnavailable => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::QueryUnavailable);
            }
            OwnerIdentityStatus::WindowUnavailable
            | OwnerIdentityStatus::ProcessIdentityMismatch => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityDrift);
            }
            OwnerIdentityStatus::Bound(_) => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityAbsent);
            }
        }
    } else {
        0
    };
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
        target.require_focus.then_some(owner_pid),
    )
}

/// Recheck only bounded atomic target, focus, and owner state after the
/// modifier queries. The fresh foreground/process proof remains owned by
/// `verify_final_down_admission` immediately before this final suffix.
#[inline(always)]
pub(crate) fn final_down_atomic_revalidation(
    target: &SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
    expected_owner_pid: Option<u32>,
) -> FinalDownVerdict {
    if !target_stamp_still_current(target, expected) {
        return FinalDownVerdict::TargetChanged;
    }
    if !focus_matches(require_focus, focus_active) {
        return FinalDownVerdict::FocusLost;
    }
    if require_focus {
        match target.owner_identity_status(expected.generation) {
            OwnerIdentityStatus::Bound(owner_pid) if owner_pid != 0 => {
                if expected_owner_pid.is_some_and(|expected| expected != owner_pid) {
                    return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::Mismatch);
                }
            }
            OwnerIdentityStatus::NotRequired | OwnerIdentityStatus::Missing => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityAbsent);
            }
            OwnerIdentityStatus::Stale => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityStale);
            }
            OwnerIdentityStatus::ProcessTerminated => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::ProcessTerminated);
            }
            OwnerIdentityStatus::OwnerMismatch => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::Mismatch);
            }
            OwnerIdentityStatus::QueryUnavailable => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::QueryUnavailable);
            }
            OwnerIdentityStatus::WindowUnavailable
            | OwnerIdentityStatus::ProcessIdentityMismatch => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityDrift);
            }
            OwnerIdentityStatus::Bound(_) => {
                return FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityAbsent);
            }
        }
    }
    FinalDownVerdict::Allowed
}

/// The shared post-modifier authority gate. Both dispatch paths enter here
/// after the same fresh foreground proof and modifier observation. This keeps
/// target revalidation, the single lease-age sample, and the final control
/// check in one auditable order without adding a sample or changing sender
/// ownership.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(crate) fn final_down_post_modifier_verdict(
    target: &SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
    control_signals: FinalControlSignals<'_>,
    lease_timeout_ticks: DurationTicks,
    qpc_clock: QpcClock,
    #[cfg(any(test, feature = "test-support"))] post_lease_race_hook: Option<
        &super::super::config::FinalGateRaceHook,
    >,
    #[cfg(test)] qpc_sample_count: &mut u64,
    #[cfg(test)] qpc_sample_override: &mut Option<Result<QpcTicks, QpcError>>,
) -> Result<FinalDownVerdict, QpcError> {
    let authority =
        final_down_atomic_revalidation(target, expected, require_focus, focus_active, None);
    if authority != FinalDownVerdict::Allowed {
        return Ok(authority);
    }

    let lease_proof = final_down_lease_age_proof(
        control_signals.supervisor_expired,
        lease_timeout_ticks,
        qpc_clock,
        #[cfg(test)]
        qpc_sample_count,
        #[cfg(test)]
        qpc_sample_override,
    )?;
    match lease_proof.admission {
        SupervisorLeaseAdmission::Fresh => {}
        SupervisorLeaseAdmission::Expired => return Ok(FinalDownVerdict::LeaseExpired),
        SupervisorLeaseAdmission::Unproven => return Ok(FinalDownVerdict::LeaseUnproven),
    }

    #[cfg(any(test, feature = "test-support"))]
    if let (Some(hook), Some(system_power)) = (post_lease_race_hook, control_signals.system_power) {
        hook(
            focus_active,
            target,
            control_signals.quit_requested,
            control_signals.skip_requested,
            control_signals.panic_requested,
            control_signals.desired_pause,
            system_power,
        );
    }

    Ok(final_down_control_verdict(control_signals))
}

#[inline(always)]
pub(crate) fn final_down_modifier_verdict(backend: &TrackedKeyState) -> FinalDownVerdict {
    match backend.observe_modifiers_before_down() {
        ModifierKeyObservation::NoHeldObserved => FinalDownVerdict::Allowed,
        ModifierKeyObservation::HeldObserved(mask) => FinalDownVerdict::ModifierHeld(mask),
    }
}

/// Apply the common metric bucket and target-authority invalidation for a
/// rejected final Down. Focus loss is completed by each caller's existing
/// focus-pause transition, while modifiers leave target authorization intact.
#[inline(always)]
pub(crate) fn apply_final_down_rejection(
    verdict: FinalDownVerdict,
    target: &SessionTarget,
    expected: TargetStamp,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
) {
    let rejection = match verdict {
        FinalDownVerdict::Allowed => return,
        FinalDownVerdict::ControlRejected => FinalGateRejection::Control,
        FinalDownVerdict::ModifierHeld(_) => FinalGateRejection::Modifier,
        FinalDownVerdict::TargetChanged => FinalGateRejection::Target,
        FinalDownVerdict::FocusLost => FinalGateRejection::Focus,
        FinalDownVerdict::OwnerRejected(_) => FinalGateRejection::Owner,
        FinalDownVerdict::LeaseExpired => FinalGateRejection::LeaseExpired,
        FinalDownVerdict::LeaseUnproven => FinalGateRejection::LeaseUnproven,
    };
    record_final_gate_rejection(local_metrics, rejection);

    match verdict {
        FinalDownVerdict::ControlRejected => runtime.verified_target = None,
        FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::ProofBusy) => {
            retain_current_preflight_for_retry(target, expected, runtime);
        }
        FinalDownVerdict::TargetChanged
        | FinalDownVerdict::OwnerRejected(_)
        | FinalDownVerdict::LeaseExpired
        | FinalDownVerdict::LeaseUnproven => runtime.verified_target = None,
        FinalDownVerdict::Allowed
        | FinalDownVerdict::ModifierHeld(_)
        | FinalDownVerdict::FocusLost => {}
    }
}

/// A transient owner-lock miss says nothing about the startup physical-key
/// scan. Retain only the exact, already validated target stamp when that same
/// HWND/generation is still published; never create or transfer a stamp.
#[inline(always)]
fn retain_current_preflight_for_retry(
    target: &SessionTarget,
    expected: TargetStamp,
    runtime: &mut WorkerRuntime,
) {
    if runtime.verified_target != Some(expected) || !target_stamp_still_current(target, expected) {
        runtime.verified_target = None;
    }
}

pub(crate) fn owner_rejection_reason(failure: OwnerAdmissionFailure) -> &'static str {
    match failure {
        OwnerAdmissionFailure::Mismatch => "prepared_down_owner_mismatch",
        OwnerAdmissionFailure::QueryUnavailable => "prepared_down_owner_query_unavailable",
        OwnerAdmissionFailure::ProofBusy => "prepared_down_owner_proof_busy",
        OwnerAdmissionFailure::IdentityAbsent => "prepared_down_owner_identity_absent",
        OwnerAdmissionFailure::IdentityStale => "prepared_down_owner_identity_stale",
        OwnerAdmissionFailure::ProcessTerminated => "prepared_down_owner_process_terminated",
        OwnerAdmissionFailure::IdentityDrift => "prepared_down_owner_identity_drift",
    }
}

fn fresh_final_release_owner_proof(
    target: &SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
) -> Result<ReleaseOwnerProof, FinalDownVerdict> {
    let release_owner = target
        .try_release_owner_proof_for_generation(expected.hwnd, expected.generation)
        .map_err(|error| match error {
            OwnerAuthorityReadError::Busy => {
                FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::ProofBusy)
            }
            OwnerAuthorityReadError::Status(OwnerIdentityStatus::Stale)
                if !target.is_current(expected.hwnd, expected.generation) =>
            {
                FinalDownVerdict::TargetChanged
            }
            OwnerAuthorityReadError::Status(status) => {
                FinalDownVerdict::OwnerRejected(owner_status_failure(status))
            }
        })?;

    if !require_focus {
        return Ok(release_owner);
    }
    if expected.hwnd == 0 {
        return Err(FinalDownVerdict::FocusLost);
    }
    let Some(owner_pid) = release_owner.owner_pid().filter(|pid| *pid != 0) else {
        return Err(FinalDownVerdict::OwnerRejected(
            OwnerAdmissionFailure::IdentityAbsent,
        ));
    };
    match sky_dispatch_win32::focus::foreground_window_owner_matches(expected.hwnd, owner_pid) {
        sky_dispatch_win32::focus::ForegroundOwnerMatch::Match => {}
        sky_dispatch_win32::focus::ForegroundOwnerMatch::NotForeground => {
            return Err(FinalDownVerdict::FocusLost);
        }
        sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerMismatch => {
            return Err(FinalDownVerdict::OwnerRejected(
                OwnerAdmissionFailure::Mismatch,
            ));
        }
        sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerQueryUnavailable => {
            return Err(FinalDownVerdict::OwnerRejected(
                OwnerAdmissionFailure::QueryUnavailable,
            ));
        }
    }
    if target.owner_process_identity_required() {
        let Some(authority) = release_owner.authority() else {
            return Err(FinalDownVerdict::OwnerRejected(
                OwnerAdmissionFailure::IdentityAbsent,
            ));
        };
        if authority.identity().hwnd != expected.hwnd || authority.identity().owner_pid != owner_pid
        {
            return Err(FinalDownVerdict::OwnerRejected(
                OwnerAdmissionFailure::Mismatch,
            ));
        }
        match authority.verify_final_process_identity(expected.hwnd, owner_pid) {
            sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous => {}
            sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated => {
                return Err(FinalDownVerdict::OwnerRejected(
                    OwnerAdmissionFailure::ProcessTerminated,
                ));
            }
            sky_dispatch_win32::focus::WindowIdentityContinuity::OwnerMismatch => {
                return Err(FinalDownVerdict::OwnerRejected(
                    OwnerAdmissionFailure::Mismatch,
                ));
            }
            sky_dispatch_win32::focus::WindowIdentityContinuity::WindowUnavailable
            | sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessIdentityMismatch => {
                return Err(FinalDownVerdict::OwnerRejected(
                    OwnerAdmissionFailure::IdentityDrift,
                ));
            }
            sky_dispatch_win32::focus::WindowIdentityContinuity::QueryUnavailable => {
                return Err(FinalDownVerdict::OwnerRejected(
                    OwnerAdmissionFailure::QueryUnavailable,
                ));
            }
        }
    }
    Ok(release_owner)
}

fn owner_status_failure(status: OwnerIdentityStatus) -> OwnerAdmissionFailure {
    match status {
        OwnerIdentityStatus::NotRequired
        | OwnerIdentityStatus::Missing
        | OwnerIdentityStatus::Bound(_) => OwnerAdmissionFailure::IdentityAbsent,
        OwnerIdentityStatus::Stale => OwnerAdmissionFailure::IdentityStale,
        OwnerIdentityStatus::ProcessTerminated => OwnerAdmissionFailure::ProcessTerminated,
        OwnerIdentityStatus::OwnerMismatch => OwnerAdmissionFailure::Mismatch,
        OwnerIdentityStatus::WindowUnavailable | OwnerIdentityStatus::ProcessIdentityMismatch => {
            OwnerAdmissionFailure::IdentityDrift
        }
        OwnerIdentityStatus::QueryUnavailable => OwnerAdmissionFailure::QueryUnavailable,
    }
}

pub(crate) fn enter_focus_pause(
    clock_state: &mut PlaybackClockState,
    runtime: &mut super::WorkerRuntime,
    focus_ticks: QpcTicks,
    progress_clock: &SharedProgressClock,
) -> Result<bool, String> {
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

// Keep the failure-only QPC sample out of the prepared Down admission body so
// its single eligible lease-age sample remains directly auditable.
#[inline(never)]
pub(crate) fn handle_final_focus_loss(
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    runtime: &mut super::WorkerRuntime,
    progress_clock: &SharedProgressClock,
) -> Result<(), DispatchStep> {
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

/// Validate retained ownership before invoking the tracked safety-release
/// path after a post-call anomaly. This intentionally does not require current
/// foreground focus: safety Up is a distinct ownership-verified path.
pub(crate) fn verify_safety_cleanup_target<'a>(
    target: &'a SessionTarget,
    expected: TargetStamp,
    release_owner: &sky_dispatch_win32::input::ReleaseOwnerProof,
    require_owner_identity: bool,
) -> Result<parking_lot::MutexGuard<'a, ()>, String> {
    if release_owner.target_hwnd() != expected.hwnd
        || release_owner.target_generation() != expected.generation
    {
        return Err("tracked safety cleanup origin target generation drifted".into());
    }
    if !target_stamp_still_current(target, expected) {
        return Err("target HWND/generation changed before tracked safety cleanup".into());
    }
    if require_owner_identity
        || target.owner_process_identity_required()
        || release_owner.owner_pid().is_some()
    {
        let owner_pid = match target.owner_identity_status(expected.generation) {
            OwnerIdentityStatus::Bound(owner_pid) if owner_pid != 0 => owner_pid,
            OwnerIdentityStatus::Bound(_) | OwnerIdentityStatus::NotRequired => {
                return Err("tracked safety cleanup has no bound target owner PID".into());
            }
            OwnerIdentityStatus::Missing => {
                return Err("tracked safety cleanup target owner identity is missing".into());
            }
            OwnerIdentityStatus::Stale => {
                return Err("tracked safety cleanup target owner identity is stale".into());
            }
            OwnerIdentityStatus::ProcessTerminated => {
                return Err("tracked safety cleanup target process terminated".into());
            }
            OwnerIdentityStatus::OwnerMismatch => {
                return Err("tracked safety cleanup target owner mismatched".into());
            }
            OwnerIdentityStatus::WindowUnavailable => {
                return Err("tracked safety cleanup target window is unavailable".into());
            }
            OwnerIdentityStatus::ProcessIdentityMismatch => {
                return Err("tracked safety cleanup process identity drifted".into());
            }
            OwnerIdentityStatus::QueryUnavailable => {
                return Err("tracked safety cleanup target owner query is unavailable".into());
            }
        };
        if release_owner.owner_pid() != Some(owner_pid) {
            return Err("tracked safety cleanup origin owner PID changed".into());
        }
        if target.owner_process_identity_required() {
            let origin_authority = release_owner.authority().ok_or_else(|| {
                "tracked safety cleanup origin process authority is missing".to_string()
            })?;
            let authority = match target.try_owner_authority_for_generation(expected.generation) {
                Ok(authority) => authority,
                Err(OwnerAuthorityReadError::Busy) => {
                    return Err("tracked safety cleanup owner proof is busy".into());
                }
                Err(OwnerAuthorityReadError::Status(_)) => {
                    return Err("tracked safety cleanup retained owner proof is unavailable".into());
                }
            };
            if !std::sync::Arc::ptr_eq(origin_authority, &authority)
                || origin_authority.continuity()
                    != sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous
                || origin_authority.identity().owner_pid != owner_pid
                || origin_authority.identity().hwnd != expected.hwnd
            {
                return Err(
                    "tracked safety cleanup origin process proof is no longer current".into(),
                );
            }
            #[cfg(test)]
            target.run_safety_release_validation_hook_for_test();
        }
    }
    let release_permit = target
        .lock_current_release_owner_proof(release_owner)
        .ok_or_else(|| {
            "tracked safety cleanup origin owner proof changed during validation".to_string()
        })?;
    if !target_stamp_still_current(target, expected) {
        return Err("target HWND/generation changed during tracked safety proof".into());
    }
    Ok(release_permit)
}

pub(crate) enum FinalDownAdmissionResult<'a> {
    Allowed {
        release_owner: ReleaseOwnerProof,
        permit: MutexGuard<'a, ()>,
    },
    Replan,
    PermitBusy,
    Terminal(String),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_final_down_admission<'a>(
    target: &'a SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
    control_signals: FinalControlSignals<'_>,
    lease_timeout_ticks: DurationTicks,
    qpc_clock: QpcClock,
    progress_clock: &SharedProgressClock,
    backend: &TrackedKeyState,
    playback: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    metrics: &mut WorkerMetricsLocal,
) -> FinalDownAdmissionResult<'a> {
    verify_final_musical_packet_admission(
        target,
        expected,
        require_focus,
        focus_active,
        control_signals,
        lease_timeout_ticks,
        qpc_clock,
        progress_clock,
        backend,
        playback,
        runtime,
        metrics,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_final_musical_up_admission<'a>(
    target: &'a SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
    control_signals: FinalControlSignals<'_>,
    lease_timeout_ticks: DurationTicks,
    qpc_clock: QpcClock,
    progress_clock: &SharedProgressClock,
    backend: &TrackedKeyState,
    playback: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    metrics: &mut WorkerMetricsLocal,
) -> FinalDownAdmissionResult<'a> {
    verify_final_musical_packet_admission(
        target,
        expected,
        require_focus,
        focus_active,
        control_signals,
        lease_timeout_ticks,
        qpc_clock,
        progress_clock,
        backend,
        playback,
        runtime,
        metrics,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_final_musical_packet_admission<'a>(
    target: &'a SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
    control_signals: FinalControlSignals<'_>,
    lease_timeout_ticks: DurationTicks,
    qpc_clock: QpcClock,
    progress_clock: &SharedProgressClock,
    backend: &TrackedKeyState,
    playback: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    metrics: &mut WorkerMetricsLocal,
    check_modifiers: bool,
) -> FinalDownAdmissionResult<'a> {
    if final_down_control_verdict(control_signals) != FinalDownVerdict::Allowed {
        return FinalDownAdmissionResult::Replan;
    }

    let target_hint = final_down_target_hint(&FinalTargetSignals {
        expected,
        require_focus,
        focus_active,
        target,
        #[cfg(any(test, feature = "test-support"))]
        post_focus_race_hook: runtime.final_gate_post_focus_race_hook.as_ref(),
        #[cfg(any(test, feature = "test-support"))]
        post_focus_control_signals: Some(control_signals),
    });
    if target_hint != FinalDownVerdict::Allowed {
        apply_final_down_rejection(target_hint, target, expected, runtime, metrics);
        if target_hint == FinalDownVerdict::FocusLost
            && let Err(step) = handle_final_focus_loss(qpc_clock, playback, runtime, progress_clock)
        {
            return match step {
                DispatchStep::Terminate(err) => FinalDownAdmissionResult::Terminal(err),
            };
        }
        return FinalDownAdmissionResult::Replan;
    }

    // Model state changes at the final boundary before checking modifiers and
    // making the fresh foreground/owner proof. This keeps musical Up and Down
    // packets from relying on a focus stamp sampled before a wait.
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.final_gate_before_syscall_hook.as_ref() {
        hook();
    }

    if check_modifiers {
        let modifier_verdict = final_down_modifier_verdict(backend);
        if modifier_verdict != FinalDownVerdict::Allowed {
            apply_final_down_rejection(modifier_verdict, target, expected, runtime, metrics);
            return FinalDownAdmissionResult::Replan;
        }
    }

    let fresh_target_verdict = final_down_fresh_target_admission(FinalTargetSignals {
        expected,
        require_focus,
        focus_active,
        target,
        #[cfg(any(test, feature = "test-support"))]
        post_focus_race_hook: runtime.final_gate_post_focus_race_hook.as_ref(),
        #[cfg(any(test, feature = "test-support"))]
        post_focus_control_signals: Some(control_signals),
    });
    if fresh_target_verdict != FinalDownVerdict::Allowed {
        apply_final_down_rejection(fresh_target_verdict, target, expected, runtime, metrics);
        match fresh_target_verdict {
            FinalDownVerdict::FocusLost => {
                if let Err(step) =
                    handle_final_focus_loss(qpc_clock, playback, runtime, progress_clock)
                {
                    return match step {
                        DispatchStep::Terminate(err) => FinalDownAdmissionResult::Terminal(err),
                    };
                }
            }
            FinalDownVerdict::OwnerRejected(failure) => {
                return FinalDownAdmissionResult::Terminal(
                    owner_rejection_reason(failure).to_string(),
                );
            }
            _ => {}
        }
        return FinalDownAdmissionResult::Replan;
    }

    let verdict = match final_down_post_modifier_verdict(
        target,
        expected,
        require_focus,
        focus_active,
        control_signals,
        lease_timeout_ticks,
        qpc_clock,
        #[cfg(any(test, feature = "test-support"))]
        runtime.final_gate_post_lease_race_hook.as_ref(),
        #[cfg(test)]
        &mut runtime.final_lease_qpc_sample_count_for_test,
        #[cfg(test)]
        &mut runtime.final_lease_qpc_sample_override_for_test,
    ) {
        Ok(verdict) => verdict,
        Err(_) => {
            return FinalDownAdmissionResult::Terminal("prepared_down_qpc_failure".to_string());
        }
    };
    if verdict != FinalDownVerdict::Allowed {
        apply_final_down_rejection(verdict, target, expected, runtime, metrics);
        match verdict {
            FinalDownVerdict::LeaseExpired => {
                return FinalDownAdmissionResult::Terminal("supervisor_lease_expired".to_string());
            }
            FinalDownVerdict::LeaseUnproven => {
                return FinalDownAdmissionResult::Terminal("supervisor_lease_unproven".to_string());
            }
            _ => return FinalDownAdmissionResult::Replan,
        }
    }

    // A publication or focus/control race after the fresh OS identity query
    // must still fail before the caller reaches the prepared sender.
    let final_target_verdict =
        final_down_atomic_revalidation(target, expected, require_focus, focus_active, None);
    if final_target_verdict != FinalDownVerdict::Allowed {
        apply_final_down_rejection(final_target_verdict, target, expected, runtime, metrics);
        match final_target_verdict {
            FinalDownVerdict::FocusLost => {
                if let Err(step) =
                    handle_final_focus_loss(qpc_clock, playback, runtime, progress_clock)
                {
                    return match step {
                        DispatchStep::Terminate(error) => FinalDownAdmissionResult::Terminal(error),
                    };
                }
            }
            FinalDownVerdict::OwnerRejected(failure) => {
                return FinalDownAdmissionResult::Terminal(
                    owner_rejection_reason(failure).to_string(),
                );
            }
            _ => {}
        }
        return FinalDownAdmissionResult::Replan;
    }

    let final_control_verdict = final_down_control_verdict(control_signals);
    if final_control_verdict != FinalDownVerdict::Allowed {
        apply_final_down_rejection(final_control_verdict, target, expected, runtime, metrics);
        return FinalDownAdmissionResult::Replan;
    }

    // Keep the retained process object alive in the session target and query
    // its creation-time proof after lease/control checks, immediately before
    // the final atomic target check and sender boundary. Lock contention is a
    // pre-call rejection for this packet; it does not consume an onset.
    let release_owner = match fresh_final_release_owner_proof(target, expected, require_focus) {
        Ok(proof) => proof,
        Err(verdict) => {
            apply_final_down_rejection(verdict, target, expected, runtime, metrics);
            return match verdict {
                FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::ProofBusy) => {
                    FinalDownAdmissionResult::PermitBusy
                }
                FinalDownVerdict::OwnerRejected(failure) => {
                    FinalDownAdmissionResult::Terminal(owner_rejection_reason(failure).to_string())
                }
                _ => FinalDownAdmissionResult::Replan,
            };
        }
    };

    let final_identity_target_verdict =
        final_down_atomic_revalidation(target, expected, require_focus, focus_active, None);
    if final_identity_target_verdict != FinalDownVerdict::Allowed {
        apply_final_down_rejection(
            final_identity_target_verdict,
            target,
            expected,
            runtime,
            metrics,
        );
        return match final_identity_target_verdict {
            FinalDownVerdict::OwnerRejected(failure) => {
                FinalDownAdmissionResult::Terminal(owner_rejection_reason(failure).to_string())
            }
            FinalDownVerdict::FocusLost => {
                if let Err(step) =
                    handle_final_focus_loss(qpc_clock, playback, runtime, progress_clock)
                {
                    match step {
                        DispatchStep::Terminate(error) => {
                            return FinalDownAdmissionResult::Terminal(error);
                        }
                    }
                }
                FinalDownAdmissionResult::Replan
            }
            _ => FinalDownAdmissionResult::Replan,
        };
    }

    let final_control_verdict = final_down_control_verdict(control_signals);
    if final_control_verdict != FinalDownVerdict::Allowed {
        apply_final_down_rejection(final_control_verdict, target, expected, runtime, metrics);
        return FinalDownAdmissionResult::Replan;
    }

    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.final_gate_before_owner_permit_hook.as_ref() {
        hook();
    }
    let permit = match target.try_lock_current_release_owner_proof(&release_owner) {
        Ok(permit) => permit,
        Err(error) => {
            return match error {
                ReleaseOwnerPermitError::WriterBusy
                | ReleaseOwnerPermitError::OwnerAuthorityBusy => {
                    retain_current_preflight_for_retry(target, expected, runtime);
                    FinalDownAdmissionResult::PermitBusy
                }
                ReleaseOwnerPermitError::NotCurrent => {
                    runtime.verified_target = None;
                    FinalDownAdmissionResult::Replan
                }
            };
        }
    };

    FinalDownAdmissionResult::Allowed {
        release_owner,
        permit,
    }
}
