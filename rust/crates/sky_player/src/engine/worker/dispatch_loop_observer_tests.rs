#[test]
fn completion_qpc_missing_and_clock_error_fails_closed() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.resources.backend.set_packet_emitter(|packet| {
        sky_dispatch_win32::clock::set_qpc_now_fault_for_test(true);
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
                started_ticks: None,
                completed_ticks: None,
                timing_error: None,
            },
        }
    });
    harness.attach_executor_for_test().expect("attach executor");

    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("some candidate");

    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(false);

    assert!(matches!(outcome, super::StagedDispatchOutcome::Exit));
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("SendInput completion QPC unavailable")
    );
    assert_eq!(
        harness
            .resources
            .coordinator
            .executor
            .as_ref()
            .unwrap()
            .chord_cursor(),
        0
    );
    assert!(harness.runtime.last_physical_send_completion_qpc.is_none());
    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.active_mask, 0);
    assert_eq!(evidence.possibly_active_mask, 0);
    assert_eq!(evidence.failed_release_mask, 0);
    assert!(!cleanup.lock().expect("cleanup capture").is_empty());
}

#[test]
fn completion_qpc_missing_and_clock_success_dispatches() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness
        .resources
        .backend
        .set_packet_emitter(|packet| SendTransactionOutcome {
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
                started_ticks: None,
                completed_ticks: None,
                timing_error: None,
            },
        });
    harness.attach_executor_for_test().expect("attach executor");

    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("some candidate");

    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(false);
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);

    assert!(matches!(outcome, super::StagedDispatchOutcome::Dispatched));
    assert!(harness.runtime.terminal_error.is_none());
    assert_eq!(
        harness
            .resources
            .coordinator
            .executor
            .as_ref()
            .unwrap()
            .chord_cursor(),
        1
    );
}

#[test]
fn wait_observation_is_published_promptly_on_dispatch() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.attach_executor_for_test().expect("attach executor");

    let target_qpc = harness.qpc_now_for_test().expect("target");
    harness.runtime.pending_wait_observation = crate::engine::worker::dispatch::record_wait_attempt(
        Some(&harness.observer),
        Some(sky_dispatch_win32::wait::WaitResult {
            outcome: sky_dispatch_win32::wait::WaitOutcome::Deadline,
            wake_qpc: Some(target_qpc),
            spin_ticks: DurationTicks::from_raw(123),
            timer_return_qpc: Some(target_qpc),
            timer_arm_target_qpc: Some(target_qpc),
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_qpc: Some(target_qpc),
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_generation: Some(1),
            #[cfg(any(test, feature = "test-support"))]
            timer_return_generation: Some(1),
        }),
        target_qpc,
        DurationTicks::from_raw(10_000),
        TimelineTicks::ZERO,
        harness.resources.playback.epoch,
        None,
        &mut harness.local_metrics,
    );
    assert!(harness.runtime.pending_wait_observation.is_some());

    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("some candidate");

    let outcome = harness.dispatch_staged_candidate_for_test(candidate, true);
    assert!(matches!(outcome, super::StagedDispatchOutcome::Dispatched));

    assert!(harness.runtime.pending_wait_observation.is_none());
    let observations: Vec<_> = std::iter::from_fn(|| harness.observer.pop_front()).collect();
    let waits: Vec<_> = observations
        .iter()
        .filter_map(|observation| match observation {
            crate::engine::worker::dispatch::observation::DispatchObservation::Wait(wait) => {
                Some(wait)
            }
            _ => None,
        })
        .collect();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].wake_qpc, Some(target_qpc));
    assert_eq!(waits[0].physical_target_qpc, target_qpc);
    assert_eq!(waits[0].spin_ticks, DurationTicks::from_raw(123));
    assert_eq!(waits[0].timer_arm_generation, Some(1));
    assert_eq!(waits[0].timer_return_generation, Some(1));
    let packet = observations
        .iter()
        .find_map(|observation| match observation {
            crate::engine::worker::dispatch::observation::DispatchObservation::Down(packet) => {
                Some(packet)
            }
            _ => None,
        })
        .expect("Down packet observation");
    assert_eq!(packet.wait_sequence, waits[0].sequence);
    assert_eq!(packet.wait_spin_ticks, 123);

    let second_target = harness.qpc_now_for_test().expect("second target");
    harness.runtime.pending_wait_observation = crate::engine::worker::dispatch::record_wait_attempt(
        Some(&harness.observer),
        Some(sky_dispatch_win32::wait::WaitResult {
            outcome: sky_dispatch_win32::wait::WaitOutcome::Deadline,
            wake_qpc: Some(second_target),
            spin_ticks: DurationTicks::from_raw(456),
            timer_return_qpc: Some(second_target),
            timer_arm_target_qpc: Some(second_target),
            timer_arm_qpc: Some(second_target),
            timer_arm_generation: Some(2),
            timer_return_generation: Some(2),
        }),
        second_target,
        DurationTicks::from_raw(20_000),
        TimelineTicks::from_raw(20_000),
        harness.resources.playback.epoch,
        None,
        &mut harness.local_metrics,
    );
    let candidate = harness
        .stage_candidate_for_test(1, None, TimelineTicks::from_raw(20_000))
        .expect("stage Up")
        .expect("Up candidate");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, true),
        super::StagedDispatchOutcome::Dispatched
    ));
    let observations: Vec<_> = std::iter::from_fn(|| harness.observer.pop_front()).collect();
    let packet = observations
        .iter()
        .find_map(|observation| match observation {
            crate::engine::worker::dispatch::observation::DispatchObservation::Down(packet) => {
                Some(packet)
            }
            _ => None,
        })
        .expect("Up packet observation");
    assert_eq!(packet.up_slot_count, 1);
    assert!(packet.up_slots.is_some());
    let waits: Vec<_> = observations
        .iter()
        .filter_map(|observation| match observation {
            crate::engine::worker::dispatch::observation::DispatchObservation::Wait(wait) => {
                Some(wait)
            }
            _ => None,
        })
        .collect();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].wake_qpc, Some(second_target));
    assert_eq!(waits[0].physical_target_qpc, second_target);
    assert_eq!(waits[0].spin_ticks, DurationTicks::from_raw(456));
    assert_eq!(waits[0].timer_arm_generation, Some(2));
    assert_eq!(waits[0].timer_return_generation, Some(2));
    assert_eq!(packet.wait_sequence, waits[0].sequence);
    assert_eq!(packet.wait_spin_ticks, 456);
    assert_ne!(waits[0].sequence, 1);
}

#[test]
fn mixed_packet_preserves_one_wait_with_matching_identity() {
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let first = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage first")
        .expect("first");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        super::StagedDispatchOutcome::Dispatched
    ));
    let target = harness.qpc_now_for_test().expect("wait target");
    harness.runtime.pending_wait_observation = crate::engine::worker::dispatch::record_wait_attempt(
        Some(&harness.observer),
        Some(sky_dispatch_win32::wait::WaitResult {
            outcome: sky_dispatch_win32::wait::WaitOutcome::Deadline,
            wake_qpc: Some(target),
            spin_ticks: DurationTicks::from_raw(19),
            timer_return_qpc: Some(target),
            timer_arm_target_qpc: Some(target),
            timer_arm_qpc: Some(target),
            timer_arm_generation: Some(3),
            timer_return_generation: Some(3),
        }),
        target,
        DurationTicks::from_raw(100),
        TimelineTicks::from_raw(1_000),
        harness.resources.playback.epoch,
        None,
        &mut harness.local_metrics,
    );
    let chord = harness.current_executor_chord_for_test();
    let mixed = harness
        .stage_candidate_for_test(1, chord, TimelineTicks::from_raw(1_000))
        .expect("stage mixed")
        .expect("mixed");
    assert_ne!(mixed.packet_masks.up_mask, 0);
    assert_ne!(mixed.packet_masks.down_mask, 0);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(mixed, true),
        super::StagedDispatchOutcome::Dispatched
    ));
    let observations: Vec<_> = std::iter::from_fn(|| harness.observer.pop_front()).collect();
    let packet = observations
        .iter()
        .find_map(|item| match item {
            crate::engine::worker::dispatch::observation::DispatchObservation::Down(packet) => {
                Some(packet)
            }
            _ => None,
        })
        .expect("mixed packet observation");
    let waits: Vec<_> = observations
        .iter()
        .filter_map(|item| match item {
            crate::engine::worker::dispatch::observation::DispatchObservation::Wait(wait) => {
                Some(wait)
            }
            _ => None,
        })
        .collect();
    assert_eq!(waits.len(), 1);
    assert_eq!(packet.wait_sequence, waits[0].sequence);
    assert_eq!(packet.wait_spin_ticks, 19);
    assert_eq!(waits[0].wake_qpc, Some(target));
    assert_eq!(waits[0].physical_target_qpc, target);
}

#[test]
fn interrupted_wait_is_not_attributed_to_later_dispatch() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.attach_executor_for_test().expect("attach executor");
    let old_target = harness.qpc_now_for_test().expect("old target");
    harness.runtime.pending_wait_observation = crate::engine::worker::dispatch::record_wait_attempt(
        Some(&harness.observer),
        Some(sky_dispatch_win32::wait::WaitResult::interrupted()),
        old_target,
        DurationTicks::from_raw(50),
        TimelineTicks::ZERO,
        harness.resources.playback.epoch,
        None,
        &mut harness.local_metrics,
    );
    crate::engine::worker::dispatch::publish_pending_wait_observation(
        &mut harness.runtime,
        Some(&harness.observer),
        &mut harness.local_metrics,
    );
    let target = harness.qpc_now_for_test().expect("new target");
    harness.runtime.pending_wait_observation = crate::engine::worker::dispatch::record_wait_attempt(
        Some(&harness.observer),
        Some(sky_dispatch_win32::wait::WaitResult {
            outcome: sky_dispatch_win32::wait::WaitOutcome::Deadline,
            wake_qpc: Some(target),
            spin_ticks: DurationTicks::from_raw(17),
            timer_return_qpc: Some(target),
            timer_arm_target_qpc: Some(target),
            timer_arm_qpc: Some(target),
            timer_arm_generation: Some(2),
            timer_return_generation: Some(2),
        }),
        target,
        DurationTicks::from_raw(50),
        TimelineTicks::ZERO,
        harness.resources.playback.epoch,
        None,
        &mut harness.local_metrics,
    );
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage")
        .expect("candidate");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, true),
        super::StagedDispatchOutcome::Dispatched
    ));
    let observations: Vec<_> = std::iter::from_fn(|| harness.observer.pop_front()).collect();
    let waits: Vec<_> = observations
        .iter()
        .filter_map(|item| match item {
            crate::engine::worker::dispatch::observation::DispatchObservation::Wait(wait) => {
                Some(wait)
            }
            _ => None,
        })
        .collect();
    let packet = observations
        .iter()
        .find_map(|item| match item {
            crate::engine::worker::dispatch::observation::DispatchObservation::Down(packet) => {
                Some(packet)
            }
            _ => None,
        })
        .expect("packet");
    assert_eq!(waits.len(), 2);
    assert_eq!(
        waits[0].outcome,
        sky_dispatch_win32::wait::WaitOutcome::Interrupted
    );
    assert_eq!(waits[0].wake_qpc, None);
    assert_ne!(waits[0].sequence, packet.wait_sequence);
    assert_eq!(waits[1].sequence, packet.wait_sequence);
    assert_eq!(waits[1].wake_qpc, Some(target));
    assert_eq!(packet.wait_spin_ticks, 17);
    assert_eq!(harness.local_metrics.spin_observation_expected_samples, 2);
    assert_eq!(harness.local_metrics.spin_observation_missing_samples, 0);
    assert_eq!(harness.local_metrics.spin_observation_dropped_samples, 0);
}
#[test]
fn final_control_rejects_staged_down_and_up_before_send() {
    let mut down = ProductionDispatchTestHarness::new_down_only();
    let down_packets = down.configure_mock_packet_sender_for_test(&[0, 0]);
    down.attach_executor_for_test().expect("attach executor");
    let chord = down.current_executor_chord_for_test();
    let candidate = down
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage Down")
        .expect("Down candidate");
    down.quit_requested.store(true, Ordering::Release);
    assert!(matches!(
        down.dispatch_staged_candidate_for_test(candidate, true),
        super::StagedDispatchOutcome::Replan
    ));
    assert!(down_packets.lock().expect("Down packets").is_empty());

    let mut up = ProductionDispatchTestHarness::new_down_only();
    let up_packets = up.configure_mock_packet_sender_for_test(&[0, 0]);
    up.attach_executor_for_test().expect("attach executor");
    let chord = up.current_executor_chord_for_test();
    let first = up
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage first")
        .expect("first Down");
    assert!(matches!(
        up.dispatch_staged_candidate_for_test(first, false),
        super::StagedDispatchOutcome::Dispatched
    ));
    let candidate = up
        .stage_candidate_for_test(1, None, TimelineTicks::from_raw(20_000))
        .expect("stage Up")
        .expect("Up candidate");
    up.quit_requested.store(true, Ordering::Release);
    assert!(matches!(
        up.dispatch_staged_candidate_for_test(candidate, true),
        super::StagedDispatchOutcome::Exit
    ));
    assert_eq!(up_packets.lock().expect("Up packets").len(), 1);
}

#[test]
fn coalesced_up_packet_preserves_each_authored_source_in_consumer() {
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: vec![0x15, 0x16].into(),
            reason: "two-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: vec![0x15].into(),
            reason: "first-up".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 2_000,
            scan_codes: vec![0x16].into(),
            reason: "second-up".into(),
        },
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let first = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage Down")
        .expect("Down");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        super::StagedDispatchOutcome::Dispatched
    ));
    let up = harness
        .stage_candidate_for_test(3, None, TimelineTicks::from_raw(20_000))
        .expect("stage Up")
        .expect("Up");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, true),
        super::StagedDispatchOutcome::Dispatched
    ));
    let packet = match harness.observer.pop_front().expect("packet observation") {
        crate::engine::worker::dispatch::observation::DispatchObservation::Down(packet) => packet,
        other => panic!("unexpected observation: {other:?}"),
    };
    assert_eq!(packet.up_slot_count, 2);
    let mut collector =
        crate::engine::TelemetryCollector::new(crate::engine::TelemetryMode::Ring, 1);
    collector.output.accepted = 1;
    crate::engine::worker::dispatch::attach_up_slot_traces(&mut collector, &packet)
        .expect("consume Up evidence");
    let slots = &collector.output.up_slot_records;
    assert_eq!(slots.len(), 2);
    assert_eq!(
        (slots[0].source_action_index, slots[0].scan_code),
        (1, 0x15)
    );
    assert_eq!(
        (slots[1].source_action_index, slots[1].scan_code),
        (2, 0x16)
    );
    assert_ne!(
        slots[0].authored_offset_ticks,
        slots[1].authored_offset_ticks
    );
    assert_ne!(
        slots[0].floor_displacement_ticks,
        slots[1].floor_displacement_ticks
    );
    for slot in slots {
        let target = packet.epoch_qpc.as_u64() + slot.authored_offset_ticks;
        let floor = (packet.epoch_qpc.as_u64() + slot.release_floor_offset_ticks)
            .max(target)
            .max(packet.physical_timing_window.packet_not_before_qpc.as_u64());
        assert_eq!(slot.floor_displacement_ticks, floor - target);
        assert_eq!(
            slot.residual_ticks,
            slot.pre_call_qpc_ticks
                .map(|start| i64::try_from(i128::from(start) - i128::from(floor))
                    .expect("residual range"))
        );
    }
}
