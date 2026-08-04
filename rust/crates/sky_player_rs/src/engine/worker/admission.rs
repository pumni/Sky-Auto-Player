use super::*;

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

pub(crate) fn focus_matches(
    require_focus: bool,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
) -> bool {
    focus_matches_hwnd(
        require_focus,
        focus_active,
        target_hwnd.load(Ordering::Acquire),
    )
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
}

#[allow(clippy::too_many_arguments)]
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

pub(crate) fn ensure_preflight_for_target(
    backend: &TrackedKeyState,
    current: TargetStamp,
    verified_target: &mut Option<TargetStamp>,
) -> Result<(), sky_dispatch_win32::input::PhysicalKeyPreflightError> {
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
