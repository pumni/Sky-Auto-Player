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
            config.frame_ticks,
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
    let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
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
        .apply_scheduler_resumable_lifecycle_transition_for_test()
        .expect("verified scheduler suspension cleanup");

    let accounting = harness.resources.scheduler.accounting();
    assert_eq!(accounting.activated, 1);
    assert_eq!(accounting.active, 0);
    assert_eq!(accounting.cancelled, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    harness
        .resources
        .scheduler
        .check_invariants()
        .expect("scheduler cleanup invariants");
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
        .checked_add_duration(clock.duration_from_us(DISPATCH_FRAME_US).unwrap())
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
