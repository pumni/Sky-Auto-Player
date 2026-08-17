use super::{TrackedKeyState, focus_gate_matches};
use sky_dispatch_core::time::DurationTicks;
#[cfg(test)]
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::clock::{QpcError, QpcTicks};
use sky_dispatch_win32::input::PhysicalKeyPreflightError;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TargetStamp {
    pub(crate) hwnd: isize,
    pub(crate) generation: u64,
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
    final_admission_qpc: QpcTicks,
    lease_timeout_ticks: DurationTicks,
    signals: FinalControlSignals<'_>,
) -> Result<FinalControlAdmission, QpcError> {
    classify_final_control(final_admission_qpc, lease_timeout_ticks, signals)
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
    DownAdmission::Allowed
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
