use super::support::*;
use super::*;
use crate::engine::snapshot::WorkerProgressState;
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

fn schedule_long(keys: &[u16]) -> sky_dispatch_core::model::OnsetPlan {
    let scan_codes = smallvec::SmallVec::from_slice(keys);
    sky_dispatch_core::plan::build_onset_plan(
        &[OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes,
        }],
        keys,
        10_000_000,
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

#[test]
fn signal_supervisor_expiry_prompt_without_locks_under_contention() {
    use crate::engine::session::signal_supervisor_expiry_with_reason;

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

    // Contention scenario: simulate another thread holding metrics.terminal_trigger_reason mutex
    let mutex_held = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mutex_held_clone = Arc::clone(&mutex_held);
    let shared_clone = Arc::clone(&shared);
    let blocker_thread = std::thread::spawn(move || {
        let _guard = shared_clone
            .publication
            .metrics
            .terminal_trigger_reason
            .lock();
        while mutex_held_clone.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(5));
        }
    });

    // Give blocker thread time to acquire mutex:
    std::thread::sleep(Duration::from_millis(20));

    // Calling signal_supervisor_expiry_with_reason must NOT block on the held mutex!
    let start = Instant::now();
    let signaled = signal_supervisor_expiry_with_reason(&shared, "supervisor_unwind_or_exit");
    let duration = start.elapsed();

    assert!(signaled, "first signal must win CAS");
    assert!(
        duration < Duration::from_millis(50),
        "signaling must not block on held metrics lock; took {:?}",
        duration
    );
    assert!(
        shared.commands.supervisor_expired.is_expired(),
        "supervisor lease must be latched expired immediately"
    );
    assert!(
        shared.commands.panic_requested.load(Ordering::Acquire),
        "panic release must be requested immediately"
    );

    // Release the blocker thread's lock:
    mutex_held.store(false, Ordering::Release);
    blocker_thread.join().expect("blocker thread join");

    // The snapshot must still accurately report the atomic reason code:
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("supervisor_unwind_or_exit")
    );
}

#[test]
fn validate_resume_prerequisites_checks_target_and_owner_proof_currency() {
    use crate::engine::shared::{SessionTarget, SupervisorLeaseState};
    use crate::engine::worker::{TargetStamp, validate_resume_prerequisites};
    use sky_dispatch_core::time::DurationTicks;
    use sky_dispatch_win32::clock::QpcTicks;
    use sky_dispatch_win32::focus::WindowIdentityAuthority;
    use sky_dispatch_win32::input::{InstrumentKeyProfileSpec, ReleaseOwnerProof, TrackedKeyState};

    let target = SessionTarget::new(TARGET_HWND, 1, true);
    let authority = Arc::new(WindowIdentityAuthority::synthetic_for_test(
        TARGET_HWND,
        TARGET_PID,
    ));
    target.bind_owner_identity_authority(TARGET_HWND, 1, TARGET_PID, authority.clone());

    let target_stamp = TargetStamp {
        hwnd: TARGET_HWND,
        generation: 1,
    };
    let lease = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let lease_timeout = DurationTicks::from_raw(1_000_000);
    let now = QpcTicks::from_raw(2_000);

    let profile = InstrumentKeyProfileSpec::canonical().keys;
    let valid_proof = ReleaseOwnerProof::new_with_foreground_policy(
        TARGET_HWND,
        1,
        Some(TARGET_PID),
        Some(authority.clone()),
        true,
    )
    .expect("valid proof");
    let valid_backend =
        TrackedKeyState::from_terminal_release_residual(profile, valid_proof, 0, 0, 0)
            .expect("valid backend");

    // 1. All match: OK
    let result = validate_resume_prerequisites(
        now,
        &target,
        target_stamp,
        &lease,
        lease_timeout,
        true,
        &valid_backend,
    );
    assert!(result.is_ok(), "prerequisites should succeed: {:?}", result);

    // 2. Stale target stamp generation
    let stale_stamp = TargetStamp {
        hwnd: TARGET_HWND,
        generation: 2,
    };
    let result = validate_resume_prerequisites(
        now,
        &target,
        stale_stamp,
        &lease,
        lease_timeout,
        true,
        &valid_backend,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("target invalidated")),
        "should reject stale stamp generation: {:?}",
        result
    );

    // 3. Stale release owner proof HWND mismatch
    let bad_hwnd_auth = Arc::new(WindowIdentityAuthority::synthetic_for_test(
        0x9999, TARGET_PID,
    ));
    let bad_hwnd_proof = ReleaseOwnerProof::new_with_foreground_policy(
        0x9999,
        1,
        Some(TARGET_PID),
        Some(bad_hwnd_auth),
        true,
    )
    .expect("bad hwnd proof");
    let bad_hwnd_backend =
        TrackedKeyState::from_terminal_release_residual(profile, bad_hwnd_proof, 0, 0, 0)
            .expect("bad hwnd backend");
    let result = validate_resume_prerequisites(
        now,
        &target,
        target_stamp,
        &lease,
        lease_timeout,
        true,
        &bad_hwnd_backend,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("target mismatch on resume")),
        "should reject proof HWND mismatch: {:?}",
        result
    );

    // 4. Stale release owner proof generation mismatch
    let bad_gen_proof = ReleaseOwnerProof::new_with_foreground_policy(
        TARGET_HWND,
        2,
        Some(TARGET_PID),
        Some(authority.clone()),
        true,
    )
    .expect("bad gen proof");
    let bad_gen_backend =
        TrackedKeyState::from_terminal_release_residual(profile, bad_gen_proof, 0, 0, 0)
            .expect("bad gen backend");
    let result = validate_resume_prerequisites(
        now,
        &target,
        target_stamp,
        &lease,
        lease_timeout,
        true,
        &bad_gen_backend,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("target mismatch on resume")),
        "should reject proof generation mismatch: {:?}",
        result
    );

    // 5. Stale owner authority PID mismatch
    let wrong_pid_auth = Arc::new(WindowIdentityAuthority::synthetic_for_test(
        TARGET_HWND,
        0x9999,
    ));
    let bad_pid_proof = ReleaseOwnerProof::new_with_foreground_policy(
        TARGET_HWND,
        1,
        Some(0x9999),
        Some(wrong_pid_auth),
        true,
    )
    .expect("bad pid proof");
    let bad_pid_backend =
        TrackedKeyState::from_terminal_release_residual(profile, bad_pid_proof, 0, 0, 0)
            .expect("bad pid backend");
    let result = validate_resume_prerequisites(
        now,
        &target,
        target_stamp,
        &lease,
        lease_timeout,
        true,
        &bad_pid_backend,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("PID mismatch on resume")),
        "should reject PID mismatch: {:?}",
        result
    );

    // 6. Lease expired while paused
    lease.latch_expired();
    let result = validate_resume_prerequisites(
        now,
        &target,
        target_stamp,
        &lease,
        lease_timeout,
        true,
        &valid_backend,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("supervisor lease expired while paused")),
        "should reject expired lease: {:?}",
        result
    );

    // 7. Different authority Arc with same HWND, PID, and generation
    let different_auth = Arc::new(WindowIdentityAuthority::synthetic_for_test(
        TARGET_HWND,
        TARGET_PID,
    ));
    let different_auth_proof = ReleaseOwnerProof::new_with_foreground_policy(
        TARGET_HWND,
        1,
        Some(TARGET_PID),
        Some(different_auth),
        true,
    )
    .expect("different auth proof");
    let different_auth_backend =
        TrackedKeyState::from_terminal_release_residual(profile, different_auth_proof, 0, 0, 0)
            .expect("different auth backend");
    let unexpired_lease = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let result = validate_resume_prerequisites(
        now,
        &target,
        target_stamp,
        &unexpired_lease,
        lease_timeout,
        true,
        &different_auth_backend,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("session release owner proof is not current on resume")),
        "should reject different authority Arc even with identical HWND/PID/generation: {:?}",
        result
    );
}

#[test]
fn active_worker_contested_reason_lock_preserves_supervisor_unwind_reason_after_finalization() {
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

    // Wait for the active worker to start playing notes
    wait_until(
        Duration::from_secs(3),
        || send_calls.load(Ordering::Acquire) >= 1,
        "first down send",
    );

    let shared = session.shared_arc_for_test();

    // Contention scenario: simulate another thread holding metrics.terminal_trigger_reason mutex
    let mutex_held = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mutex_held_clone = Arc::clone(&mutex_held);
    let shared_clone = Arc::clone(&shared);
    let blocker_thread = std::thread::spawn(move || {
        let _guard = shared_clone
            .publication
            .metrics
            .terminal_trigger_reason
            .lock();
        while mutex_held_clone.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(5));
        }
    });

    // Wait until blocker thread acquires mutex:
    std::thread::sleep(Duration::from_millis(20));

    // Drop supervisor guard (simulate supervisor unwind while mutex is contested):
    // This MUST succeed without blocking, record reason code atomically, and interrupt worker:
    let start = Instant::now();
    drop(guard);
    let duration = start.elapsed();

    assert!(
        duration < Duration::from_millis(50),
        "supervisor guard drop must not block on held metrics lock; took {:?}",
        duration
    );

    // Release the blocker thread's lock so worker finalization can proceed:
    mutex_held.store(false, Ordering::Release);
    blocker_thread.join().expect("blocker thread join");

    // Join the worker thread to wait for completed finalization:
    assert!(
        session
            .join(Duration::from_secs(3))
            .expect("join after guard drop"),
        "worker must terminate after supervisor guard drop"
    );

    // Now verify snapshot: winning atomic reason MUST be observable, NOT masked by generic panic:
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("supervisor_unwind_or_exit"),
        "winning specific supervisor unwind reason must not be masked by generic trigger"
    );
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(snapshot.active_count, 0);
}

#[test]
fn active_worker_preserves_earlier_terminal_cause_over_subsequent_supervisor_expiry() {
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
    session.arm(0).expect("arm worker loop");

    wait_until(
        Duration::from_secs(3),
        || send_calls.load(Ordering::Acquire) >= 1,
        "first down send",
    );

    // Intentional quit occurs first:
    let _ = session.quit();
    assert!(
        session
            .join(Duration::from_secs(3))
            .expect("join after quit"),
        "worker must terminate after quit"
    );

    // Subsequent dropped guard or expiry signal must NOT overwrite the earlier "quit" reason:
    drop(guard);

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("quit"),
        "genuinely earlier 'quit' trigger reason must be preserved"
    );
}

#[test]
fn overlapping_focus_and_manual_pause_focus_restored_first_remains_paused() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15, 0x16]);
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

    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "transition to Active",
    );

    // 1. Enter manual pause:
    session.pause().expect("manual pause");
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Paused,
        "transition to Paused",
    );

    // 2. Also lose focus while paused:
    session.set_focus_hint(false);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(session.worker_progress(), WorkerProgressState::Paused);

    // 3. Restore focus FIRST:
    session.set_focus_hint(true);
    // Worker loop processes focus restoration, but manual pause is still active!
    // Worker progress MUST remain Paused, NOT Active!
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        session.worker_progress(),
        WorkerProgressState::Paused,
        "worker_progress must remain Paused when focus restores while manual pause is still active"
    );
    assert!(session.snapshot().is_paused);

    // 4. Now resume manual pause SECOND:
    session.resume().expect("manual resume");
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "transition back to Active only when both pauses are cleared",
    );
    assert_eq!(session.worker_progress(), WorkerProgressState::Active);
    assert!(!session.snapshot().is_paused);

    session.quit().expect("quit worker");
    assert!(session.join(Duration::from_secs(3)).expect("worker join"));
}

#[test]
fn overlapping_focus_and_manual_pause_manual_resumed_first_remains_paused() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15, 0x16]);
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

    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "transition to Active",
    );

    // 1. Lose focus:
    session.set_focus_hint(false);
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Paused,
        "transition to Paused on focus lost",
    );

    // 2. Also enter manual pause:
    session.pause().expect("manual pause");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(session.worker_progress(), WorkerProgressState::Paused);

    // 3. Resume manual pause FIRST:
    session.resume().expect("manual resume");
    // Worker loop processes manual resume, but focus is still lost!
    // Worker progress MUST remain Paused, NOT Active!
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        session.worker_progress(),
        WorkerProgressState::Paused,
        "worker_progress must remain Paused when manual resumes while focus is still lost"
    );
    assert!(session.snapshot().is_paused);

    // 4. Now restore focus SECOND:
    session.set_focus_hint(true);
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "transition back to Active only when both pauses are cleared",
    );
    assert_eq!(session.worker_progress(), WorkerProgressState::Active);
    assert!(!session.snapshot().is_paused);

    session.quit().expect("quit worker");
    assert!(session.join(Duration::from_secs(3)).expect("worker join"));
}

#[test]
fn validate_resume_prerequisites_samples_continuity_once_and_rejects_broken() {
    use crate::engine::shared::{SessionTarget, SupervisorLeaseState};
    use crate::engine::worker::{TargetStamp, validate_resume_prerequisites};
    use sky_dispatch_core::time::DurationTicks;
    use sky_dispatch_win32::clock::QpcTicks;
    use sky_dispatch_win32::focus::WindowIdentityAuthority;
    use sky_dispatch_win32::input::{InstrumentKeyProfileSpec, ReleaseOwnerProof, TrackedKeyState};

    let target = SessionTarget::new(TARGET_HWND, 1, true);
    let authority = Arc::new(WindowIdentityAuthority::synthetic_for_test(
        TARGET_HWND,
        TARGET_PID,
    ));
    target.bind_owner_identity_authority(TARGET_HWND, 1, TARGET_PID, authority.clone());

    let target_stamp = TargetStamp {
        hwnd: TARGET_HWND,
        generation: 1,
    };
    let lease = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let lease_timeout = DurationTicks::from_raw(1_000_000);
    let now = QpcTicks::from_raw(2_000);

    let profile = InstrumentKeyProfileSpec::canonical().keys;
    let valid_proof = ReleaseOwnerProof::new_with_foreground_policy(
        TARGET_HWND,
        1,
        Some(TARGET_PID),
        Some(authority.clone()),
        true,
    )
    .expect("valid proof");
    let valid_backend =
        TrackedKeyState::from_terminal_release_residual(profile, valid_proof, 0, 0, 0)
            .expect("valid backend");

    // Break continuity on the retained authority:
    authority.set_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
    );

    let result = validate_resume_prerequisites(
        now,
        &target,
        target_stamp,
        &lease,
        lease_timeout,
        true,
        &valid_backend,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("continuity broken on resume")),
        "should reject broken continuity: {:?}",
        result
    );
}

#[test]
fn fresh_lease_check_at_resume_decision_fails_closed_when_expired_during_cleanup() {
    let _lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let sched = schedule_long(&[0x15, 0x16]);
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
    options.wait.supervisor_lease_timeout_us = 5_000_000; // 5s timeout so worker stays alive while paused

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

    let _guard = session.supervisor_lifetime_guard();
    session.arm(0).expect("arm worker loop");

    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Active,
        "transition to Active",
    );

    session.pause().expect("manual pause");
    wait_until(
        Duration::from_secs(3),
        || session.worker_progress() == WorkerProgressState::Paused,
        "transition to Paused",
    );

    // Latch lease expired right before resume decision:
    session
        .shared_arc_for_test()
        .commands
        .supervisor_expired
        .latch_expired();

    // Resume worker:
    session.resume().expect("manual resume");

    // The fresh lease check with resumed_ticks immediately before exit_pause
    // must catch the expired lease and terminate the worker fail-closed!
    assert!(
        session.join(Duration::from_secs(3)).expect("worker join"),
        "worker must terminate on expired lease"
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
