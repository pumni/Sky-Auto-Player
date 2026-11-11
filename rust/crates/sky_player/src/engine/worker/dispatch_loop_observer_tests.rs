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
fn successful_send_keeps_missing_pre_call_unavailable_with_real_completion_qpc() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    let qpc_clock = harness.resources.clock;
    let completion_qpc = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let completion_capture = std::sync::Arc::clone(&completion_qpc);
    harness
        .resources
        .backend
        .disable_pre_call_qpc_sample_for_test();
    harness
        .resources
        .backend
        .set_packet_emitter(move |packet| {
            let completion_qpc = qpc_clock.now().expect("real sender completion QPC");
            completion_capture.store(completion_qpc.as_u64(), Ordering::Release);
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
                    completed_ticks: Some(completion_qpc),
                    timing_error: None,
                },
            }
        });

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, true),
        super::StagedDispatchOutcome::Dispatched
    ));
    let completion_qpc = sky_dispatch_win32::clock::QpcTicks::from_raw(
        completion_qpc.load(Ordering::Acquire),
    );
    let observation = match harness.observer.pop_front().expect("Down observation") {
        crate::engine::worker::dispatch::observation::DispatchObservation::Down(observation) => {
            observation
        }
        other => panic!("unexpected observation: {other:?}"),
    };
    assert_eq!(
        observation.trace.result_status,
        SendTransactionStatus::Complete
    );
    assert_eq!(observation.pre_call_qpc, None);
    assert_eq!(observation.sendinput_completion_qpc, completion_qpc);

    let mut telemetry = crate::engine::TelemetryCollector::new(
        crate::engine::TelemetryMode::Ring,
        1,
    );
    crate::engine::worker::dispatch::observation::record_down_send_telemetry(
        &observation,
        &mut telemetry,
        0,
        0,
        false,
        0,
        0,
        None,
        Some(TimelineTicks::from_raw(completion_qpc.as_u64())),
        None,
        completion_qpc,
        false,
        false,
    )
    .expect("serialize successful sender evidence");
    let record = telemetry.output.records.back().expect("trace record");
    assert!(!record.pre_call_qpc_available);
    assert_eq!(record.pre_call_qpc_ticks, 0);
    assert_eq!(record.pre_call_qpc_provenance, crate::engine::TimestampProvenance::Unavailable);
    assert!(record.sendinput_completion_qpc_available);
    assert_eq!(record.sendinput_completion_qpc_ticks, completion_qpc.as_u64());
    assert_eq!(record.sendinput_completion_qpc_provenance, crate::engine::TimestampProvenance::Measured);
    assert!(!record.authored_lateness_available);
    assert!(!record.sendinput_envelope_available);
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
    let one_millisecond_ticks = harness
        .resources
        .clock
        .duration_from_us(1_000)
        .expect("one millisecond QPC duration")
        .as_u64();
    assert_eq!(
        (
            slots[0].source_action_index,
            slots[0].scan_code,
            slots[0].authored_offset_ticks
        ),
        (1, 0x15, one_millisecond_ticks)
    );
    assert_eq!(
        (
            slots[1].source_action_index,
            slots[1].scan_code,
            slots[1].authored_offset_ticks
        ),
        (2, 0x16, one_millisecond_ticks * 2)
    );
    assert_ne!(
        slots[0].authored_offset_ticks,
        slots[1].authored_offset_ticks
    );
    assert_ne!(
        slots[0].floor_displacement_ticks,
        slots[1].floor_displacement_ticks
    );
    assert_eq!(
        slots[0].floor_displacement_ticks - slots[1].floor_displacement_ticks,
        one_millisecond_ticks,
        "the coalesced Up sidecar retains the exact one millisecond authored separation"
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

#[test]
fn coalesced_up_sidecar_loss_is_counted_when_parent_observation_is_dropped() {
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
    for index in 0..crate::engine::worker::dispatch::observation::OBSERVATION_QUEUE_CAPACITY {
        harness.observer.push(
            crate::engine::worker::dispatch::observation::DispatchObservation::StaleMetadata(
                crate::engine::worker::dispatch::observation::StaleMetadataObservation {
                    source_action_index: u32::try_from(index).unwrap_or(u32::MAX),
                    effective_scheduled_ticks: TimelineTicks::from_raw(index as u64),
                    suppressed_intent_count: 1,
                },
            ),
            &mut harness.local_metrics.observer_dropped_samples,
        );
    }

    let up = harness
        .stage_candidate_for_test(3, None, TimelineTicks::from_raw(20_000))
        .expect("stage coalesced Up")
        .expect("Up");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, true),
        super::StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(harness.local_metrics.observer_dropped_samples, 1);
    assert_eq!(harness.local_metrics.observer_dropped_up_slot_records, 2);
    assert_eq!(harness.local_metrics.up_slot_records_expected, 2);
    assert_eq!(
        harness.local_metrics.up_slot_records_expected,
        harness.local_metrics.observer_dropped_up_slot_records
            + harness.local_metrics.observer_dropped_up_slot_pool_records
    );
}

#[test]
fn telemetry_ring_drop_accounts_each_coalesced_up_slot() {
    use crate::engine::worker::dispatch::observation::DownObservation;
    use crate::engine::{TelemetryCollector, TelemetryMode};

    let queue = crate::engine::worker::dispatch::observer::PendingObservationQueue::default();
    let mut observation = DownObservation::test_fixture(1);
    let mut storage = queue.acquire_up_slots().expect("bounded Up-slot pool");
    storage.slots.as_mut().expect("owned Up-slot storage")[..2].copy_from_slice(&[
        sky_dispatch_core::executor::UpSlotTiming {
            slot: 0,
            scan_code: 21,
            source_action_index: 5,
            authored_up_offset_ticks: TimelineTicks::from_raw(100),
            release_not_before_ticks: TimelineTicks::from_raw(120),
        },
        sky_dispatch_core::executor::UpSlotTiming {
            slot: 1,
            scan_code: 22,
            source_action_index: 6,
            authored_up_offset_ticks: TimelineTicks::from_raw(110),
            release_not_before_ticks: TimelineTicks::from_raw(145),
        },
    ]);
    observation.epoch_qpc = sky_dispatch_win32::clock::QpcTicks::from_raw(1_000);
    observation.pre_call_qpc = Some(sky_dispatch_win32::clock::QpcTicks::from_raw(1_150));
    observation.sendinput_completion_qpc =
        sky_dispatch_win32::clock::QpcTicks::from_raw(1_155);
    observation.up_slots = Some(storage);
    observation.up_slot_count = 2;

    let mut telemetry = TelemetryCollector::new(TelemetryMode::Ring, 0);
    telemetry.output.up_slot_records_expected = 2;
    crate::engine::worker::dispatch::observation::record_down_send_telemetry(
        &observation,
        &mut telemetry,
        0,
        0,
        false,
        0,
        0,
        None,
        None,
        None,
        sky_dispatch_win32::clock::QpcTicks::from_raw(1_155),
        false,
        false,
    )
    .expect("record dropped packet trace");

    assert_eq!(telemetry.output.records.len(), 0);
    assert_eq!(telemetry.output.up_slot_records.len(), 0);
    assert_eq!(telemetry.output.up_slot_records_dropped_by_telemetry_ring, 2);
    assert_eq!(telemetry.output.up_slot_records_dropped, 2);
    assert_eq!(
        telemetry.output.up_slot_records_expected,
        telemetry.output.up_slot_records.len() as u64
            + telemetry.output.up_slot_records_dropped
            + telemetry.output.up_slot_records_not_captured_by_mode
    );

    let mut disabled = TelemetryCollector::new(TelemetryMode::Off, 0);
    disabled.output.up_slot_records_expected = 2;
    crate::engine::worker::dispatch::observation::record_down_send_telemetry(
        &observation,
        &mut disabled,
        0,
        0,
        false,
        0,
        0,
        None,
        None,
        None,
        sky_dispatch_win32::clock::QpcTicks::from_raw(1_155),
        false,
        false,
    )
    .expect("account disabled Up-slot capture");
    assert_eq!(disabled.output.up_slot_capture_mode, "off");
    assert_eq!(disabled.output.up_slot_records_dropped, 0);
    assert_eq!(disabled.output.up_slot_records_not_captured_by_mode, 2);
    assert_eq!(
        disabled.output.up_slot_records_expected,
        disabled.output.up_slot_records.len() as u64
            + disabled.output.up_slot_records_dropped
            + disabled.output.up_slot_records_not_captured_by_mode
    );
}

#[test]
fn missing_pool_storage_is_counted_once_when_observer_queue_is_also_full() {
    use crate::engine::worker::dispatch::observation::{
        DispatchObservation, DownObservation, OBSERVATION_QUEUE_CAPACITY,
    };
    use crate::engine::worker::dispatch::observer::PendingObservationQueue;
    use crate::engine::WorkerMetricsLocal;

    let observer = PendingObservationQueue::default();
    let mut metrics = WorkerMetricsLocal::default();
    for index in 0..OBSERVATION_QUEUE_CAPACITY {
        observer.push(
            DispatchObservation::StaleMetadata(
                crate::engine::worker::dispatch::observation::StaleMetadataObservation {
                    source_action_index: u32::try_from(index).expect("queue source index"),
                    effective_scheduled_ticks: TimelineTicks::from_raw(index as u64),
                    suppressed_intent_count: 1,
                },
            ),
            &mut metrics.observer_dropped_samples,
        );
    }
    let mut observation = DownObservation::test_fixture(0);
    observation.up_slot_count = 2;
    super::publish_down_observation(
        &observer,
        DispatchObservation::Down(observation),
        2,
        &mut metrics,
    );

    assert_eq!(metrics.observer_dropped_samples, 1);
    assert_eq!(metrics.up_slot_records_expected, 2);
    assert_eq!(metrics.observer_dropped_up_slot_records, 0);
    assert_eq!(metrics.observer_dropped_up_slot_pool_records, 2);
    assert_eq!(
        metrics.up_slot_records_expected,
        metrics.observer_dropped_up_slot_records
            + metrics.observer_dropped_up_slot_pool_records
    );
}

#[test]
fn absent_observer_counts_up_slots_as_not_captured_by_mode() {
    use crate::engine::WorkerMetricsLocal;

    let mut metrics = WorkerMetricsLocal::default();
    super::account_unobserved_up_slots(&mut metrics, 2);

    assert_eq!(metrics.up_slot_records_expected, 2);
    assert_eq!(metrics.up_slot_records_not_captured_by_mode, 2);
    assert_eq!(metrics.observer_dropped_up_slot_records, 0);
    assert_eq!(metrics.observer_dropped_up_slot_pool_records, 0);
    assert_eq!(
        metrics.up_slot_records_expected,
        metrics.up_slot_records_not_captured_by_mode
            + metrics.observer_dropped_up_slot_records
            + metrics.observer_dropped_up_slot_pool_records
    );
}
