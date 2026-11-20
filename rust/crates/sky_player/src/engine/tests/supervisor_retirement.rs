use super::support::*;
use super::*;
use crate::engine::snapshot::WorkerProgressState;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_win32::focus::WindowIdentityContinuity;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const TARGET_HWND: isize = 0x1234;
const TARGET_PID: u32 = 0x5678;

struct ForegroundOverrideReset;

impl Drop for ForegroundOverrideReset {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
    }
}

fn install_foreground(hwnd: isize, pid: u32) {
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(hwnd));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(pid)));
}

fn wait_until(timeout: Duration, condition: impl Fn() -> bool, description: &'static str) {
    let start = Instant::now();
    while !condition() {
        if start.elapsed() > timeout {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn schedule_long(keys: &[u16]) -> sky_dispatch_core::model::RuntimeSchedule {
    let scan_codes = smallvec::SmallVec::from_slice(keys);
    sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: scan_codes.clone(),
                reason: "initial-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 10_000_000,
                scan_codes,
                reason: "distant-up".into(),
            },
        ],
        keys,
    )
    .expect("compile long schedule")
}

#[test]
fn worker_progress_state_canonical_strings() {
    assert_eq!(WorkerProgressState::Starting.as_str(), "starting");
    assert_eq!(WorkerProgressState::Active.as_str(), "active");
    assert_eq!(WorkerProgressState::Paused.as_str(), "paused");
    assert_eq!(WorkerProgressState::Draining.as_str(), "draining");
}

#[test]
fn supervisor_lifetime_guard_detects_unwind_and_signals_panic_release_once() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));

    let sched = schedule_long(&[0x15]);
    let mut options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.focus.require_focus = true;
    options.wait.supervisor_lease_timeout_us = 5_000_000;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind target owner");
    session.set_focus_hint(true);

    let guard = session.supervisor_lifetime_guard();
    assert!(!guard.is_disarmed());

    session.arm(0).expect("arm worker loop");

    wait_until(
        Duration::from_secs(3),
        || send_calls.load(Ordering::Acquire) >= 1,
        "first down send",
    );

    // Simulate unexpected supervisor thread unwind/exit by dropping the undisarmed guard:
    drop(guard);

    assert!(
        session
            .join(Duration::from_secs(3))
            .expect("join after guard drop"),
        "worker must terminate after supervisor guard drop"
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("supervisor_unwind_or_exit")
    );
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(snapshot.active_count, 0);
}

#[test]
fn supervisor_lifetime_guard_disarmed_on_intentional_quit() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15]);
    let mut options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.focus.require_focus = true;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind target owner");
    session.set_focus_hint(true);

    let guard = session.supervisor_lifetime_guard();
    session.arm(0).expect("arm worker loop");

    session.quit().expect("intentional quit");
    assert!(guard.is_disarmed(), "guard must be disarmed on quit");

    drop(guard);

    assert!(
        session
            .join(Duration::from_secs(3))
            .expect("join after quit"),
        "worker must join cleanly"
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Quit.as_str());
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("quit"),
        "terminal trigger reason must remain quit"
    );
}

#[test]
fn supervisor_lifetime_guard_disarmed_on_intentional_skip() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15]);
    let mut options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.focus.require_focus = true;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind target owner");
    session.set_focus_hint(true);

    let guard = session.supervisor_lifetime_guard();
    session.arm(0).expect("arm worker loop");

    session.skip().expect("intentional skip");
    assert!(guard.is_disarmed(), "guard must be disarmed on skip");

    drop(guard);

    assert!(
        session
            .join(Duration::from_secs(3))
            .expect("join after skip"),
        "worker must join cleanly"
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Skipped.as_str());
    assert_eq!(snapshot.terminal_trigger_reason.as_deref(), Some("skipped"));
}

#[test]
fn supervisor_signal_once_race_between_watchdog_and_guard() {
    let sched = schedule_long(&[0x15]);
    let options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    let session = NativeDispatchSession::new(options).expect("session admission");
    let shared = session.shared_arc_for_test();

    let winners = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for i in 0..16 {
        let s = Arc::clone(&shared);
        let w = Arc::clone(&winners);
        handles.push(std::thread::spawn(move || {
            let reason = if i % 2 == 0 {
                "supervisor_lease_expired"
            } else {
                "supervisor_unwind_or_exit"
            };
            if crate::engine::session::signal_supervisor_expiry_with_reason(&s, reason) {
                w.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    for h in handles {
        h.join().expect("thread join");
    }

    assert_eq!(
        winners.load(Ordering::SeqCst),
        1,
        "exactly one caller must win the supervisor expiry publication race"
    );
    assert!(shared.commands.supervisor_expired.is_expired());
    assert!(shared.commands.panic_requested.load(Ordering::Acquire));
}

#[test]
fn worker_progress_state_transitions_from_starting_to_active_to_paused_to_draining() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15]);
    let mut options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.focus.require_focus = true;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind target owner");
    session.set_focus_hint(true);

    assert_eq!(
        session.worker_progress(),
        WorkerProgressState::Starting,
        "before arm state is Starting"
    );

    session.arm(0).expect("arm worker loop");

    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "transition to Active",
    );
    assert_eq!(
        session.poll_state().worker_progress,
        WorkerProgressState::Active
    );

    session.pause().expect("pause worker");
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Paused,
        "transition to Paused",
    );
    assert_eq!(
        session.poll_state().worker_progress,
        WorkerProgressState::Paused
    );

    session.resume().expect("resume worker");
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "transition back to Active",
    );
    assert_eq!(
        session.poll_state().worker_progress,
        WorkerProgressState::Active
    );

    session.quit().expect("quit worker");
    assert!(
        session.join(Duration::from_secs(3)).expect("worker join"),
        "worker join"
    );

    assert_eq!(
        session.worker_progress(),
        WorkerProgressState::Draining,
        "after termination worker progress is Draining"
    );
}

#[test]
fn paused_clean_journal_does_not_wake_without_interrupt_signals() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15]);
    let mut options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.focus.require_focus = true;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind target owner");
    session.set_focus_hint(true);

    session.arm(0).expect("arm worker loop");

    // Wait until preroll finishes and session enters playing:
    wait_until(
        Duration::from_secs(3),
        || session.snapshot().status == EnginePollStatus::Playing.as_str(),
        "worker playing",
    );

    session.pause().expect("pause");
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Paused,
        "worker paused",
    );

    // Sleep 100ms with zero signals:
    std::thread::sleep(Duration::from_millis(100));

    // Must still be safely and cleanly asleep in Paused:
    assert_eq!(session.worker_progress(), WorkerProgressState::Paused);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Paused.as_str());
    assert_eq!(snapshot.final_release_obligation_mask, 0);

    session.quit().expect("quit");
    assert!(session.join(Duration::from_secs(3)).expect("worker join"));
}

#[test]
fn paused_with_dirty_journal_cleans_up_obligations_before_idle() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));

    let sched = schedule_long(&[0x15, 0x16]);
    let mut options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.focus.require_focus = true;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind target owner");
    session.set_focus_hint(true);

    session.arm(0).expect("arm worker loop");

    // Wait until the initial Down packet has been dispatched:
    wait_until(
        Duration::from_secs(3),
        || send_calls.load(Ordering::Acquire) >= 1,
        "initial down sent",
    );

    // Pause while the two keys are physically held:
    session.pause().expect("pause with held keys");

    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Paused,
        "paused with cleaned obligations",
    );

    // Verification: all physical release obligations were cleanly drained:
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.final_release_obligation_mask, 0,
        "release obligations must be zero while paused"
    );
    assert_eq!(
        snapshot.active_count, 0,
        "active count must be zero while paused"
    );

    session.quit().expect("quit");
    assert!(session.join(Duration::from_secs(3)).expect("worker join"));
}

#[test]
fn resume_fails_closed_when_supervisor_lease_expired() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15]);
    let mut options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.focus.require_focus = true;
    options.wait.supervisor_lease_timeout_us = 50_000; // 50ms lease timeout

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind target owner");
    session.set_focus_hint(true);

    session.arm(0).expect("arm worker loop");

    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "worker active",
    );

    session.pause().expect("pause");
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Paused,
        "worker paused",
    );

    // Sleep 150ms without progress publication so the supervisor lease expires:
    std::thread::sleep(Duration::from_millis(150));

    // Attempt to resume (or join if already terminated by lease expiry):
    let _ = session.resume();

    assert!(
        session
            .join(Duration::from_secs(3))
            .expect("worker join on lease expiry"),
        "worker must terminate after resume with expired lease"
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    let err = snapshot.terminal_error.as_deref().unwrap_or_default();
    let trigger = snapshot
        .terminal_trigger_reason
        .as_deref()
        .unwrap_or_default();
    assert!(
        err.contains("supervisor lease expired") || trigger.contains("supervisor_lease_expired"),
        "expected lease expired error or trigger, got err={err}, trigger={trigger}"
    );
}

#[test]
fn resume_fails_closed_when_target_continuity_broken() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15]);
    let mut options = test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.focus.require_focus = true;
    options.wait.supervisor_lease_timeout_us = 5_000_000;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind target owner");
    session.set_focus_hint(true);

    session.arm(0).expect("arm worker loop");

    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "worker active",
    );

    session.pause().expect("pause");
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Paused,
        "worker paused",
    );

    // Break the process continuity of the target owner:
    assert!(
        session.set_window_process_identity_status_for_test(
            WindowIdentityContinuity::ProcessTerminated
        )
    );

    // Attempt to resume:
    session.resume().expect("resume signal");

    assert!(
        session
            .join(Duration::from_secs(3))
            .expect("worker join on broken continuity"),
        "worker must terminate after resume with broken process continuity"
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    let err = snapshot.terminal_error.as_deref().unwrap_or_default();
    assert!(
        err.contains("continuity broken"),
        "expected continuity broken error, got: {err}"
    );
}
