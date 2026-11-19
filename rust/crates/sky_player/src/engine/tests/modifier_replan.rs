use super::super::test_support::MockSendInputCapture;
use super::*;
use sky_dispatch_core::model::RuntimeSchedule;
use sky_dispatch_win32::input::PhysicalPacket;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const TARGET_HWND: isize = 123;
const TARGET_PID: u32 = 77;

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

fn mock_session(
    schedule: RuntimeSchedule,
    held: &Arc<AtomicBool>,
    admission_checks: &Arc<AtomicUsize>,
    captures: &Arc<Mutex<Vec<MockSendInputCapture>>>,
    send_calls: &Arc<AtomicU64>,
) -> NativeDispatchSession {
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(send_calls));
    fault_script.send_input_capture = Some(Arc::clone(captures));
    let mut options = support::test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.focus.require_focus = true;
    let held = Arc::clone(held);
    let checks = Arc::clone(admission_checks);
    options.modifier_key_state_query_for_test = Some(Arc::new(move |virtual_key| {
        if virtual_key == 0x5b {
            checks.fetch_add(1, Ordering::AcqRel);
        }
        if virtual_key == 0x11 && held.load(Ordering::Acquire) {
            i16::MIN
        } else {
            0
        }
    }));

    let session = NativeDispatchSession::new(options).expect("mock native session admission");
    session.bind_initial_target_for_test(TARGET_HWND);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET_HWND,
                TARGET_PID,
            ),
            TARGET_PID,
        )
        .expect("bind synthetic owner before Arm");
    session.set_focus_hint(true);
    session
}

fn schedule(actions: &[KeyActionInput], scan_codes: &[u16]) -> RuntimeSchedule {
    sky_dispatch_core::compile::compile_runtime_intents(actions, scan_codes)
        .expect("valid modifier replan fixture")
}

fn captured_packets(captures: &Mutex<Vec<MockSendInputCapture>>) -> Vec<PhysicalPacket> {
    captures
        .lock()
        .expect("mock packet captures")
        .iter()
        .filter_map(|capture| match capture {
            MockSendInputCapture::Packet { packet, .. } => Some(*packet),
            MockSendInputCapture::SingleEmitter { .. } => None,
        })
        .collect()
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool, label: &str) {
    let deadline = Instant::now() + timeout;
    while !condition() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(condition(), "timed out waiting for {label}");
}

fn held_modifier_mixed_schedule() -> RuntimeSchedule {
    schedule(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "modifier-replan-first-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "modifier-replan-independent-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x16, 0x17],
                reason: "modifier-replan-two-key-chord".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_500_000,
                scan_codes: smallvec::smallvec![0x16, 0x17],
                reason: "modifier-replan-chord-up".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
}

#[test]
fn production_worker_modifier_replan_backoff_preserves_due_up_and_fresh_chord_gate() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let held = Arc::new(AtomicBool::new(false));
    let admission_checks = Arc::new(AtomicUsize::new(0));
    let captures = Arc::new(Mutex::new(Vec::new()));
    let send_calls = Arc::new(AtomicU64::new(0));
    let session = mock_session(
        held_modifier_mixed_schedule(),
        &held,
        &admission_checks,
        &captures,
        &send_calls,
    );
    session.arm(0).expect("arm production worker loop");

    wait_until(
        Duration::from_secs(3),
        || !captured_packets(&captures).is_empty(),
        "first Down packet",
    );
    assert_eq!(
        captured_packets(&captures)[0],
        PhysicalPacket {
            up_mask: 0,
            down_mask: 0b001,
        }
    );
    held.store(true, Ordering::Release);

    wait_until(
        Duration::from_secs(3),
        || captured_packets(&captures).len() >= 2,
        "independent Up from a mixed Up+Down decision",
    );
    assert_eq!(
        captured_packets(&captures)[1],
        PhysicalPacket {
            up_mask: 0b001,
            down_mask: 0,
        },
        "modifier-held Down must replan to the independent Up-only packet",
    );

    let checks_before_hold = admission_checks.load(Ordering::Acquire);
    std::thread::sleep(Duration::from_millis(120));
    let held_admission_iterations = admission_checks
        .load(Ordering::Acquire)
        .saturating_sub(checks_before_hold);
    println!("modifier-replan held_ms=120 admission_iterations={held_admission_iterations}");
    assert_eq!(
        captured_packets(&captures).len(),
        2,
        "no Down may be sent while Ctrl remains held",
    );

    let pause_started = Instant::now();
    session
        .pause()
        .expect("pause while modifier backoff is active");
    wait_until(
        Duration::from_secs(2),
        || session.snapshot_lite().is_paused,
        "interruptible pause acknowledgement",
    );
    assert!(
        pause_started.elapsed() < Duration::from_secs(2),
        "pause must interrupt the modifier backoff promptly"
    );
    assert_eq!(captured_packets(&captures).len(), 2);
    session
        .resume()
        .expect("resume after modifier-backoff pause");

    // Lose focus while the modifier remains held. The worker must pause; after
    // same-owner focus recovery and modifier release it re-enters final
    // foreground, owner, and modifier admission before the chord Down.
    install_foreground(456, 88);
    session.set_focus_hint(false);
    wait_until(
        Duration::from_secs(2),
        || session.snapshot_lite().is_paused,
        "focus-loss pause during modifier hold",
    );
    let checks_at_modifier_release = admission_checks.load(Ordering::Acquire);
    held.store(false, Ordering::Release);
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(
        captured_packets(&captures).len(),
        2,
        "modifier release cannot bypass the foreground gate",
    );
    install_foreground(TARGET_HWND, TARGET_PID);
    session.set_focus_hint(true);

    wait_until(
        Duration::from_secs(3),
        || {
            captured_packets(&captures)
                .iter()
                .any(|packet| packet.down_mask == 0b110)
        },
        "freshly admitted two-key chord Down after modifier release",
    );
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("worker completion")
    );

    let packets = captured_packets(&captures);
    assert_eq!(
        packets,
        [
            PhysicalPacket {
                up_mask: 0,
                down_mask: 0b001,
            },
            PhysicalPacket {
                up_mask: 0b001,
                down_mask: 0,
            },
            PhysicalPacket {
                up_mask: 0,
                down_mask: 0b110,
            },
            PhysicalPacket {
                up_mask: 0b110,
                down_mask: 0,
            },
        ],
        "there must be no duplicate Down, chord split, or lost Up",
    );
    assert_eq!(send_calls.load(Ordering::Acquire), 4);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert_eq!(snapshot.failed_release_count, 0);
    assert!(
        admission_checks.load(Ordering::Acquire) > checks_at_modifier_release,
        "the released modifier and restored foreground must pass a fresh final Down gate"
    );
    assert!(
        held_admission_iterations >= 10,
        "the sustained hold must exercise repeated production admission checks"
    );
    assert!(
        held_admission_iterations <= 256,
        "modifier hold must use bounded interruptible backoff; observed {held_admission_iterations} admission iterations in 120 ms"
    );
}

#[test]
fn owner_invalidation_interrupts_modifier_backoff_and_retains_origin_obligation() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let held = Arc::new(AtomicBool::new(false));
    let admission_checks = Arc::new(AtomicUsize::new(0));
    let captures = Arc::new(Mutex::new(Vec::new()));
    let send_calls = Arc::new(AtomicU64::new(0));
    let blocked_schedule = schedule(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owner-replan-first-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 250_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "owner-replan-blocked-down".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 1_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owner-replan-origin-up".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_100_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "owner-replan-unattempted-up".into(),
            },
        ],
        &[0x15, 0x16],
    );
    let session = mock_session(
        blocked_schedule,
        &held,
        &admission_checks,
        &captures,
        &send_calls,
    );
    session.arm(0).expect("arm production worker loop");
    wait_until(
        Duration::from_secs(3),
        || !captured_packets(&captures).is_empty(),
        "owned first Down",
    );
    let prior_checks = admission_checks.load(Ordering::Acquire);
    held.store(true, Ordering::Release);
    wait_until(
        Duration::from_secs(3),
        || admission_checks.load(Ordering::Acquire) > prior_checks,
        "modifier-held final admission",
    );
    std::thread::sleep(Duration::from_millis(20));

    let invalidated_at = Instant::now();
    session.invalidate_window_identity_authority_for_test();
    assert!(
        session
            .join(Duration::from_millis(750))
            .expect("owner invalidation must complete the worker"),
        "owner invalidation must interrupt bounded modifier backoff promptly"
    );
    assert!(invalidated_at.elapsed() < Duration::from_millis(750));
    assert_eq!(captured_packets(&captures).len(), 1);
    assert_eq!(send_calls.load(Ordering::Acquire), 1);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert!(snapshot.terminal_error.is_some());
    assert_eq!(
        snapshot.final_release_obligation_mask, 1,
        "owner loss must preserve the origin release obligation"
    );
    assert_eq!(snapshot.release_attempts_total, 0);
    assert_eq!(snapshot.active_count, 1);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert_eq!(
        snapshot.failed_release_count, 1,
        "owner loss withholds terminal Up and quarantines the origin obligation"
    );
}

#[test]
fn terminal_controls_interrupt_modifier_replan_without_attempting_blocked_down() {
    for (control, expected_status, expected_trigger) in [
        ("quit", EnginePollStatus::Quit, "quit"),
        ("skip", EnginePollStatus::Skipped, "skipped"),
        ("panic", EnginePollStatus::Error, "panic_release_requested"),
    ] {
        let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
        let _foreground_reset = ForegroundOverrideReset;
        install_foreground(TARGET_HWND, TARGET_PID);
        let held = Arc::new(AtomicBool::new(false));
        let admission_checks = Arc::new(AtomicUsize::new(0));
        let captures = Arc::new(Mutex::new(Vec::new()));
        let send_calls = Arc::new(AtomicU64::new(0));
        let schedule = schedule(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "control-replan-first-down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Down,
                    scheduled_us: 200_000,
                    scan_codes: smallvec::smallvec![0x16],
                    reason: "control-replan-blocked-down".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "control-replan-origin-up".into(),
                },
                KeyActionInput {
                    source_action_index: 3,
                    kind: ActionKind::Up,
                    scheduled_us: 1_100_000,
                    scan_codes: smallvec::smallvec![0x16],
                    reason: "control-replan-unattempted-up".into(),
                },
            ],
            &[0x15, 0x16],
        );
        let session = mock_session(schedule, &held, &admission_checks, &captures, &send_calls);
        session.arm(0).expect("arm production worker loop");
        wait_until(
            Duration::from_secs(3),
            || !captured_packets(&captures).is_empty(),
            "first owned Down",
        );
        let prior_checks = admission_checks.load(Ordering::Acquire);
        held.store(true, Ordering::Release);
        wait_until(
            Duration::from_secs(3),
            || admission_checks.load(Ordering::Acquire) > prior_checks,
            "modifier-held worker replan",
        );
        std::thread::sleep(Duration::from_millis(20));

        match control {
            "quit" => session.quit().expect("Stop/quit request"),
            "skip" => session.skip().expect("skip request"),
            "panic" => session.panic_release().expect("panic release request"),
            _ => unreachable!(),
        }
        assert!(
            session
                .join(Duration::from_secs(1))
                .expect("bounded control join"),
            "{control} must interrupt the modifier-specific wait"
        );
        assert_eq!(
            captured_packets(&captures).len(),
            1,
            "{control} must not allow the modifier-blocked Down"
        );
        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, expected_status.as_str(), "{control}");
        assert_eq!(
            snapshot.terminal_trigger_reason.as_deref(),
            Some(expected_trigger),
            "{control}"
        );
        assert_eq!(snapshot.final_release_obligation_mask, 0, "{control}");
        assert_eq!(snapshot.possibly_active_count, 0, "{control}");
    }
}

#[test]
fn release_owner_proof_is_retained_once_per_session_not_per_down_slot() {
    use crate::engine::test_support::ProductionDispatchTestHarness;

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    for key_count in [1, 5, 15] {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(key_count);
        let epoch = harness.resources.playback.epoch;
        harness.configure_focus_owner_for_test(1, 177);
        harness.configure_mock_packet_sender_for_test(&[]);
        harness
            .attach_executor_for_test()
            .expect("executor invariant");
        harness.set_mock_sender_start_qpc_for_test(epoch);
        let candidate = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage chord")
            .expect("one due chord");

        sky_dispatch_win32::input::reset_release_owner_proof_arc_clone_count_for_test();
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            crate::engine::worker::StagedDispatchOutcome::Dispatched
        );
        let proof_arc_clones =
            sky_dispatch_win32::input::release_owner_proof_arc_clone_count_for_test();
        println!(
            "owner-proof-clones chord_slots={key_count} release_identity_arc_clones={proof_arc_clones}"
        );
        assert_eq!(
            proof_arc_clones, 0,
            "dispatching additional Down slots does not clone session owner authority"
        );
        let expected_mask = (1_u16 << key_count) - 1;
        assert_eq!(harness.release_obligation_mask_for_test(), expected_mask);
        let _ = harness.finalize_for_test();
    }
}

#[test]
fn modifier_inconclusive_query_fails_closed_without_sends() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let captures = Arc::new(Mutex::new(Vec::new()));
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    fault_script.send_input_capture = Some(Arc::clone(&captures));

    let sched = schedule(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "inconclusive-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "inconclusive-up".into(),
            },
        ],
        &[0x15],
    );

    let mut options = support::test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.focus.require_focus = true;
    options.modifier_key_state_query_for_test = Some(Arc::new(|_vk| {
        sky_dispatch_win32::input::MODIFIER_QUERY_INCONCLUSIVE
    }));

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
        .expect("bind synthetic owner");
    session.set_focus_hint(true);

    session.arm(0).expect("arm worker loop");

    assert!(
        session
            .join(Duration::from_secs(2))
            .expect("worker completion on error"),
        "inconclusive modifier must terminate the worker promptly"
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    let err = snapshot.terminal_error.as_deref().unwrap_or_default();
    assert!(
        err.contains("modifier_state_inconclusive"),
        "expected terminal error containing 'modifier_state_inconclusive', got: {err}"
    );
    assert_eq!(
        captured_packets(&captures).len(),
        0,
        "no packets may be sent when modifier query is inconclusive"
    );
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(snapshot.active_count, 0);
}

#[test]
fn modifier_hold_exceeding_bounded_deadline_fails_closed() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let held = Arc::new(AtomicBool::new(true));
    let admission_checks = Arc::new(AtomicUsize::new(0));
    let captures = Arc::new(Mutex::new(Vec::new()));
    let send_calls = Arc::new(AtomicU64::new(0));

    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    fault_script.send_input_capture = Some(Arc::clone(&captures));

    let sched = schedule(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "held-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "held-up".into(),
            },
        ],
        &[0x15],
    );

    let mut options = support::test_session_options(
        sched,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.focus.require_focus = true;
    options.wait.modifier_hold_timeout_us = 50_000;

    let checks = Arc::clone(&admission_checks);
    let held_flag = Arc::clone(&held);
    options.modifier_key_state_query_for_test = Some(Arc::new(move |virtual_key| {
        if virtual_key == 0x5b {
            checks.fetch_add(1, Ordering::AcqRel);
        }
        if virtual_key == 0x11 && held_flag.load(Ordering::Acquire) {
            i16::MIN
        } else {
            0
        }
    }));

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
        .expect("bind synthetic owner");
    session.set_focus_hint(true);

    let start = Instant::now();
    session.arm(0).expect("arm worker loop");

    assert!(
        session
            .join(Duration::from_secs(2))
            .expect("worker completion on timeout"),
        "exceeded modifier hold deadline must terminate worker"
    );

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "worker must terminate near bounded deadline (elapsed: {elapsed:?})"
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    let err = snapshot.terminal_error.as_deref().unwrap_or_default();
    assert!(
        err.contains("modifier held exceeded bounded wait deadline"),
        "expected error containing 'modifier held exceeded bounded wait deadline', got: {err}"
    );
    assert_eq!(captured_packets(&captures).len(), 0, "no packets sent");
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert!(
        admission_checks.load(Ordering::Acquire) > 0,
        "should have checked modifier admission"
    );
}

#[test]
fn overlapping_manual_pause_and_focus_loss_resumes_cleanly() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let held = Arc::new(AtomicBool::new(false));
    let admission_checks = Arc::new(AtomicUsize::new(0));
    let captures = Arc::new(Mutex::new(Vec::new()));
    let send_calls = Arc::new(AtomicU64::new(0));

    let sched = schedule(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "overlap-first-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "overlap-first-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "overlap-second-down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "overlap-second-up".into(),
            },
        ],
        &[0x15, 0x16],
    );

    let session = mock_session(sched, &held, &admission_checks, &captures, &send_calls);
    session.arm(0).expect("arm worker loop");

    wait_until(
        Duration::from_secs(3),
        || !captured_packets(&captures).is_empty(),
        "first Down dispatched",
    );

    // 1. Lose focus
    install_foreground(999, 999);
    session.set_focus_hint(false);

    wait_until(
        Duration::from_secs(2),
        || session.snapshot_lite().is_paused,
        "worker paused on focus loss",
    );

    // 2. Pause while focus loss is active (manual pause overlapping focus loss)
    session.pause().expect("manual pause during focus loss");

    // 3. Restore focus while manual pause is still active
    install_foreground(TARGET_HWND, TARGET_PID);
    session.set_focus_hint(true);
    std::thread::sleep(Duration::from_millis(50));

    // Session must remain paused because manual pause is still active!
    assert!(
        session.snapshot_lite().is_paused,
        "session must remain paused due to manual pause"
    );

    // 4. Resume manual pause
    session.resume().expect("manual resume");

    // Playback should now resume and finish cleanly
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("clean worker completion after resume"),
        "session must finish playback after overlapping pause/focus resolution"
    );

    let all_captures = captures.lock().unwrap().clone();
    assert_eq!(all_captures.len(), 4, "all 4 actions dispatched");
    // Verify key 21 was released during pause via safety emission
    assert!(
        all_captures.iter().any(|c| matches!(
            c,
            MockSendInputCapture::SingleEmitter {
                key_up: true,
                scan_codes
            } if scan_codes.as_slice() == [21]
        )),
        "first key must be safely released on pause"
    );
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Finished.as_str());
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.failed_release_count, 0);
}

#[test]
fn simultaneous_stop_and_skip_during_modifier_wait() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    install_foreground(TARGET_HWND, TARGET_PID);

    let held = Arc::new(AtomicBool::new(false));
    let admission_checks = Arc::new(AtomicUsize::new(0));
    let captures = Arc::new(Mutex::new(Vec::new()));
    let send_calls = Arc::new(AtomicU64::new(0));

    let sched = schedule(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "simul-first-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "simul-blocked-down".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 1_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "simul-origin-up".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_100_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "simul-unattempted-up".into(),
            },
        ],
        &[0x15, 0x16],
    );

    let session = mock_session(sched, &held, &admission_checks, &captures, &send_calls);
    session.arm(0).expect("arm worker loop");

    wait_until(
        Duration::from_secs(3),
        || !captured_packets(&captures).is_empty(),
        "first Down dispatched",
    );

    let prior_checks = admission_checks.load(Ordering::Acquire);
    held.store(true, Ordering::Release);
    wait_until(
        Duration::from_secs(3),
        || admission_checks.load(Ordering::Acquire) > prior_checks,
        "modifier-held wait entered",
    );

    // Call quit and skip in close succession
    let _ = session.skip();
    let _ = session.quit();

    assert!(
        session.join(Duration::from_secs(2)).expect("worker join"),
        "simultaneous stop/skip must interrupt modifier wait cleanly"
    );

    let snapshot = session.snapshot();
    assert!(
        snapshot.status == EnginePollStatus::Skipped.as_str()
            || snapshot.status == EnginePollStatus::Quit.as_str(),
        "status must be skipped or quit, got: {}",
        snapshot.status
    );
    assert_eq!(
        captured_packets(&captures).len(),
        1,
        "blocked Down must not be sent"
    );
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(snapshot.active_count, 0);
}
