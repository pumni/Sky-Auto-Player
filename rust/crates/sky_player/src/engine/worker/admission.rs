use super::super::{PlaybackClockState, QpcClock};
use super::dispatch::DispatchStep;
use super::{TrackedKeyState, WorkerRuntime, focus_gate_matches};
use crate::engine::shared::{
    SessionTarget, SharedProgressClock, SupervisorLeaseAdmission, SupervisorLeaseAgeProof,
    SupervisorLeaseState,
};
use crate::engine::target::OwnerIdentityStatus;
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::clock::PauseReason;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcError, QpcTicks};
use sky_dispatch_win32::input::{
    ModifierKeyObservation, ModifierMask, PhysicalKeyPreflightError, ReleaseOwnerProof,
};
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(all(test, feature = "qpc-read-counting"))]
std::thread_local! {
    static FINAL_ADMISSION_ATOMIC_READS: std::cell::Cell<u64> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(all(test, feature = "qpc-read-counting"))]
fn record_final_admission_atomic_reads(reads: u64) {
    FINAL_ADMISSION_ATOMIC_READS.with(|count| {
        count.set(count.get().saturating_add(reads));
    });
}

#[cfg(all(test, feature = "qpc-read-counting"))]
pub(crate) fn reset_final_admission_atomic_reads_for_test() {
    FINAL_ADMISSION_ATOMIC_READS.with(|count| count.set(0));
}

#[cfg(all(test, feature = "qpc-read-counting"))]
pub(crate) fn final_admission_atomic_reads_for_test() -> u64 {
    FINAL_ADMISSION_ATOMIC_READS.with(std::cell::Cell::get)
}

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

    let proof = lease.prove_down_age(now, timeout_ticks);
    #[cfg(all(test, feature = "qpc-read-counting"))]
    record_final_admission_atomic_reads(u64::from(proof.state_loads));
    Ok(proof)
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
    #[cfg(all(test, feature = "qpc-read-counting"))]
    record_final_admission_atomic_reads(1);
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
    if !require_focus {
        return true;
    }
    #[cfg(all(test, feature = "qpc-read-counting"))]
    record_final_admission_atomic_reads(1);
    focus_active.load(Ordering::Acquire)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerAdmissionFailure {
    Mismatch,
    QueryUnavailable,
    IdentityAbsent,
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
}

pub(crate) fn final_control_precheck(signals: FinalControlSignals<'_>) -> FinalControlAdmission {
    #[cfg(all(test, feature = "qpc-read-counting"))]
    record_final_admission_atomic_reads(1);
    if signals.supervisor_expired.is_expired() {
        return FinalControlAdmission::PanicRequested;
    }
    #[cfg(all(test, feature = "qpc-read-counting"))]
    record_final_admission_atomic_reads(1);
    if signals.panic_requested.load(Ordering::Acquire) {
        return FinalControlAdmission::PanicRequested;
    }
    if let Some(system_power) = signals.system_power {
        #[cfg(all(test, feature = "qpc-read-counting"))]
        record_final_admission_atomic_reads(1);
        if system_power.down_blocked() {
            return FinalControlAdmission::SystemSuspendRequested;
        }
    }
    #[cfg(all(test, feature = "qpc-read-counting"))]
    record_final_admission_atomic_reads(1);
    if signals.quit_requested.load(Ordering::Acquire) {
        return FinalControlAdmission::QuitRequested;
    }
    #[cfg(all(test, feature = "qpc-read-counting"))]
    record_final_admission_atomic_reads(1);
    if signals.skip_requested.load(Ordering::Acquire) {
        return FinalControlAdmission::SkipRequested;
    }
    #[cfg(all(test, feature = "qpc-read-counting"))]
    record_final_admission_atomic_reads(1);
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

#[inline(always)]
pub(crate) fn final_down_modifier_verdict(backend: &TrackedKeyState) -> FinalDownVerdict {
    match backend.observe_modifiers_before_down() {
        ModifierKeyObservation::NoHeldObserved => FinalDownVerdict::Allowed,
        ModifierKeyObservation::HeldObserved(mask) => FinalDownVerdict::ModifierHeld(mask),
    }
}

/// Apply the common metric bucket and cached-preflight reset for a rejected
/// final Down. Focus loss is completed by each caller's existing focus-pause
/// transition, while modifiers leave target authorization intact.
#[inline(always)]
pub(crate) fn apply_final_down_rejection(
    verdict: FinalDownVerdict,
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
        FinalDownVerdict::TargetChanged
        | FinalDownVerdict::OwnerRejected(_)
        | FinalDownVerdict::LeaseExpired
        | FinalDownVerdict::LeaseUnproven => runtime.verified_target = None,
        FinalDownVerdict::Allowed
        | FinalDownVerdict::ModifierHeld(_)
        | FinalDownVerdict::FocusLost => {}
    }
}

pub(crate) fn owner_rejection_reason(failure: OwnerAdmissionFailure) -> &'static str {
    match failure {
        OwnerAdmissionFailure::Mismatch => "prepared_down_owner_mismatch",
        OwnerAdmissionFailure::QueryUnavailable => "prepared_down_owner_query_unavailable",
        OwnerAdmissionFailure::IdentityAbsent => "prepared_down_owner_identity_absent",
        OwnerAdmissionFailure::ProcessTerminated => "prepared_down_owner_process_terminated",
        OwnerAdmissionFailure::IdentityDrift => "prepared_down_owner_identity_drift",
    }
}

fn fresh_final_release_owner_proof(
    target: &SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
    release_owner: &ReleaseOwnerProof,
) -> Result<(), FinalDownVerdict> {
    if release_owner.target_hwnd() != expected.hwnd
        || release_owner.target_generation() != expected.generation
    {
        return Err(FinalDownVerdict::TargetChanged);
    }

    // Reject an already-invalidated immutable session before querying the
    // foreground or retained process. Keep the final proof check below too,
    // because the target may still be invalidated while those reads run.
    if !target.is_valid() {
        return Err(FinalDownVerdict::TargetChanged);
    }

    if !require_focus {
        return Ok(());
    }
    if expected.hwnd == 0 {
        return Err(FinalDownVerdict::FocusLost);
    }
    let Some(owner_pid) = release_owner.owner_pid().filter(|pid| *pid != 0) else {
        return Err(FinalDownVerdict::OwnerRejected(
            OwnerAdmissionFailure::IdentityAbsent,
        ));
    };
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

    // This is the last Win32 target read in the gate. It samples the current
    // foreground HWND and resolves that exact HWND's owner PID in one call.
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
    if !target.release_owner_proof_is_current(release_owner) {
        return Err(FinalDownVerdict::TargetChanged);
    }
    Ok(())
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
pub(crate) fn verify_safety_cleanup_target(
    target: &SessionTarget,
    expected: TargetStamp,
    release_owner: &sky_dispatch_win32::input::ReleaseOwnerProof,
    require_owner_identity: bool,
) -> Result<(), String> {
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
            let authority = target
                .owner_authority_for_generation(expected.generation)
                .ok_or_else(|| {
                    "tracked safety cleanup retained owner proof is unavailable".to_string()
                })?;
            if !std::sync::Arc::ptr_eq(origin_authority, authority)
                || origin_authority.continuity()
                    != sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous
                || origin_authority.identity().owner_pid != owner_pid
                || origin_authority.identity().hwnd != expected.hwnd
            {
                return Err(
                    "tracked safety cleanup origin process proof is no longer current".into(),
                );
            }
            match origin_authority.verify_final_process_identity(expected.hwnd, owner_pid) {
                sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous => {}
                sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated => {
                    return Err("tracked safety cleanup origin process terminated".into());
                }
                sky_dispatch_win32::focus::WindowIdentityContinuity::OwnerMismatch => {
                    return Err("tracked safety cleanup origin owner mismatched".into());
                }
                sky_dispatch_win32::focus::WindowIdentityContinuity::WindowUnavailable
                | sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessIdentityMismatch => {
                    return Err("tracked safety cleanup origin identity drifted".into());
                }
                sky_dispatch_win32::focus::WindowIdentityContinuity::QueryUnavailable => {
                    return Err("tracked safety cleanup origin liveness is unavailable".into());
                }
            }
        }
    }
    if !target.release_owner_proof_is_current(release_owner) {
        return Err("tracked safety cleanup origin owner proof is no longer current".into());
    }
    Ok(())
}

pub(crate) fn safety_cleanup_drain_decision(
    target: &SessionTarget,
    expected: TargetStamp,
    release_owner: &ReleaseOwnerProof,
    require_owner_identity: bool,
    terminal_requested: bool,
) -> Result<sky_dispatch_win32::input::CleanupDrainDecision, String> {
    verify_safety_cleanup_target(target, expected, release_owner, require_owner_identity)?;
    let origin_foreground = if target.foreground_required() {
        let owner_pid = release_owner.owner_pid().ok_or_else(|| {
            "tracked focus-required cleanup has no bound origin owner PID".to_string()
        })?;
        match sky_dispatch_win32::focus::foreground_window_owner_matches(expected.hwnd, owner_pid) {
            sky_dispatch_win32::focus::ForegroundOwnerMatch::Match => true,
            sky_dispatch_win32::focus::ForegroundOwnerMatch::NotForeground => false,
            sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerMismatch => {
                return Err("tracked cleanup origin HWND owner identity drifted".into());
            }
            sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerQueryUnavailable => {
                return Err("tracked cleanup foreground owner query is unavailable".into());
            }
        }
    } else {
        // A focus-free session may retain an owner PID solely to prove the
        // immutable window/process identity. That PID does not impose a
        // foreground policy on startup or terminal cleanup.
        true
    };
    Ok(sky_dispatch_win32::input::CleanupDrainDecision {
        terminal_requested,
        origin_foreground,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MusicalPacketKind {
    DownOnly,
    UpOnly,
    Mixed,
}

impl MusicalPacketKind {
    #[inline(always)]
    pub(crate) fn from_masks(down_mask: u16, up_mask: u16) -> Option<Self> {
        match (down_mask != 0, up_mask != 0) {
            (true, false) => Some(Self::DownOnly),
            (false, true) => Some(Self::UpOnly),
            (true, true) => Some(Self::Mixed),
            (false, false) => None,
        }
    }

    #[inline(always)]
    pub(crate) fn has_down(self) -> bool {
        matches!(self, Self::DownOnly | Self::Mixed)
    }

    #[inline(always)]
    pub(crate) fn has_up(self) -> bool {
        matches!(self, Self::UpOnly | Self::Mixed)
    }
}

/// Qualified precheck proof captured during Candidate B's pre-spin lead window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PrecheckProof {
    pub(crate) precheck_begin_qpc: QpcTicks,
    pub(crate) max_age_ticks: DurationTicks,
    pub(crate) target_stamp: TargetStamp,
    pub(crate) owner_pid: u32,
    pub(crate) lease_expiry_qpc: Option<QpcTicks>,
    pub(crate) packet_kind: MusicalPacketKind,
}

impl PrecheckProof {
    #[inline(always)]
    pub(crate) fn is_fresh(&self, now_qpc: QpcTicks) -> bool {
        if now_qpc < self.precheck_begin_qpc {
            return false;
        }
        let age_ticks = DurationTicks::from_raw(
            now_qpc
                .as_u64()
                .saturating_sub(self.precheck_begin_qpc.as_u64()),
        );
        if age_ticks > self.max_age_ticks {
            return false;
        }
        if let Some(expiry) = self.lease_expiry_qpc
            && now_qpc >= expiry
        {
            return false;
        }
        true
    }
}

#[derive(Debug)]
pub(crate) enum FinalDownAdmissionResult {
    Allowed,
    ModifierHeld,
    Replan,
    Terminal(String),
}

/// Single typed entry point evaluating musical admission across Down-only,
/// Up-only, and mixed packets. Eliminates redundant wrapper and repeated
/// target validation layers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_final_packet_admission(
    target: &SessionTarget,
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
    packet_kind: MusicalPacketKind,
) -> FinalDownAdmissionResult {
    // 1. Control check: Down-bearing callers check early before target/modifier.
    if packet_kind.has_down()
        && final_down_control_verdict(control_signals) != FinalDownVerdict::Allowed
    {
        return FinalDownAdmissionResult::Replan;
    }

    // 2. Cheap target and published-focus hint.
    let target_hint = final_down_target_hint(&FinalTargetSignals {
        expected,
        require_focus,
        focus_active,
        target,
    });
    if target_hint != FinalDownVerdict::Allowed {
        apply_final_down_rejection(target_hint, runtime, metrics);
        if target_hint == FinalDownVerdict::FocusLost
            && let Err(step) = handle_final_focus_loss(qpc_clock, playback, runtime, progress_clock)
        {
            return match step {
                DispatchStep::Terminate(err) => FinalDownAdmissionResult::Terminal(err),
            };
        }
        return FinalDownAdmissionResult::Replan;
    }

    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.final_gate_before_syscall_hook.as_ref() {
        hook();
    }

    // 3. Modifiers are scanned only for Down-bearing packets; Up-only is independent.
    if packet_kind.has_down() {
        let modifier_verdict = final_down_modifier_verdict(backend);
        if modifier_verdict != FinalDownVerdict::Allowed {
            apply_final_down_rejection(modifier_verdict, runtime, metrics);
            return match modifier_verdict {
                FinalDownVerdict::ModifierHeld(_) => FinalDownAdmissionResult::ModifierHeld,
                _ => FinalDownAdmissionResult::Replan,
            };
        }
    }

    // 4. Supervisor lease proof with bounded age sample.
    let lease_proof = match final_down_lease_age_proof(
        control_signals.supervisor_expired,
        lease_timeout_ticks,
        qpc_clock,
        #[cfg(test)]
        &mut runtime.final_lease_qpc_sample_count_for_test,
        #[cfg(test)]
        &mut runtime.final_lease_qpc_sample_override_for_test,
    ) {
        Ok(proof) => proof,
        Err(_) => {
            return FinalDownAdmissionResult::Terminal("prepared_down_qpc_failure".to_string());
        }
    };
    let lease_verdict = match lease_proof.admission {
        SupervisorLeaseAdmission::Fresh => FinalDownVerdict::Allowed,
        SupervisorLeaseAdmission::Expired => FinalDownVerdict::LeaseExpired,
        SupervisorLeaseAdmission::Unproven => FinalDownVerdict::LeaseUnproven,
    };
    if lease_verdict != FinalDownVerdict::Allowed {
        apply_final_down_rejection(lease_verdict, runtime, metrics);
        return match lease_verdict {
            FinalDownVerdict::LeaseExpired => {
                FinalDownAdmissionResult::Terminal("supervisor_lease_expired".to_string())
            }
            FinalDownVerdict::LeaseUnproven => {
                FinalDownAdmissionResult::Terminal("supervisor_lease_unproven".to_string())
            }
            _ => FinalDownAdmissionResult::Replan,
        };
    }

    #[cfg(any(test, feature = "test-support"))]
    if let (Some(hook), Some(system_power)) = (
        runtime.final_gate_post_lease_race_hook.as_ref(),
        control_signals.system_power,
    ) {
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

    // 5. Retained process liveness and exact foreground HWND/PID match.
    if require_focus {
        let Some(release_owner) = backend.session_release_owner_proof() else {
            return FinalDownAdmissionResult::Terminal(
                "session release owner proof is missing before final admission".to_string(),
            );
        };
        if let Err(verdict) = fresh_final_release_owner_proof(target, expected, true, release_owner)
        {
            apply_final_down_rejection(verdict, runtime, metrics);
            if verdict == FinalDownVerdict::FocusLost
                && let Err(step) =
                    handle_final_focus_loss(qpc_clock, playback, runtime, progress_clock)
            {
                return match step {
                    DispatchStep::Terminate(err) => FinalDownAdmissionResult::Terminal(err),
                };
            }
            return match verdict {
                FinalDownVerdict::OwnerRejected(failure) => {
                    FinalDownAdmissionResult::Terminal(owner_rejection_reason(failure).to_string())
                }
                FinalDownVerdict::TargetChanged => {
                    FinalDownAdmissionResult::Terminal("prepared_down_target_changed".to_string())
                }
                _ => FinalDownAdmissionResult::Replan,
            };
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    if let (Some(hook), Some(system_power)) = (
        runtime.final_gate_post_focus_race_hook.as_ref(),
        control_signals.system_power,
    ) {
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

    // 6. Final control state check immediately before raw send.
    let final_control_verdict = final_down_control_verdict(control_signals);
    if final_control_verdict != FinalDownVerdict::Allowed {
        apply_final_down_rejection(final_control_verdict, runtime, metrics);
        return FinalDownAdmissionResult::Replan;
    }

    FinalDownAdmissionResult::Allowed
}

/// Candidate B precheck evaluated ahead of the crossing boundary at
/// `eligible_qpc - precheck_lead`. Samples modifiers, process liveness, and lease age.
#[allow(clippy::too_many_arguments)]
pub(crate) fn perform_candidate_b_precheck(
    target: &SessionTarget,
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
    control_signals: FinalControlSignals<'_>,
    lease_timeout_ticks: DurationTicks,
    qpc_clock: QpcClock,
    backend: &TrackedKeyState,
    packet_kind: MusicalPacketKind,
    max_proof_age_ticks: DurationTicks,
) -> Result<PrecheckProof, FinalDownAdmissionResult> {
    if final_control_precheck(control_signals) != FinalControlAdmission::Allowed {
        return Err(FinalDownAdmissionResult::Replan);
    }
    if !target_stamp_still_current(target, expected) || !target.is_valid() {
        return Err(FinalDownAdmissionResult::Terminal(
            "prepared_down_target_changed".to_string(),
        ));
    }
    if !focus_matches(require_focus, focus_active) {
        return Err(FinalDownAdmissionResult::Replan);
    }
    if packet_kind.has_down()
        && matches!(
            backend.observe_modifiers_before_down(),
            ModifierKeyObservation::HeldObserved(_)
        )
    {
        return Err(FinalDownAdmissionResult::ModifierHeld);
    }
    let precheck_begin_qpc = qpc_clock
        .now()
        .map_err(|_| FinalDownAdmissionResult::Terminal("prepared_down_qpc_failure".to_string()))?;
    let lease_proof = control_signals
        .supervisor_expired
        .prove_down_age(precheck_begin_qpc, lease_timeout_ticks);
    if lease_proof.admission != SupervisorLeaseAdmission::Fresh {
        return Err(FinalDownAdmissionResult::Terminal(
            "supervisor_lease_expired".to_string(),
        ));
    }
    let lease_expiry_qpc = if lease_timeout_ticks > DurationTicks::ZERO {
        Some(precheck_begin_qpc.saturating_add_duration(lease_timeout_ticks))
    } else {
        None
    };

    let owner_pid = if require_focus {
        let Some(release_owner) = backend.session_release_owner_proof() else {
            return Err(FinalDownAdmissionResult::Terminal(
                "session release owner proof is missing before final admission".to_string(),
            ));
        };
        if release_owner.target_hwnd() != expected.hwnd
            || release_owner.target_generation() != expected.generation
        {
            return Err(FinalDownAdmissionResult::Terminal(
                "prepared_down_target_changed".to_string(),
            ));
        }
        let Some(owner_pid) = release_owner.owner_pid().filter(|pid| *pid != 0) else {
            return Err(FinalDownAdmissionResult::Terminal(
                "prepared_down_owner_identity_absent".to_string(),
            ));
        };
        if target.owner_process_identity_required() {
            let Some(authority) = release_owner.authority() else {
                return Err(FinalDownAdmissionResult::Terminal(
                    "prepared_down_owner_identity_absent".to_string(),
                ));
            };
            if authority.verify_final_process_identity(expected.hwnd, owner_pid)
                != sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous
            {
                return Err(FinalDownAdmissionResult::Terminal(
                    "prepared_down_owner_process_terminated".to_string(),
                ));
            }
        }
        owner_pid
    } else {
        0
    };

    Ok(PrecheckProof {
        precheck_begin_qpc,
        max_age_ticks: max_proof_age_ticks,
        target_stamp: expected,
        owner_pid,
        lease_expiry_qpc,
        packet_kind,
    })
}

/// Candidate B crossing check. If precheck proof is fresh, executes exactly one
/// admission query (foreground HWND/PID); otherwise cleanly falls back to Control A.
/// Fail-closed invariants (target invalidation, release owner unbind/mismatch,
/// window destruction, process exit, control exit/suspend) terminate immediately.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_candidate_b_crossing(
    precheck: Option<&PrecheckProof>,
    target: &SessionTarget,
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
    packet_kind: MusicalPacketKind,
) -> (FinalDownAdmissionResult, bool) {
    let now_qpc = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(_) => {
            return (
                FinalDownAdmissionResult::Terminal("prepared_down_qpc_failure".to_string()),
                false,
            );
        }
    };

    // Target invalidation is latching and must fail-closed immediately
    if !target.is_valid() {
        return (
            FinalDownAdmissionResult::Terminal("prepared_down_target_invalidated".to_string()),
            false,
        );
    }

    // Release owner binding must remain valid and match expected target
    let Some(release_owner) = backend.session_release_owner_proof() else {
        return (
            FinalDownAdmissionResult::Terminal(
                "session release owner proof is missing before final admission".to_string(),
            ),
            false,
        );
    };
    if release_owner.target_hwnd() != expected.hwnd
        || release_owner.target_generation() != expected.generation
    {
        return (
            FinalDownAdmissionResult::Terminal("prepared_down_target_changed".to_string()),
            false,
        );
    }

    // Target HWND destruction / reuse check
    if !sky_dispatch_win32::focus::is_window_live(expected.hwnd) {
        return (
            FinalDownAdmissionResult::Terminal("prepared_down_window_unavailable".to_string()),
            false,
        );
    }

    // Process termination check on retained authority
    if target.owner_process_identity_required()
        && let Some(authority) = release_owner.authority()
    {
        let expected_owner_pid = release_owner.owner_pid().unwrap_or(0);
        if authority.verify_final_process_identity(expected.hwnd, expected_owner_pid)
            != sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous
        {
            return (
                FinalDownAdmissionResult::Terminal(
                    "prepared_down_owner_process_terminated".to_string(),
                ),
                false,
            );
        }
    }

    // Control and termination signals check
    if !packet_kind.has_down() {
        match final_control_precheck(control_signals) {
            FinalControlAdmission::Allowed => {}
            FinalControlAdmission::PauseRequested => {
                return (FinalDownAdmissionResult::Replan, true);
            }
            FinalControlAdmission::SystemSuspendRequested => {
                runtime.terminal_trigger = Some(super::control::TerminalTrigger::SystemSuspend);
                return (
                    FinalDownAdmissionResult::Terminal("prepared_down_system_suspend".to_string()),
                    false,
                );
            }
            FinalControlAdmission::PanicRequested
            | FinalControlAdmission::QuitRequested
            | FinalControlAdmission::SkipRequested => {
                return (
                    FinalDownAdmissionResult::Terminal("prepared_down_control_exit".to_string()),
                    false,
                );
            }
        }
    } else if final_down_control_verdict(control_signals) != FinalDownVerdict::Allowed {
        return (FinalDownAdmissionResult::Replan, true);
    }

    if let Some(proof) = precheck
        && proof.is_fresh(now_qpc)
        && proof.target_stamp == expected
        && proof.packet_kind == packet_kind
    {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = runtime.final_gate_before_syscall_hook.as_ref() {
            hook();
        }

        if require_focus {
            match sky_dispatch_win32::focus::foreground_window_owner_matches(
                expected.hwnd,
                proof.owner_pid,
            ) {
                sky_dispatch_win32::focus::ForegroundOwnerMatch::Match => {}
                sky_dispatch_win32::focus::ForegroundOwnerMatch::NotForeground => {
                    let _ = handle_final_focus_loss(qpc_clock, playback, runtime, progress_clock);
                    return (FinalDownAdmissionResult::Replan, true);
                }
                sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerMismatch => {
                    return (
                        FinalDownAdmissionResult::Terminal(
                            "prepared_down_owner_mismatch".to_string(),
                        ),
                        true,
                    );
                }
                sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerQueryUnavailable => {
                    return (
                        FinalDownAdmissionResult::Terminal(
                            "prepared_down_owner_query_unavailable".to_string(),
                        ),
                        true,
                    );
                }
            }
        }

        #[cfg(any(test, feature = "test-support"))]
        if let (Some(hook), Some(system_power)) = (
            runtime.final_gate_post_focus_race_hook.as_ref(),
            control_signals.system_power,
        ) {
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

        return (FinalDownAdmissionResult::Allowed, true);
    }

    // Missing or stale precheck proof: fallback to full Control A admission
    let fallback_result = verify_final_packet_admission(
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
        packet_kind,
    );
    (fallback_result, false)
}

#[cfg(test)]
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn verify_final_down_admission(
    target: &SessionTarget,
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
) -> FinalDownAdmissionResult {
    verify_final_packet_admission(
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
        MusicalPacketKind::DownOnly,
    )
}

#[cfg(test)]
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn verify_final_musical_up_admission(
    target: &SessionTarget,
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
) -> FinalDownAdmissionResult {
    verify_final_packet_admission(
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
        MusicalPacketKind::UpOnly,
    )
}
