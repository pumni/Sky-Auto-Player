use super::{TrackedKeyState, focus_gate_matches};
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
    PauseRequested,
    QuitRequested,
    SkipRequested,
    PanicRequested,
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
    pub(crate) now_qpc: QpcTicks,
    pub(crate) lease_timeout_ticks: sky_dispatch_core::time::DurationTicks,
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn final_down_admission(
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
) -> DownAdmission {
    if !focus_matches_hwnd(require_focus, focus_active, expected.hwnd) {
        return DownAdmission::FocusLost;
    }
    if !target_stamp_still_current(target_hwnd, target_generation, expected) {
        return DownAdmission::TargetChanged;
    }
    if quit_requested.load(Ordering::Acquire) {
        return DownAdmission::QuitRequested;
    }
    if skip_requested.load(Ordering::Acquire) {
        return DownAdmission::SkipRequested;
    }
    if panic_requested.load(Ordering::Acquire) {
        return DownAdmission::PanicRequested;
    }
    if desired_pause.load(Ordering::Acquire) {
        return DownAdmission::PauseRequested;
    }
    DownAdmission::Allowed
}

/// Authoritative last-mile gate used by production Down-bearing dispatch.
/// Control state is checked before target/focus state so an explicit command
/// always wins over a newly-arriving note.  The lease check is deliberately
/// performed here, immediately before transport admission.
pub(crate) fn final_down_admission_with_lease(
    target: FinalTargetSignals<'_>,
    signals: FinalControlSignals<'_>,
) -> Result<DownAdmission, QpcError> {
    if signals.panic_requested.load(Ordering::Acquire) {
        return Ok(DownAdmission::PanicRequested);
    }
    if signals.quit_requested.load(Ordering::Acquire) {
        return Ok(DownAdmission::QuitRequested);
    }
    if signals.skip_requested.load(Ordering::Acquire) {
        return Ok(DownAdmission::SkipRequested);
    }
    if signals.desired_pause.load(Ordering::Acquire) {
        return Ok(DownAdmission::PauseRequested);
    }
    if super::supervisor_lease_expired(
        target.now_qpc,
        target.lease_timeout_ticks,
        signals.supervisor_heartbeat_ticks,
    )? {
        return Ok(DownAdmission::LeaseExpired);
    }
    if !target_stamp_still_current(
        target.target_hwnd,
        target.target_generation,
        target.expected,
    ) {
        return Ok(DownAdmission::TargetChanged);
    }
    if !focus_matches_hwnd(
        target.require_focus,
        target.focus_active,
        target.expected.hwnd,
    ) {
        return Ok(DownAdmission::FocusLost);
    }
    Ok(DownAdmission::Allowed)
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
