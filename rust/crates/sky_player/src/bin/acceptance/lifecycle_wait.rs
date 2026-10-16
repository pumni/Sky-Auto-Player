use sky_player::engine::NativeDispatchSession;
use std::thread;
use std::time::{Duration, Instant};

pub(super) fn publish_foreground_focus_hint(
    session: &NativeDispatchSession,
    target_hwnd: isize,
) -> bool {
    let focused = super::foreground_window_matches(target_hwnd);
    session.set_focus_hint(focused);
    focused
}

pub(super) fn wait_for_startup_ready(session: &NativeDispatchSession) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot();
        if snapshot.startup_ready {
            return true;
        }
        if snapshot.is_finished || Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn wait_for_focus_pause(session: &NativeDispatchSession) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let state = session.poll_state();
        if state.is_paused {
            return true;
        }
        if state.is_finished || Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn wait_for_resume(session: &NativeDispatchSession) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let state = session.poll_state();
        if !state.is_paused {
            return true;
        }
        if state.is_finished || Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}
