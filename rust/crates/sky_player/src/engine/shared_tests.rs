use super::{
    SYSTEM_POWER_PENDING_MASK, SYSTEM_POWER_RESUME_PENDING, SYSTEM_POWER_SUSPEND_PENDING,
    SharedProgressClock, SystemPowerEndpoint, SystemPowerState,
};
use sky_dispatch_core::clock::{PauseReason, PlaybackClockState};
use sky_dispatch_core::time::{DurationTicks, QpcTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::event::OwnedEvent;
use std::num::NonZeroU64;
use std::sync::atomic::Ordering;

fn test_qpc_clock() -> QpcClock {
    QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap())
}

#[test]
fn progress_projection_advances_without_worker_publication() {
    let clock = PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
    let shared = SharedProgressClock::default();
    shared.publish(&clock);
    let anchor = shared.load().expect("published playback anchor");
    let qpc_clock = test_qpc_clock();

    assert_eq!(
        anchor.elapsed_us(QpcTicks::from_raw(2_000), qpc_clock),
        1_000
    );
    assert_eq!(
        anchor.elapsed_us(QpcTicks::from_raw(3_000), qpc_clock),
        2_000
    );
}

#[test]
fn progress_projection_freezes_pause_and_excludes_pause_on_resume() {
    let mut clock =
        PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
    let shared = SharedProgressClock::default();
    let qpc_clock = test_qpc_clock();

    clock
        .enter_pause(PauseReason::Manual, QpcTicks::from_raw(2_000))
        .unwrap();
    shared.publish(&clock);
    let paused_anchor = shared.load().expect("published pause anchor");
    assert!(paused_anchor.paused);
    assert_eq!(
        paused_anchor.elapsed_us(QpcTicks::from_raw(9_000), qpc_clock),
        1_000
    );

    clock
        .exit_pause(PauseReason::Manual, QpcTicks::from_raw(5_000))
        .unwrap();
    shared.publish(&clock);
    let resumed_anchor = shared.load().expect("published resume anchor");
    assert!(!resumed_anchor.paused);
    assert_eq!(
        resumed_anchor.elapsed_us(QpcTicks::from_raw(7_000), qpc_clock),
        3_000
    );
}

#[test]
fn progress_projection_clamps_before_future_epoch() {
    let clock = PlaybackClockState::new(QpcTicks::from_raw(2_000), DurationTicks::ZERO).unwrap();
    let shared = SharedProgressClock::default();
    shared.publish(&clock);
    let anchor = shared.load().expect("published future anchor");

    assert_eq!(
        anchor.elapsed_us(QpcTicks::from_raw(1_000), test_qpc_clock()),
        0
    );
}

#[test]
fn terminal_progress_projection_stays_frozen() {
    let clock = PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
    let shared = SharedProgressClock::default();
    shared.publish_terminal(&clock, QpcTicks::from_raw(3_000));
    let anchor = shared.load().expect("published terminal anchor");

    assert!(anchor.frozen);
    assert_eq!(
        anchor.elapsed_us(QpcTicks::from_raw(9_000), test_qpc_clock()),
        2_000
    );
}

#[test]
fn focus_pause_before_future_epoch_remains_clamped() {
    let mut clock =
        PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
    clock
        .enter_pause(PauseReason::Focus, QpcTicks::from_raw(900))
        .unwrap();
    let shared = SharedProgressClock::default();
    shared.publish(&clock);
    let anchor = shared.load().expect("published focus anchor");

    assert_eq!(
        anchor.elapsed_us(QpcTicks::from_raw(2_000), test_qpc_clock()),
        0
    );
}

#[test]
fn power_notifications_are_idempotent_and_down_stays_blocked_until_worker_resume() {
    let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
    let power = SystemPowerState::default();

    assert!(power.notify(true, &interrupt));
    assert!(!power.notify(true, &interrupt));
    assert!(power.os_suspended());
    assert!(power.down_blocked());
    assert_eq!(power.take_pending(), SYSTEM_POWER_SUSPEND_PENDING);

    assert!(power.notify(false, &interrupt));
    assert!(!power.notify(false, &interrupt));
    assert!(!power.os_suspended());
    assert!(power.down_blocked());
    assert_eq!(power.take_pending(), SYSTEM_POWER_RESUME_PENDING);

    assert!(power.complete_resume());
    assert!(!power.down_blocked());
    let (_, _, suspends, resumes, duplicates) = power.snapshot();
    assert_eq!((suspends, resumes, duplicates), (1, 1, 2));

    assert!(power.notify(true, &interrupt));
    assert!(!power.complete_resume());
    assert!(power.down_blocked());
}

#[test]
fn pending_power_fast_path_returns_without_consume_when_clear() {
    let power = SystemPowerState::default();

    assert_eq!(power.take_pending(), 0);
    assert_eq!(power.take_pending(), 0);
}

#[test]
fn notify_cas_exhaustion_fails_closed_until_current_epoch_resume() {
    let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
    let power = SystemPowerState::default();

    power.force_notify_cas_failures_for_test(true);
    assert!(!power.notify_at(true, Some(QpcTicks::from_raw(40_000)), &interrupt));
    assert!(power.os_suspended());
    assert!(power.down_blocked());
    assert_eq!(
        power.state.load(Ordering::Acquire) & SYSTEM_POWER_PENDING_MASK,
        SYSTEM_POWER_SUSPEND_PENDING
    );
    assert_eq!(
        power.suspend_boundary_qpc(),
        Some(QpcTicks::from_raw(40_000))
    );
    assert_eq!(interrupt.signal_generation(), 1);
    let (_, _, suspends, resumes, duplicates) = power.snapshot();
    assert_eq!((suspends, resumes, duplicates), (0, 0, 0));

    power.force_notify_cas_failures_for_test(false);
    assert!(power.notify_at(false, None, &interrupt));
    assert!(!power.os_suspended());
    assert!(power.down_blocked());
    assert_eq!(
        power.state.load(Ordering::Acquire) & SYSTEM_POWER_PENDING_MASK,
        SYSTEM_POWER_SUSPEND_PENDING | SYSTEM_POWER_RESUME_PENDING
    );
    assert!(!power.complete_resume());
    assert_eq!(
        power.take_pending(),
        SYSTEM_POWER_SUSPEND_PENDING | SYSTEM_POWER_RESUME_PENDING
    );
    assert!(power.complete_resume());
    assert!(!power.down_blocked());
    assert_eq!(interrupt.signal_generation(), 2);
}

#[test]
fn pending_power_source_loads_before_conditional_consume() {
    let source = include_str!("shared.rs");
    let method = source
        .split("pub(super) fn take_pending")
        .nth(1)
        .expect("pending power method")
        .split("pub(super) fn down_blocked")
        .next()
        .expect("pending power method body");
    assert!(method.find("load(Ordering::Acquire)").unwrap() < method.find("fetch_and(").unwrap());
    assert!(method.contains("return 0"));
}

#[test]
fn suspend_boundary_is_captured_before_worker_wakes_and_survives_resume_notification() {
    let endpoint = SystemPowerEndpoint::new().expect("power endpoint");
    endpoint.reset_for_new_session().expect("new power epoch");
    let suspend_qpc = QpcTicks::from_raw(10_000);
    assert!(endpoint.notify_system_power(true, Some(suspend_qpc)));
    // The worker has not run yet. A resume callback may arrive while it is
    // asleep; the original suspend boundary must remain available.
    assert!(endpoint.notify_system_power(false, None));
    assert_eq!(endpoint.state().suspend_boundary_qpc(), Some(suspend_qpc));
}

#[test]
fn callback_suspend_boundary_excludes_sleep_sized_qpc_interval() {
    let mut clock =
        PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
    let endpoint = SystemPowerEndpoint::new().expect("power endpoint");
    endpoint.reset_for_new_session().expect("new power epoch");
    let suspend_qpc = QpcTicks::from_raw(2_000);
    let resume_qpc = QpcTicks::from_raw(2_000_000_000);
    assert!(endpoint.notify_system_power(true, Some(suspend_qpc)));
    assert!(endpoint.notify_system_power(false, None));
    clock
        .enter_pause(
            PauseReason::SystemSuspend,
            endpoint.state().suspend_boundary_qpc().unwrap(),
        )
        .unwrap();
    clock
        .exit_pause(PauseReason::SystemSuspend, resume_qpc)
        .unwrap();
    assert_eq!(
        test_qpc_clock()
            .duration_to_us(DurationTicks::from_raw(
                clock.get_elapsed(resume_qpc).unwrap().as_u64(),
            ))
            .unwrap(),
        1_000
    );
}

#[test]
fn stale_resume_callback_crossing_session_reset_can_clear_new_session_suspend() {
    use std::sync::Arc;

    let state = Arc::new(SystemPowerState::default());
    let old_interrupt = Arc::new(OwnedEvent::new_auto_reset().expect("old session interrupt"));
    let new_interrupt = OwnedEvent::new_auto_reset().expect("new session interrupt");
    assert!(state.notify_at(true, Some(QpcTicks::from_raw(10_000)), &old_interrupt));

    state.pause_after_epoch_capture_for_test();
    let callback_state = Arc::clone(&state);
    let callback_interrupt = Arc::clone(&old_interrupt);
    let old_resume =
        std::thread::spawn(move || callback_state.notify_at(false, None, &callback_interrupt));

    // The callback captured the old epoch but has not acquired mutation
    // authority. Reset can close and drain without waiting for it; a new
    // suspend then proves that releasing the stale callback is harmless.
    while !state.test_pause(true).reached.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    state.deactivate(&old_interrupt);
    state
        .reset_for_new_session(&new_interrupt)
        .expect("open next power epoch");
    assert!(state.notify_at(true, Some(QpcTicks::from_raw(20_000)), &new_interrupt));
    let current_signal_generation = new_interrupt.signal_generation();
    state
        .test_pause(true)
        .released
        .store(true, Ordering::Release);
    assert!(!old_resume.join().expect("old resume callback is fenced"));

    let os_suspended_after_stale_resume = state.os_suspended();
    let pending = state.state.load(Ordering::Acquire) & SYSTEM_POWER_PENDING_MASK;
    let resume_completed = state.complete_resume();
    let down_blocked_after_resume = state.down_blocked();
    let (_, _, suspends, resumes, duplicates) = state.snapshot();
    assert!(
        os_suspended_after_stale_resume
            && pending == SYSTEM_POWER_SUSPEND_PENDING
            && !resume_completed
            && down_blocked_after_resume
            && (suspends, resumes, duplicates) == (1, 0, 0)
            && state.suspend_boundary_qpc() == Some(QpcTicks::from_raw(20_000))
            && new_interrupt.signal_generation() == current_signal_generation,
        "old-session resume altered new-session suspend: os_suspended={os_suspended_after_stale_resume}, pending={pending:#04x}, resume_completed={resume_completed}, down_blocked={down_blocked_after_resume}, counts={:?}",
        (suspends, resumes, duplicates)
    );
}

#[test]
fn reset_waits_for_callback_that_already_acquired_epoch_authority() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let state = Arc::new(SystemPowerState::default());
    let interrupt = Arc::new(OwnedEvent::new_auto_reset().expect("power interrupt"));
    state.pause_after_lease_for_test();

    let callback_state = Arc::clone(&state);
    let callback_interrupt = Arc::clone(&interrupt);
    let callback = std::thread::spawn(move || {
        callback_state.notify_at(true, Some(QpcTicks::from_raw(30_000)), &callback_interrupt)
    });
    while !state.test_pause(false).reached.load(Ordering::Acquire) {
        std::thread::yield_now();
    }

    let reset_started = Arc::new(AtomicBool::new(false));
    let reset_finished = Arc::new(AtomicBool::new(false));
    let reset_state = Arc::clone(&state);
    let reset_interrupt = Arc::clone(&interrupt);
    let started = Arc::clone(&reset_started);
    let finished = Arc::clone(&reset_finished);
    let reset = std::thread::spawn(move || {
        started.store(true, Ordering::Release);
        reset_state
            .reset_for_new_session(&reset_interrupt)
            .expect("new epoch after rundown");
        finished.store(true, Ordering::Release);
    });
    while !reset_started.load(Ordering::Acquire)
        || state.callback_epoch.load(Ordering::SeqCst) & 1 == 0
    {
        std::thread::yield_now();
    }
    assert!(!reset_finished.load(Ordering::Acquire));
    state
        .test_pause(false)
        .released
        .store(true, Ordering::Release);
    assert!(callback.join().expect("old callback completes"));
    reset.join().expect("reset completes after rundown");

    assert!(reset_finished.load(Ordering::Acquire));
    assert_eq!(state.snapshot(), (false, false, 0, 0, 0));
    assert_eq!(state.suspend_boundary_qpc(), None);
    assert!(!interrupt.try_take(), "old callback event was drained");
    assert_eq!(interrupt.signal_generation(), 1);
}

#[test]
fn stale_suspend_cannot_publish_boundary_counters_or_signal_after_reset() {
    use std::sync::Arc;

    let state = Arc::new(SystemPowerState::default());
    let old_interrupt = Arc::new(OwnedEvent::new_auto_reset().expect("old interrupt"));
    let new_interrupt = OwnedEvent::new_auto_reset().expect("new interrupt");
    state.pause_after_epoch_capture_for_test();

    let callback_state = Arc::clone(&state);
    let callback_interrupt = Arc::clone(&old_interrupt);
    let stale_suspend = std::thread::spawn(move || {
        callback_state.notify_at(true, Some(QpcTicks::from_raw(40_000)), &callback_interrupt)
    });
    while !state.test_pause(true).reached.load(Ordering::Acquire) {
        std::thread::yield_now();
    }

    state
        .reset_for_new_session(&new_interrupt)
        .expect("open new epoch while stale callback is pre-lease");
    state
        .test_pause(true)
        .released
        .store(true, Ordering::Release);
    assert!(!stale_suspend.join().expect("stale suspend is fenced"));

    assert_eq!(state.snapshot(), (false, false, 0, 0, 0));
    assert_eq!(state.suspend_boundary_qpc(), None);
    assert_eq!(new_interrupt.signal_generation(), 0);
    assert!(!new_interrupt.try_take());
}

#[test]
fn callback_epoch_exhaustion_stays_closed_without_wrapping() {
    let state = SystemPowerState::default();
    let interrupt = OwnedEvent::new_auto_reset().expect("power interrupt");
    state.callback_epoch.store(u64::MAX - 1, Ordering::SeqCst);

    let error = state
        .reset_for_new_session(&interrupt)
        .expect_err("epoch exhaustion must fail closed");
    assert!(error.contains("epoch exhausted"));
    assert_eq!(state.callback_epoch.load(Ordering::SeqCst), u64::MAX);
    assert!(!state.notify_at(true, Some(QpcTicks::from_raw(1)), &interrupt));
    assert_eq!(state.snapshot(), (false, false, 0, 0, 0));
}
