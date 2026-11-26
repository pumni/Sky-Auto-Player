#![cfg(any(test, feature = "test-support"))]

use super::*;

struct ForegroundTestReset;

impl Drop for ForegroundTestReset {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
    }
}

fn dispatch_owned_down_for_cleanup(harness: &mut ProductionDispatchTestHarness) {
    let epoch = harness.qpc_now_for_test().expect("cleanup test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor cleanup test clock");
    harness.configure_focus_owner_for_test(1, 77);
    harness.configure_mock_packet_sender_for_test(&[]);
    harness
        .attach_executor_for_test()
        .expect("attach cleanup executor");
    let down = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage owned Down")
        .expect("owned Down candidate");
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(down, false),
        crate::engine::worker::StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
}

fn wait_until_qpc(clock: QpcClock, deadline: QpcTicks) {
    loop {
        let now = clock.now().expect("production scheduler test QPC");
        if now >= deadline {
            return;
        }
        let remaining = deadline
            .checked_duration_since(now)
            .expect("scheduler wait ordering");
        let sleep_us = clock
            .duration_to_us(remaining)
            .expect("scheduler wait conversion")
            .max(1);
        std::thread::sleep(std::time::Duration::from_micros(sleep_us));
    }
}

fn dispatch_live_scheduler_next(
    harness: &mut ProductionDispatchTestHarness,
    now: QpcTicks,
    epoch: QpcTicks,
) -> (u16, u16, crate::engine::worker::StagedDispatchOutcome) {
    use sky_dispatch_core::executor::DispatchDecision;

    let (up_mask, down_chord, timing_window): (
        u16,
        Option<sky_dispatch_core::executor::PreparedChord>,
        sky_dispatch_core::executor::DispatchTimingWindow,
    ) = match harness
        .resources
        .scheduler
        .evaluate_at(now, epoch)
        .expect("live scheduler evaluation")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => (up_mask, down_chord, timing_window),
        decision => panic!("expected production scheduler Send decision, got {decision:?}"),
    };
    let down_mask = down_chord.map_or(0, |chord| chord.key_mask);
    let candidate = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage live scheduler candidate")
        .expect("live scheduler Send candidate");
    let sender_start = harness
        .resources
        .clock
        .now()
        .expect("live scheduler sender start QPC");
    harness.set_mock_sender_start_qpc_for_test(sender_start);
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    (up_mask, down_mask, outcome)
}

#[test]
fn lifecycle_fixture_admission_and_config_metadata_match_native_contract() {
    let mut fixtures = vec![
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(10_000_000, false)
            .expect("lifecycle fixture admission"),
        ProductionDispatchTestHarness::new_hol_sequence_for_test(10_000_000)
            .expect("HOL fixture admission"),
        ProductionDispatchTestHarness::new_mixed(),
        ProductionDispatchTestHarness::new_down_chord(15),
    ];

    for fixture in &mut fixtures {
        let config = fixture
            .dispatch_timing_config_for_test()
            .expect("dispatch timing metadata");
        assert!(config.native_admission_passed);
        assert_eq!(config.frame_us, DISPATCH_FRAME_US);
        assert_eq!(config.frame_base_hold_us, DISPATCH_FRAME_US);
        assert_eq!(config.timing_margin_us, DISPATCH_MARGIN_US);
        assert_eq!(config.effective_hold_us, DISPATCH_EFFECTIVE_HOLD_US);
        assert_eq!(
            config.frame_interval_ticks,
            fixture
                .resources
                .clock
                .duration_from_us(DISPATCH_FRAME_US)
                .unwrap()
                .as_u64()
        );
        assert_eq!(
            config.effective_hold_ticks,
            fixture
                .resources
                .clock
                .duration_from_us(DISPATCH_EFFECTIVE_HOLD_US)
                .unwrap()
                .as_u64()
        );
    }
}

#[test]
fn test_support_cleanup_accounting_uses_the_live_scheduler() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let scheduler_before = harness.resources.scheduler.accounting();
    assert_eq!(harness.generation_accounting_for_test(), scheduler_before);

    harness.runtime.terminal_trigger = Some(super::super::super::worker::TerminalTrigger::Quit);
    let mut observation = super::super::super::worker::FinalizeTestObservation::default();
    let _ = harness.finalize_worker_for_test(false, &mut observation);

    assert_eq!(
        observation.pre_cancel_generation_accounting, scheduler_before,
        "terminal pre-cancel audit must read the scheduler"
    );
    assert!(
        observation.generation_accounting.cancelled > scheduler_before.cancelled,
        "terminal cleanup must cancel the scheduler's remaining generations"
    );
}

#[test]
fn test_support_resumable_cleanup_releases_and_cancels_live_scheduler_state() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_mock_cleanup_emitter_for_test();
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage scheduler Down")
        .expect("scheduler Down candidate");
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let completed = clock.now().expect("mock send completion");
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(completed),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        crate::engine::worker::StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(harness.resources.scheduler.accounting().active, 1);

    harness
        .apply_scheduler_resumable_lifecycle_transition_for_test()
        .expect("verified scheduler suspension cleanup");

    let accounting = harness.resources.scheduler.accounting();
    assert_eq!(accounting.activated, 1);
    assert_eq!(accounting.active, 0);
    assert_eq!(
        accounting.released, 0,
        "cleanup cancels; it does not release"
    );
    assert_eq!(accounting.cancelled, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    harness
        .resources
        .scheduler
        .check_invariants()
        .expect("scheduler cleanup invariants");
}

#[test]
fn terminal_cleanup_revalidates_owner_after_first_incomplete_attempt() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    dispatch_owned_down_for_cleanup(&mut harness);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    let target = Arc::clone(&harness.target);
    let probes = Arc::new(AtomicUsize::new(0));
    let probe_count = Arc::clone(&probes);
    harness.resources.backend.set_probe(move |_, _| {
        if probe_count.fetch_add(1, Ordering::AcqRel) == 0 {
            target.invalidate_owner_identity();
        }
        InstrumentPhysicalState::Held(vec![0x15].into())
    });

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let _outcome = harness.finalize_worker_for_test(false, &mut observation);

    assert_eq!(cleanup.lock().expect("terminal cleanup capture").len(), 1);
    assert_eq!(observation.terminal_release_attempts, 1);
    assert_eq!(observation.total_release_attempts, 1);
    assert_eq!(observation.release_obligation_mask, 1);
    assert_eq!(
        observation
            .terminal_cleanup_evidence
            .as_ref()
            .unwrap()
            .send_attempts
            .len(),
        1
    );
    assert_eq!(
        observation
            .terminal_cleanup_evidence
            .as_ref()
            .unwrap()
            .release_outcome
            .attempts,
        1
    );
    assert!(observation.terminal_error.is_some());
    assert!(
        observation
            .secondary_errors
            .iter()
            .any(|error| error.contains("target HWND/generation changed"))
    );
}

#[test]
fn probe_held_keeps_needs_probe_without_retrying_physical_up() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    dispatch_owned_down_for_cleanup(&mut harness);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    let probes = Arc::new(AtomicUsize::new(0));
    let probe_count = Arc::clone(&probes);
    harness.resources.backend.set_probe(move |_, _| {
        probe_count.fetch_add(1, Ordering::AcqRel);
        InstrumentPhysicalState::Held(vec![0x15].into())
    });

    let error = harness
        .apply_scheduler_resumable_lifecycle_transition_for_test()
        .expect_err("Held probe must retain its unverified journal obligation");
    assert!(error.contains("release verification failed"), "{error}");
    let drain_probe_count = probes.load(Ordering::Acquire);
    assert!((1..=4).contains(&drain_probe_count));
    assert_eq!(cleanup.lock().expect("one initial physical Up").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let _outcome = harness.finalize_worker_for_test(false, &mut observation);
    assert!(probes.load(Ordering::Acquire) <= drain_probe_count + 4);
    assert_eq!(cleanup.lock().expect("no retry after needs_probe").len(), 1);
    assert_eq!(observation.release_obligation_mask, 1);
    assert!(observation.terminal_error.is_some());
}
#[test]
fn live_scheduler_late_wake_sends_both_scheduled_down_generations() {
    use crate::engine::worker::StagedDispatchOutcome;
    use sky_dispatch_core::executor::DispatchDecision;
    use sky_dispatch_core::model::OnsetChordInput;

    let chords = [
        OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: vec![0x15].into(),
        },
        OnsetChordInput {
            source_action_index: 2,
            scheduled_us: 17_167,
            scan_codes: vec![0x15].into(),
        },
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let packets = harness.configure_mock_packet_sender_for_test(&[1_000, 1_000, 1_000, 1_000]);
    let clock = harness.resources.clock;
    let now = clock.now().expect("initial live scheduler QPC");
    let epoch = QpcTicks::from_raw(
        now.as_u64().saturating_sub(
            clock
                .duration_from_us(1_000_000)
                .expect("past scheduler epoch")
                .as_u64(),
        ),
    );
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("live scheduler playback epoch");
    assert!(
        now > epoch,
        "the first authored Down is already late at wake"
    );

    let before_first_down = harness.generation_accounting_for_test();
    assert_eq!(before_first_down.activated, 0);
    assert_eq!(before_first_down.released, 0);
    assert_eq!(before_first_down.scheduled, 2);
    let (up_mask, down_mask, down_outcome) = dispatch_live_scheduler_next(&mut harness, now, epoch);
    assert_eq!(
        (up_mask, down_mask),
        (0, 1),
        "first production packet is Down"
    );
    assert!(matches!(down_outcome, StagedDispatchOutcome::Dispatched));
    let down_evidence = packets
        .lock()
        .expect("live scheduler packet capture")
        .first()
        .map(|(_, evidence)| *evidence)
        .expect("successful Down packet evidence");
    assert_eq!(down_evidence.confirmed_mask, 1);
    let down_completion = down_evidence
        .completed_ticks
        .expect("successful Down has valid completion QPC");
    let after_down = harness.generation_accounting_for_test();
    assert_eq!(after_down.activated, 1, "Down commits after completion");
    assert_eq!(after_down.active, 1);
    assert_eq!(after_down.released, 0);

    let hold = clock
        .duration_from_us(harness.config.timing.min_hold_us)
        .expect("live scheduler H conversion");
    let up_floor = harness.resources.scheduler.up_slot_timings(1)[0].release_not_before_qpc;
    let committed_down_completion = harness
        .runtime
        .last_physical_send_completion_qpc
        .expect("worker committed the successful Down completion QPC");
    assert!(
        committed_down_completion >= down_completion,
        "worker completion must not precede successful transport completion evidence"
    );
    let authored_up_target = epoch
        .checked_add_duration(clock.duration_from_us(17_167).expect("authored Up target"))
        .unwrap();
    assert_eq!(
        up_floor,
        committed_down_completion
            .checked_add_duration(hold)
            .unwrap()
            .max(authored_up_target),
        "production Up floor is the later of authored target and completion + H"
    );
    wait_until_qpc(clock, up_floor);
    let up_now = clock.now().expect("live scheduler Up QPC");
    let (up_mask, down_mask, up_outcome) =
        dispatch_live_scheduler_next(&mut harness, up_now, epoch);
    assert_eq!(
        (up_mask, down_mask),
        (1, 0),
        "second production packet is Up"
    );
    assert!(matches!(up_outcome, StagedDispatchOutcome::Dispatched));
    let up_evidence = packets
        .lock()
        .expect("live scheduler packet capture")
        .get(1)
        .map(|(_, evidence)| *evidence)
        .expect("successful Up packet evidence");
    assert_eq!(up_evidence.confirmed_mask, 1);
    let up_completion = up_evidence
        .completed_ticks
        .expect("successful Up has valid completion QPC");
    let after_up = harness.generation_accounting_for_test();
    assert_eq!(after_up.activated, 1);
    assert_eq!(
        after_up.released, 1,
        "Up commits its completion to scheduler"
    );
    assert_eq!(after_up.active, 0);

    let next_chord = harness
        .current_executor_chord_for_test()
        .expect("second production Down chord");
    let authored_next_down = epoch
        .checked_add_duration(DurationTicks::from_raw(next_chord.onset_ticks.as_u64()))
        .expect("second Down authored QPC");
    assert_eq!(
        authored_next_down,
        epoch
            .checked_add_duration(clock.duration_from_us(17_167).expect("repeat target"))
            .unwrap()
    );
    assert!(
        up_completion >= authored_next_down,
        "repeat is already authored-due after Up commit"
    );
    let sends_before_early_evaluation =
        packets.lock().expect("live scheduler packet capture").len();
    let repeat_decision = harness
        .resources
        .scheduler
        .evaluate_at(up_completion, epoch)
        .expect("authored repeat evaluation after Up commit");
    assert!(
        matches!(
            &repeat_decision,
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(_),
                ..
            }
        ),
        "committed Up makes an authored-due same-key repeat eligible without a musical gap: {repeat_decision:?}"
    );
    assert_eq!(
        packets.lock().expect("live scheduler packet capture").len(),
        sends_before_early_evaluation,
        "scheduler evaluation is side-effect free before the fresh repeat dispatch"
    );
    let before_retrigger = harness.generation_accounting_for_test();
    assert_eq!(before_retrigger.activated, 1);
    assert_eq!(before_retrigger.released, 1);
    assert_eq!(before_retrigger.active, 0);
    assert!(matches!(
        harness
            .resources
            .scheduler
            .evaluate_at(up_completion, epoch)
            .expect("repeat remains eligible after Up commit"),
        DispatchDecision::Send {
            down_chord: Some(_),
            ..
        }
    ));
    let retrigger_now = clock.now().expect("authored repeat QPC");
    let (up_mask, down_mask, retrigger_outcome) =
        dispatch_live_scheduler_next(&mut harness, retrigger_now, epoch);
    assert_eq!(
        (up_mask, down_mask),
        (0, 1),
        "third production packet is Down"
    );
    assert!(matches!(
        retrigger_outcome,
        StagedDispatchOutcome::Dispatched
    ));
    let retrigger_evidence = packets
        .lock()
        .expect("live scheduler packet capture")
        .get(2)
        .map(|(_, evidence)| *evidence)
        .expect("successful repeated Down packet evidence");
    assert!(retrigger_evidence.completed_ticks.is_some());

    let final_up_floor = harness.resources.scheduler.up_slot_timings(1)[0].release_not_before_qpc;
    wait_until_qpc(clock, final_up_floor);
    let final_up_now = clock.now().expect("final production Up QPC");
    let (up_mask, down_mask, final_up_outcome) =
        dispatch_live_scheduler_next(&mut harness, final_up_now, epoch);
    assert_eq!(
        (up_mask, down_mask),
        (1, 0),
        "fourth production packet is Up"
    );
    assert!(matches!(
        final_up_outcome,
        StagedDispatchOutcome::Dispatched
    ));

    let final_accounting = harness.generation_accounting_for_test();
    let captured = packets.lock().expect("live scheduler packet capture");
    assert_eq!(captured.len(), 4);
    assert_eq!(
        captured
            .iter()
            .filter(|(packet, _)| packet.down_mask != 0)
            .count(),
        2,
        "late wake preserves both scheduled Down packets"
    );
    assert_eq!(
        captured
            .iter()
            .map(|(packet, _)| (packet.up_mask, packet.down_mask))
            .collect::<Vec<_>>(),
        [(0, 1), (1, 0), (0, 1), (1, 0)],
        "two completed generations traverse exactly Down, Up, Down, Up"
    );
    assert_eq!(final_accounting.total, 2);
    assert_eq!(final_accounting.activated, 2);
    assert_eq!(final_accounting.released, 2);
    assert_eq!(final_accounting.active, 0);
    assert_eq!(final_accounting.cancelled, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    harness
        .resources
        .scheduler
        .check_invariants()
        .expect("normal live scheduler lifecycle accounting invariants");
}

#[test]
fn production_scheduler_lifecycle_matrix_releases_or_quarantines_accepted_downs() {
    use crate::engine::worker::{FinalizeTestObservation, StagedDispatchOutcome};
    use sky_dispatch_win32::input::PlatformSendResult;
    use std::sync::{Arc, Mutex};

    for path in [
        "stop",
        "skip",
        "pause",
        "focus",
        "target_generation",
        "suspend",
        "partial_cleanup",
        "cleanup_without_post_timestamp",
    ] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_mock_packet_sender_for_test(&[]);
        let cleanup = harness.configure_mock_cleanup_emitter_for_test();
        let clock = harness.resources.clock;
        let send_start = clock.now().expect("production lifecycle send QPC");
        harness.set_mock_sender_start_qpc_for_test(send_start);
        let chord = harness.current_executor_chord_for_test();
        let candidate = harness
            .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
            .expect("stage production scheduler Down")
            .expect("production scheduler Down candidate");
        assert!(matches!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            StagedDispatchOutcome::Dispatched
        ));
        assert_eq!(
            packets.lock().expect("production packet capture").len(),
            1,
            "{path}: one accepted Down before lifecycle transition"
        );
        let before = harness.resources.scheduler.accounting();
        assert_eq!(before.activated, 1, "{path}: live scheduler activated");
        assert_eq!(before.active, 1, "{path}: live scheduler has active key");
        assert_eq!(before.released, 0, "{path}: release precedes cleanup");
        assert_eq!(harness.release_obligation_mask_for_test(), 1, "{path}");

        match path {
            "stop" | "skip" | "suspend" => {
                match path {
                    "stop" => harness
                        .quit_requested
                        .store(true, std::sync::atomic::Ordering::Release),
                    "skip" => harness
                        .skip_requested
                        .store(true, std::sync::atomic::Ordering::Release),
                    "suspend" => {
                        assert!(harness.notify_system_power_for_test(true));
                        assert!(harness.notify_system_power_for_test(false));
                        assert!(harness.system_power_down_blocked_for_test());
                    }
                    _ => unreachable!(),
                }
                assert!(harness.process_command_control_for_test(), "{path}");
                let mut observation = FinalizeTestObservation::default();
                let outcome = harness.finalize_worker_for_test(false, &mut observation);
                let expected_outcome = match path {
                    "stop" => crate::engine::OUTCOME_QUIT,
                    "skip" => crate::engine::OUTCOME_SKIPPED,
                    "suspend" => crate::engine::OUTCOME_SYSTEM_SUSPENDED,
                    _ => unreachable!(),
                };
                assert_eq!(outcome, expected_outcome, "{path}");
                assert_eq!(observation.release_obligation_mask, 0, "{path}");
                assert_eq!(observation.active_mask, 0, "{path}");
                assert_eq!(observation.possibly_active_mask, 0, "{path}");
                assert_eq!(observation.failed_release_mask, 0, "{path}");
                assert_eq!(observation.in_flight_mask, 0, "{path}");
                assert_eq!(
                    cleanup.lock().expect("terminal cleanup capture").as_slice(),
                    &[(vec![0x15], true)],
                    "{path}: finalizer releases accepted scheduler Down"
                );
                assert!(
                    observation.generation_accounting.cancelled >= 1,
                    "{path}: finalizer cancels the live scheduler generation"
                );
                assert_eq!(observation.generation_accounting.active, 0, "{path}");
                continue;
            }
            "pause" => {
                harness
                    .desired_pause
                    .store(true, std::sync::atomic::Ordering::Release);
                harness
                    .apply_scheduler_resumable_lifecycle_transition_for_test()
                    .expect("manual pause cleanup through live scheduler");
            }
            "focus" => {
                harness
                    .focus_active
                    .store(false, std::sync::atomic::Ordering::Release);
                let focus_qpc = clock.now().expect("focus transition QPC");
                crate::engine::worker::enter_focus_pause(
                    &mut harness.resources.playback,
                    &mut harness.runtime,
                    focus_qpc,
                    &harness.progress_clock,
                )
                .expect("focus pause transition");
                harness
                    .apply_scheduler_resumable_lifecycle_transition_for_test()
                    .expect("focus cleanup through live scheduler");
            }
            "target_generation" => {
                harness.target.invalidate_for_test();
                let error = harness
                    .apply_scheduler_resumable_lifecycle_transition_for_test()
                    .expect_err("invalid immutable target must withhold safety Up");
                assert!(error.contains("target generation unavailable"), "{error}");
                assert!(
                    cleanup
                        .lock()
                        .expect("invalid target cleanup capture")
                        .is_empty()
                );
                assert_eq!(harness.release_obligation_mask_for_test(), 1);
                assert_eq!(harness.resources.backend.failed_release_mask, 1);
                assert_eq!(harness.resources.backend.release_attempts_total, 0);
                continue;
            }
            "partial_cleanup" => {
                let partial_calls = Arc::new(Mutex::new(0usize));
                let partial_calls_for_emitter = Arc::clone(&partial_calls);
                harness
                    .resources
                    .backend
                    .set_emitter(move |scan_codes, _key_up| {
                        *partial_calls_for_emitter
                            .lock()
                            .expect("partial cleanup calls") += 1;
                        PlatformSendResult {
                            requested: scan_codes.len() as u8,
                            inserted: 0,
                            started_ticks: QpcTicks::ZERO,
                            completed_ticks: Some(QpcTicks::ZERO),
                            win32_error: 5,
                            timing_error: None,
                        }
                    });
                assert!(
                    harness
                        .apply_scheduler_resumable_lifecycle_transition_for_test()
                        .is_err()
                );
                assert!(*partial_calls.lock().expect("partial cleanup calls") > 0);
                assert_ne!(harness.release_obligation_mask_for_test(), 0);
                let retry_cleanup = harness.configure_mock_cleanup_emitter_for_test();
                harness
                    .apply_scheduler_resumable_lifecycle_transition_for_test()
                    .expect("verified retry clears live scheduler state");
                assert_eq!(
                    retry_cleanup
                        .lock()
                        .expect("retry cleanup capture")
                        .as_slice(),
                    &[(vec![0x15], true)]
                );
            }
            "cleanup_without_post_timestamp" => {
                harness
                    .apply_scheduler_resumable_lifecycle_transition_for_test()
                    .expect("verified cleanup does not require a separate QPC timestamp");
                assert_eq!(harness.release_obligation_mask_for_test(), 0);
                assert_eq!(harness.resources.scheduler.accounting().active, 0);
                assert_eq!(
                    cleanup
                        .lock()
                        .expect("timestamp-free cleanup capture")
                        .as_slice(),
                    &[(vec![0x15], true)]
                );
            }
            _ => unreachable!("the production scheduler lifecycle cases are explicit"),
        }

        assert_eq!(harness.release_obligation_mask_for_test(), 0, "{path}");
        assert_eq!(harness.resources.backend.active_mask, 0, "{path}");
        let after = harness.resources.scheduler.accounting();
        assert_eq!(after.active, 0, "{path}: scheduler active state cleared");
        assert!(
            after.cancelled >= 1,
            "{path}: remaining generation cancelled"
        );
        harness
            .resources
            .scheduler
            .check_invariants()
            .unwrap_or_else(|error| panic!("{path}: scheduler cleanup invariants: {error}"));
    }
}

#[test]
fn live_scheduler_final_down_gate_rejects_target_lease_and_control_without_commit() {
    use crate::engine::worker::StagedDispatchOutcome;
    use std::sync::atomic::Ordering;

    for rejection in ["target_generation", "supervisor_lease", "quit_control"] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_mock_packet_sender_for_test(&[0]);
        let chord = harness.current_executor_chord_for_test();
        let candidate = harness
            .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
            .expect("stage production final-gate candidate")
            .expect("production final-gate Down candidate");
        let before_accounting = harness.generation_accounting_for_test();

        match rejection {
            "target_generation" => harness.target.invalidate_for_test(),
            "supervisor_lease" => {
                harness.supervisor_expired.latch_expired();
            }
            "quit_control" => harness.quit_requested.store(true, Ordering::Release),
            _ => unreachable!(),
        }

        let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
        assert_ne!(outcome, StagedDispatchOutcome::Dispatched, "{rejection}");
        assert!(packets.lock().expect("scheduler packet capture").is_empty());
        assert_eq!(
            harness.generation_accounting_for_test(),
            before_accounting,
            "{rejection}: denied final gate cannot commit scheduler accounting"
        );
        assert_eq!(
            harness.current_executor_chord_for_test(),
            chord,
            "{rejection}: denied final gate cannot advance the Down cursor"
        );
    }
}

#[test]
fn live_scheduler_partial_chord_send_is_not_replayed_and_consumed_after_cleanup() {
    use crate::engine::worker::{FinalizeTestObservation, StagedDispatchOutcome};
    use std::sync::{Arc, Mutex};

    let mut harness = ProductionDispatchTestHarness::new_down_chord(2);
    let packet_attempts = Arc::new(Mutex::new(Vec::new()));
    let attempts = Arc::clone(&packet_attempts);
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        attempts
            .lock()
            .expect("partial production packet capture")
            .push(packet);
        let started_ticks = clock.now().expect("partial sender start QPC");
        let completed_ticks = clock.now().expect("partial sender completion QPC");
        SendTransactionOutcome {
            status: SendTransactionStatus::IntegrityLost,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 1,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress { inserted_count: 1 },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started_ticks),
                completed_ticks: Some(completed_ticks),
                timing_error: None,
            },
        }
    });
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage production chord")
        .expect("production chord candidate");
    let before = harness.generation_accounting_for_test();

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    ));
    let attempts = packet_attempts
        .lock()
        .expect("partial production packet capture");
    assert_eq!(attempts.len(), 1, "the split Down chord must never replay");
    assert_eq!(attempts[0].up_mask, 0);
    assert_eq!(attempts[0].down_mask.count_ones(), 2);
    drop(attempts);
    let accounting = harness.generation_accounting_for_test();
    assert_eq!(accounting.total, before.total);
    assert_eq!(accounting.activated, 0);
    assert_eq!(accounting.cancelled, 2);
    assert_eq!(harness.current_executor_chord_for_test(), None);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert!(
        harness
            .runtime
            .attempted_dispatch_anomaly
            .detail()
            .is_some_and(|detail| detail.contains("attempts=1"))
    );

    let mut observation = FinalizeTestObservation::default();
    let _ = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(observation.attempted_mask, 0);
    assert_eq!(observation.release_obligation_mask, 0);
    assert_eq!(
        cleanup.lock().expect("partial cleanup capture").as_slice(),
        &[(vec![0x15, 0x16], true)]
    );
}

#[test]
fn live_scheduler_terminal_finalizer_cleans_lease_panic_and_worker_unwind() {
    use crate::engine::worker::{FinalizeTestObservation, StagedDispatchOutcome};
    use std::sync::atomic::Ordering;

    for path in ["supervisor_lease", "panic_request", "worker_unwind"] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_mock_packet_sender_for_test(&[0]);
        let cleanup = harness.configure_mock_cleanup_emitter_for_test();
        let chord = harness.current_executor_chord_for_test();
        let candidate = harness
            .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
            .expect("stage production terminal candidate")
            .expect("production terminal Down candidate");
        assert!(matches!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            StagedDispatchOutcome::Dispatched
        ));
        assert_eq!(
            packets
                .lock()
                .expect("terminal scheduler packet capture")
                .len(),
            1,
            "{path}: one production Down before terminal path"
        );

        let mut observation = FinalizeTestObservation::default();
        match path {
            "supervisor_lease" => {
                harness.supervisor_expired.latch_expired();
                assert!(harness.process_command_control_for_test());
                assert_eq!(
                    harness.finalize_worker_for_test(false, &mut observation),
                    crate::engine::OUTCOME_ERROR
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("supervisor_lease_expired")
                );
            }
            "panic_request" => {
                harness.panic_requested.store(true, Ordering::Release);
                assert!(harness.process_command_control_for_test());
                assert_eq!(
                    harness.finalize_worker_for_test(false, &mut observation),
                    crate::engine::OUTCOME_ERROR
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("panic_release_requested")
                );
            }
            "worker_unwind" => {
                let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    harness.finalize_worker_for_test(true, &mut observation);
                }));
                assert!(
                    unwind.is_err(),
                    "worker panic must propagate after finalization"
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("worker_panic")
                );
            }
            _ => unreachable!(),
        }

        assert_eq!(
            observation.attempted_mask, 1,
            "{path}: tracked ownership only"
        );
        assert_eq!(observation.release_obligation_mask, 0, "{path}");
        assert_eq!(observation.generation_accounting.active, 0, "{path}");
        assert!(observation.generation_accounting.cancelled >= 1, "{path}");
        assert_eq!(
            cleanup.lock().expect("terminal cleanup capture").as_slice(),
            &[(vec![0x15], true)],
            "{path}: one bounded terminal Up after the accepted Down"
        );
    }
}

#[test]
fn live_scheduler_verified_cleanup_cancels_without_a_post_cleanup_timestamp() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_mock_packet_sender_for_test(&[0]);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage QPC barrier candidate")
        .expect("scheduler QPC barrier Down candidate");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        crate::engine::worker::StagedDispatchOutcome::Dispatched
    ));

    let outcome = harness.apply_scheduler_resumable_lifecycle_transition_for_test();
    assert!(outcome.is_ok());
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.scheduler.accounting().active, 0);
    assert_eq!(harness.resources.scheduler.accounting().cancelled, 1);
    assert_eq!(
        cleanup
            .lock()
            .expect("QPC failure cleanup capture")
            .as_slice(),
        &[(vec![0x15], true)]
    );
}

#[test]
fn lifecycle_precision_fixture_lease_config_materializes_qpc_timeout() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.config.wait.supervisor_lease_timeout_us = 3_000_000;
    let config = harness
        .dispatch_timing_config_for_test()
        .expect("dispatch timing metadata");
    assert!(config.native_admission_passed);
    assert!(config.lease_enabled);
    assert_eq!(config.lease_timeout_us, 3_000_000);
    assert_eq!(
        config.lease_timeout_ticks,
        harness
            .resources
            .clock
            .duration_from_us(3_000_000)
            .unwrap()
            .as_u64()
    );
}

#[test]
fn admissible_down_fences_when_focus_lost_or_stale_preflight() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_focus_owner_for_test(1, 77);
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage Down candidate")
        .expect("Down candidate exists");

    // Focus is dropped between staging (preflight) and final admission
    harness.focus_active.store(false, Ordering::Release);
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);

    assert_eq!(
        outcome,
        crate::engine::worker::StagedDispatchOutcome::Replan
    );
    assert!(
        packets.lock().expect("packets lock").is_empty(),
        "no packets sent when focus is lost at final admission"
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.generation_accounting_for_test().activated, 0);
}

#[test]
fn focus_and_pause_fencing_suppresses_pending_down() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage Down candidate")
        .expect("Down candidate exists");

    harness.desired_pause.store(true, Ordering::Release);
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);

    assert_eq!(
        outcome,
        crate::engine::worker::StagedDispatchOutcome::Replan
    );
    assert!(
        packets.lock().expect("packets lock").is_empty(),
        "no packets sent when paused"
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.generation_accounting_for_test().activated, 0);
}

#[test]
fn generation_and_release_obligation_lifecycle() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let chord = harness.current_executor_chord_for_test();
    let down_candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage Down candidate")
        .expect("Down candidate exists");

    let down_outcome = harness.dispatch_staged_candidate_for_test(down_candidate, false);
    assert_eq!(
        down_outcome,
        crate::engine::worker::StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.generation_accounting_for_test().active, 1);
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().released, 0);

    let up_floor = harness.resources.scheduler.up_slot_timings(1)[0].release_not_before_qpc;
    wait_until_qpc(harness.resources.clock, up_floor);

    let up_candidate = harness
        .stage_candidate_for_test(1, None, TimelineTicks::ZERO)
        .expect("stage Up candidate")
        .expect("Up candidate exists");
    let up_outcome = harness.dispatch_staged_candidate_for_test(up_candidate, false);
    assert_eq!(
        up_outcome,
        crate::engine::worker::StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.generation_accounting_for_test().active, 0);
    assert_eq!(harness.generation_accounting_for_test().released, 1);
    assert_eq!(packets.lock().expect("packets lock").len(), 2);
}

#[test]
fn target_owner_proof_invalidation_rejects_at_final_admission() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage Down candidate")
        .expect("Down candidate exists");

    harness.target.invalidate_owner_identity();

    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    assert_eq!(
        outcome,
        crate::engine::worker::StagedDispatchOutcome::Replan
    );
    assert!(packets.lock().expect("packets lock").is_empty());
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
}

#[test]
fn negative_control_unconfirmed_packet_fails_closed() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let completed = clock.now().expect("now");
        SendTransactionOutcome {
            status: SendTransactionStatus::IntegrityLost,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress { inserted_count: 0 },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(completed),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage Down")
        .expect("candidate");

    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    assert_eq!(outcome, crate::engine::worker::StagedDispatchOutcome::Exit);
    assert!(
        harness
            .runtime
            .attempted_dispatch_anomaly
            .detail()
            .is_some()
    );
}

#[test]
fn controlled_late_wake_reaches_mixed_packet_ambiguity_and_exact_cleanup_scope() {
    use sky_dispatch_core::model::OnsetChordInput;

    // Onset-only A@0, B@H: under completion-anchored semantics, a controlled late
    // wake past max(epoch + H, completion_A + H) deterministically makes both
    // Up(A) and Down(B) due together in one mixed packet (mask 0b11).
    let chords = [
        OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: vec![0x15].into(),
        },
        OnsetChordInput {
            source_action_index: 1,
            scheduled_us: DISPATCH_EFFECTIVE_HOLD_US,
            scan_codes: vec![0x16].into(),
        },
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let clock = harness.resources.clock;
    let epoch = clock.now().expect("epoch QPC");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor epoch");

    // 1. Dispatch Down(A) at epoch.
    let (up_0, down_0, out_0) = dispatch_live_scheduler_next(&mut harness, epoch, epoch);
    assert_eq!((up_0, down_0), (0, 0b01));
    assert_eq!(
        out_0,
        crate::engine::worker::StagedDispatchOutcome::Dispatched
    );
    let completion_a = packets
        .lock()
        .expect("packets")
        .first()
        .and_then(|(_, ev)| ev.completed_ticks)
        .expect("Down(A) completion QPC");
    let hold_ticks = clock
        .duration_from_us(DISPATCH_EFFECTIVE_HOLD_US)
        .expect("H ticks");

    // 2. Controlled late wake past completion_A + H + 500us so both Up(A) and Down(B) are due.
    let late_wake = completion_a
        .checked_add_duration(hold_ticks)
        .expect("release floor")
        .checked_add_duration(clock.duration_from_us(500).expect("late margin"))
        .expect("late wake");
    wait_until_qpc(clock, late_wake);

    // 3. Inject partial/ambiguous SendInput on the mixed 0b11 packet.
    let mixed_seen = Arc::new(AtomicUsize::new(0));
    let mixed_seen_cb = Arc::clone(&mixed_seen);
    harness.resources.backend.set_packet_emitter(move |packet| {
        assert_eq!(
            packet,
            PhysicalPacket::new(0b01, 0b10),
            "controlled late wake must reach exact mixed packet Up(A)+Down(B)"
        );
        mixed_seen_cb.fetch_add(1, Ordering::AcqRel);
        let completed = clock.now().expect("ambiguous send QPC");
        SendTransactionOutcome {
            status: SendTransactionStatus::PartialProgress,
            evidence: SendEvidence {
                requested_mask: 0b11,
                confirmed_mask: 0b01,
                skipped_mask: 0,
                first_inserted: 1,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress { inserted_count: 1 },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(completed),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    let now = clock.now().expect("mixed evaluation QPC");
    let (up_1, down_1, out_1) = dispatch_live_scheduler_next(&mut harness, now, epoch);
    assert_eq!((up_1, down_1), (0b01, 0b10));
    assert_eq!(out_1, crate::engine::worker::StagedDispatchOutcome::Exit);
    assert_eq!(mixed_seen.load(Ordering::Acquire), 1);

    // 4. Verify exact journal cleanup scope (both A and B released) and zero residual.
    let cleanup_calls = cleanup.lock().expect("cleanup capture");
    assert!(!cleanup_calls.is_empty(), "bounded cleanup must run");
    assert_eq!(
        cleanup_calls[0].0,
        vec![0x15, 0x16],
        "ambiguity cleanup must scope exactly to both journaled keys 0b11"
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.in_flight_mask_for_test(), 0);
}
