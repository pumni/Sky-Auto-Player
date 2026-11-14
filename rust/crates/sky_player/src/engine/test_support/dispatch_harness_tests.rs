#![cfg(any(test, feature = "test-support"))]

use super::*;
use std::num::NonZeroU64;

fn qpc_at(clock: QpcClock, microseconds: u64) -> QpcTicks {
    QpcTicks::from_raw(
        clock
            .duration_from_us(microseconds)
            .expect("dispatch test QPC conversion")
            .as_u64(),
    )
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

    let (up_mask, down_chord): (u16, Option<sky_dispatch_core::executor::DownChordMetadata>) =
        match harness
            .resources
            .scheduler
            .evaluate_at(now, epoch)
            .expect("live scheduler evaluation")
        {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                ..
            } => (up_mask, down_chord.map(Into::into)),
            decision => panic!("expected production scheduler Send decision, got {decision:?}"),
        };
    let down_mask = down_chord.as_ref().map_or(0, |chord| chord.down_mask);
    let candidate = harness
        .stage_candidate_for_test(up_mask, down_chord, TimelineTicks::ZERO)
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
        ProductionDispatchTestHarness::new_prepared_mixed_for_test(1, 1, 20_000)
            .expect("Mixed fixture admission"),
        ProductionDispatchTestHarness::new_prepared_down_or_uponly_for_test(1, 20_000)
            .expect("Down/UpOnly fixture admission"),
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
        assert_eq!(config.min_release_gap_us, DISPATCH_RELEASE_GAP_US);
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
            config.min_release_gap_ticks,
            fixture
                .resources
                .clock
                .duration_from_us(DISPATCH_RELEASE_GAP_US)
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
    harness.legacy_coordinator_fixture =
        sky_dispatch_core::coordinator::RuntimeDispatchCoordinator::try_new_ticks(
            sky_dispatch_core::model::RuntimeSchedule {
                packets: Vec::new(),
                batches: Vec::new(),
                intents: Vec::new(),
                generation_count: 0,
                key_registry: sky_dispatch_core::model::KeyRegistry::new(&[]),
                reason_table: Vec::new(),
                down_chords: Vec::new(),
                schedule_end_offset_us: 0,
                compile_memory: Default::default(),
            },
            0,
            sky_dispatch_core::time::DurationTicks::ZERO,
            |us| Ok(TimelineTicks::from_raw(us)),
        )
        .expect("isolated empty legacy coordinator fixture");
    assert_ne!(
        harness.legacy_coordinator_fixture.generation_accounting(),
        scheduler_before,
        "empty legacy fixture must differ so it cannot mask the production owner"
    );
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
    let (_cleanup, cleanup_completion) =
        harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    let clock = harness.resources.clock;
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage scheduler Down")
        .expect("scheduler Down candidate");
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
        .apply_scheduler_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(|| {
            let completed = cleanup_completion.load(Ordering::Acquire);
            Ok(QpcTicks::from_raw(completed))
        })
        .expect("verified scheduler suspension cleanup");

    let accounting = harness.resources.scheduler.accounting();
    assert_eq!(accounting.activated, 1);
    assert_eq!(accounting.active, 0);
    assert_eq!(
        accounting.released, 0,
        "cleanup cancels; it does not release"
    );
    assert_eq!(accounting.cancelled, 1);
    let cleanup_qpc = QpcTicks::from_raw(cleanup_completion.load(Ordering::Acquire));
    let gap = clock
        .duration_from_us(harness.config.timing.min_release_gap_us)
        .expect("production scheduler cleanup G conversion");
    assert_eq!(
        harness.resources.scheduler.resume_barrier_qpc(),
        Some(
            cleanup_qpc
                .checked_add_duration(gap)
                .expect("cleanup G barrier arithmetic")
        ),
        "verified cleanup establishes cleanup completion + G"
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    harness
        .resources
        .scheduler
        .check_invariants()
        .expect("scheduler cleanup invariants");
}

#[test]
fn live_scheduler_completion_gap_and_two_generation_accounting_qualify() {
    use crate::engine::worker::StagedDispatchOutcome;
    use sky_dispatch_core::executor::DispatchDecision;

    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(1_000_000, false)
            .expect("live scheduler lifecycle fixture");
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
    assert_eq!(
        up_floor,
        down_completion.checked_add_duration(hold).unwrap()
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

    let gap = clock
        .duration_from_us(harness.config.timing.min_release_gap_us)
        .expect("live scheduler G conversion");
    let completion_g_floor = up_completion
        .checked_add_duration(gap)
        .expect("live scheduler completion + G");
    let next_chord = harness
        .current_executor_chord_for_test()
        .expect("second production Down chord");
    let authored_next_down = epoch
        .checked_add_duration(DurationTicks::from_raw(next_chord.scheduled_ticks.as_u64()))
        .expect("second Down authored QPC");
    let exact_g_floor = completion_g_floor.max(authored_next_down);
    assert_eq!(exact_g_floor, completion_g_floor);
    let one_tick_early = QpcTicks::from_raw(exact_g_floor.as_u64() - 1);
    let sends_before_early_evaluation =
        packets.lock().expect("live scheduler packet capture").len();
    assert!(matches!(
        harness
            .resources
            .scheduler
            .evaluate_at(one_tick_early, epoch)
            .expect("G-1 scheduler evaluation"),
        DispatchDecision::Wait { deadline_qpc } if deadline_qpc == exact_g_floor
    ));
    assert_eq!(
        packets.lock().expect("live scheduler packet capture").len(),
        sends_before_early_evaluation,
        "G-1 scheduler denial performs no physical send"
    );
    let before_retrigger = harness.generation_accounting_for_test();
    assert_eq!(before_retrigger.activated, 1);
    assert_eq!(before_retrigger.released, 1);
    assert_eq!(before_retrigger.active, 0);
    assert!(matches!(
        harness
            .resources
            .scheduler
            .evaluate_at(exact_g_floor, epoch)
            .expect("exact-G scheduler evaluation"),
        DispatchDecision::Send {
            down_chord: Some(_),
            ..
        }
    ));
    wait_until_qpc(clock, exact_g_floor);
    let retrigger_now = clock.now().expect("exact-G retrigger QPC");
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
fn production_scheduler_lifecycle_matrix_cleans_accepted_downs() {
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
        "cleanup_qpc_failure",
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
                harness.target.publish(2);
                harness
                    .apply_scheduler_resumable_lifecycle_transition_for_test()
                    .expect("target generation cleanup through live scheduler");
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
            "cleanup_qpc_failure" => {
                assert!(harness
                    .apply_scheduler_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(
                        || Err("injected scheduler cleanup QPC failure".to_string())
                    )
                    .is_err());
                assert_eq!(harness.release_obligation_mask_for_test(), 0);
                assert_eq!(harness.resources.scheduler.accounting().active, 1);
                assert_eq!(
                    cleanup
                        .lock()
                        .expect("QPC failure cleanup capture")
                        .as_slice(),
                    &[(vec![0x15], true)]
                );
                harness
                    .apply_scheduler_resumable_lifecycle_transition_for_test()
                    .expect("verified retry resolves scheduler state after QPC failure");
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
            "target_generation" => assert!(harness.target.publish(2)),
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
fn live_scheduler_partial_chord_send_is_not_replayed_and_remains_owned_for_cleanup() {
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
    assert_eq!(harness.generation_accounting_for_test(), before);
    assert_eq!(harness.current_executor_chord_for_test(), chord);
    let obligation = harness.release_obligation_mask_for_test();
    assert_eq!(obligation, 0b11);

    let mut observation = FinalizeTestObservation::default();
    let _ = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(observation.attempted_mask, obligation);
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
fn live_scheduler_cleanup_qpc_failure_never_mints_resumable_barrier() {
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

    let outcome = harness
        .apply_scheduler_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(|| {
            Err("injected post-cleanup QPC failure".to_string())
        });
    assert!(outcome.is_err());
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.scheduler.accounting().active, 1);
    assert_eq!(harness.resources.scheduler.resume_barrier_qpc(), None);
    assert_eq!(
        cleanup
            .lock()
            .expect("QPC failure cleanup capture")
            .as_slice(),
        &[(vec![0x15], true)]
    );
}

#[test]
fn lifecycle_not_due_dispatch_preserves_sender_cursor_and_generation_until_floor() {
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(10_000_000, false)
            .expect("lifecycle fixture admission");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 50, 0, 0]);
    harness.prepare_prepared_stream_for_test();
    harness
        .reanchor_playback_clock_for_test(qpc_at(clock, 82_700))
        .expect("playback epoch");

    let down_at = qpc_at(clock, 100_000);
    let down_floor = harness
        .prepared_physical_floor_for_test()
        .expect("initial Down floor");
    assert!(down_at >= down_floor.packet_not_before_qpc);
    harness.set_mock_sender_start_qpc_for_test(down_at);
    assert!(matches!(
        harness
            .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(down_at,)
            .expect("due initial Down"),
        Some(DispatchStep::Dispatched)
    ));
    let up_floor = harness
        .prepared_physical_floor_for_test()
        .expect("musical Up floor");
    harness.set_mock_sender_start_qpc_for_test(up_floor.packet_not_before_qpc);
    assert!(matches!(
        harness
            .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(
                up_floor.packet_not_before_qpc,
            )
            .expect("due musical Up"),
        Some(DispatchStep::Dispatched)
    ));

    let released_at = evidence_completion_for_harness(&packets, 1);
    let expected_floor = released_at
        .checked_add_duration(
            clock
                .duration_from_us(harness.config.timing.min_release_gap_us)
                .unwrap(),
        )
        .expect("release floor");
    let floor_evidence = harness
        .prepared_physical_floor_for_test()
        .expect("next Down floor");
    let floor = floor_evidence.packet_not_before_qpc;
    assert_eq!(floor, expected_floor);

    let early = qpc_at(clock, 118_300);
    let cursor_before = harness.prepared_cursor_for_test();
    let attempts_before = packets.lock().unwrap().len();
    let accounting_before = harness.generation_accounting_for_test();
    let obligation_before = harness.release_obligation_mask_for_test();
    harness.set_mock_sender_start_qpc_for_test(early);
    assert!(
        harness
            .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(early,)
            .expect("not-due check")
            .is_none()
    );
    assert_eq!(packets.lock().unwrap().len(), attempts_before);
    assert_eq!(harness.prepared_cursor_for_test(), cursor_before);
    let accounting_after = harness.generation_accounting_for_test();
    assert_eq!(accounting_after.activated, accounting_before.activated);
    assert_eq!(accounting_after.released, accounting_before.released);
    assert_eq!(accounting_after.cancelled, accounting_before.cancelled);
    assert_eq!(
        harness.release_obligation_mask_for_test(),
        obligation_before
    );

    harness.set_mock_sender_start_qpc_for_test(floor);
    assert!(matches!(
        harness
            .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(floor,)
            .expect("due next Down"),
        Some(DispatchStep::Dispatched)
    ));
    assert_eq!(packets.lock().unwrap().len(), attempts_before + 1);
    assert_eq!(harness.prepared_cursor_for_test(), cursor_before + 1);
}

#[test]
fn lifecycle_precision_fixture_lease_config_materializes_qpc_timeout() {
    let mut harness =
        ProductionDispatchTestHarness::new_prepared_down_or_uponly_for_test(1, 20_000)
            .expect("precision fixture admission");
    harness
        .set_supervisor_lease_timeout_for_test(3_000_000)
        .expect("lease timeout conversion");
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

fn evidence_completion_for_harness(
    packets: &Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>>,
    index: usize,
) -> QpcTicks {
    packets.lock().unwrap()[index]
        .1
        .completed_ticks
        .expect("mock completion evidence")
}
