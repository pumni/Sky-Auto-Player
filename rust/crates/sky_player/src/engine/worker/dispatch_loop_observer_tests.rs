use super::support::{
    DurationTicks, Ordering, PacketRetryReason, ProductionDispatchTestHarness, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus, TimelineTicks,
};

#[test]
fn complete_transport_with_sender_timing_error_is_reconciled_by_terminal_cleanup() {
    use sky_dispatch_win32::clock::QpcError;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    let started = harness
        .resources
        .clock
        .now()
        .expect("mock sender start QPC");
    harness
        .resources
        .backend
        .set_packet_emitter(move |packet| SendTransactionOutcome {
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
                started_ticks: Some(started),
                completed_ticks: Some(started),
                timing_error: Some(QpcError::CounterUnavailable),
            },
        });
    harness.attach_executor_for_test().expect("attach executor");

    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("some candidate");

    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);

    assert!(matches!(outcome, super::super::StagedDispatchOutcome::Exit));
    assert!(
        harness
            .runtime
            .attempted_dispatch_anomaly
            .detail()
            .is_some_and(|record| record.contains("timing_error=Some(CounterUnavailable)"))
    );
    assert_eq!(harness.local_metrics.precision_packet_attempts, 1);
    assert_eq!(harness.local_metrics.pre_call_qpc_unavailable_attempts, 0);
    assert_eq!(
        harness
            .local_metrics
            .sendinput_completion_qpc_unavailable_attempts,
        0
    );
    assert_eq!(harness.local_metrics.sendinput_envelope_samples, 0);
    assert_eq!(harness.local_metrics.authored_lateness_samples, 0);
    assert_eq!(harness.local_metrics.residual_lateness_samples, 0);
    assert_eq!(harness.local_metrics.hol_delay_lower_bound_samples, 0);
    assert_eq!(harness.local_metrics.physical_floor_displacement_samples, 0);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert!(harness.runtime.last_physical_send_completion_qpc.is_none());
    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.active_mask, 0);
    assert_eq!(evidence.possibly_active_mask, 0);
    assert_eq!(evidence.failed_release_mask, 0);
    assert_eq!(evidence.release_obligation_mask, 0);
    assert!(
        !cleanup
            .lock()
            .expect("resumable cleanup capture")
            .is_empty()
    );
}
#[test]
fn complete_status_without_completion_qpc_is_reconciled_without_fabricating_time() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("mock sender start QPC");
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
                started_ticks: Some(started),
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

    assert!(matches!(outcome, super::super::StagedDispatchOutcome::Exit));
    assert!(
        harness
            .runtime
            .attempted_dispatch_anomaly
            .detail()
            .is_some_and(|error| error.contains("completion_qpc=unavailable"))
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.local_metrics.precision_packet_attempts, 1);
    assert_eq!(harness.local_metrics.pre_call_qpc_unavailable_attempts, 0);
    assert_eq!(
        harness
            .local_metrics
            .sendinput_completion_qpc_unavailable_attempts,
        1
    );
    assert_eq!(harness.local_metrics.transport_anomalies, 1);
    assert_eq!(harness.local_metrics.authored_lateness_samples, 0);
    assert_eq!(harness.local_metrics.residual_lateness_samples, 0);
    assert_eq!(harness.local_metrics.hol_delay_lower_bound_samples, 0);
    assert_eq!(harness.local_metrics.sendinput_envelope_samples, 0);
    assert_eq!(harness.local_metrics.physical_floor_displacement_samples, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.release_obligation_mask, 0);
    assert!(!cleanup.lock().expect("cleanup capture").is_empty());
}

#[test]
fn complete_status_without_pre_call_qpc_is_reconciled_without_fabricating_time() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let clock = harness.resources.clock;
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.resources.backend.set_packet_emitter(move |packet| {
        let completion = clock.now().expect("mock sender completion QPC");
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
                completed_ticks: Some(completion),
                timing_error: None,
            },
        }
    });
    harness.attach_executor_for_test().expect("attach executor");

    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        super::super::StagedDispatchOutcome::Exit
    ));
    assert!(
        harness
            .runtime
            .attempted_dispatch_anomaly
            .detail()
            .is_some_and(|error| error.contains("pre_call_qpc=unavailable"))
    );
    assert_eq!(harness.local_metrics.precision_packet_attempts, 1);
    assert_eq!(harness.local_metrics.pre_call_qpc_unavailable_attempts, 1);
    assert_eq!(
        harness
            .local_metrics
            .sendinput_completion_qpc_unavailable_attempts,
        0
    );
    assert_eq!(harness.local_metrics.transport_anomalies, 1);
    assert_eq!(harness.local_metrics.authored_lateness_samples, 0);
    assert_eq!(harness.local_metrics.residual_lateness_samples, 0);
    assert_eq!(harness.local_metrics.hol_delay_lower_bound_samples, 0);
    assert_eq!(harness.local_metrics.sendinput_envelope_samples, 0);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.release_obligation_mask, 0);
    assert!(!cleanup.lock().expect("cleanup capture").is_empty());
}

#[test]
fn complete_status_with_invalid_qpc_order_is_reconciled_without_clock_substitution() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let clock = harness.resources.clock;
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("mock sender start QPC");
        let completed = sky_dispatch_win32::clock::QpcTicks::from_raw(
            started.as_u64().checked_sub(1).expect("nonzero QPC"),
        );
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
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });
    harness.attach_executor_for_test().expect("attach executor");

    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        super::super::StagedDispatchOutcome::Exit
    ));
    assert!(
        harness
            .runtime
            .attempted_dispatch_anomaly
            .detail()
            .is_some_and(|error| error.contains("completion_qpc="))
    );
    assert_eq!(harness.local_metrics.precision_packet_attempts, 1);
    assert_eq!(harness.local_metrics.pre_call_qpc_unavailable_attempts, 0);
    assert_eq!(
        harness
            .local_metrics
            .sendinput_completion_qpc_unavailable_attempts,
        0
    );
    assert_eq!(harness.local_metrics.transport_anomalies, 1);
    assert_eq!(harness.local_metrics.authored_lateness_samples, 0);
    assert_eq!(harness.local_metrics.residual_lateness_samples, 0);
    assert_eq!(harness.local_metrics.hol_delay_lower_bound_samples, 0);
    assert_eq!(harness.local_metrics.sendinput_envelope_samples, 0);
    assert_eq!(harness.local_metrics.physical_floor_displacement_samples, 0);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.release_obligation_mask, 0);
    assert!(!cleanup.lock().expect("cleanup capture").is_empty());
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
    assert!(matches!(
        outcome,
        super::super::StagedDispatchOutcome::Dispatched
    ));

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
        .stage_candidate_with_observer_for_test(1, None, TimelineTicks::from_raw(20_000), true)
        .expect("stage Up")
        .expect("Up candidate");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, true),
        super::super::StagedDispatchOutcome::Dispatched
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
        super::super::StagedDispatchOutcome::Dispatched
    ));
    let target = harness.qpc_now_for_test().expect("wait target");
    let mixed_offset = super::support::qpc_offset(
        &harness,
        crate::engine::test_support::dispatch_harness::DISPATCH_EFFECTIVE_HOLD_US,
    );
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
        mixed_offset,
        harness.resources.playback.epoch,
        None,
        &mut harness.local_metrics,
    );
    let chord = harness.current_executor_chord_for_test();
    let mixed = harness
        .stage_candidate_with_observer_for_test(1, chord, mixed_offset, true)
        .expect("stage mixed")
        .expect("mixed");
    assert_ne!(mixed.packet_masks.up_mask, 0);
    assert_ne!(mixed.packet_masks.down_mask, 0);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(mixed, true),
        super::super::StagedDispatchOutcome::Dispatched
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
        super::super::StagedDispatchOutcome::Dispatched
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
        super::super::StagedDispatchOutcome::Replan
    ));
    assert!(down_packets.lock().expect("Down packets").is_empty());
    assert_eq!(down.local_metrics.precision_packet_attempts, 0);

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
        super::super::StagedDispatchOutcome::Dispatched
    ));
    let candidate = up
        .stage_candidate_with_observer_for_test(1, None, TimelineTicks::from_raw(20_000), true)
        .expect("stage Up")
        .expect("Up candidate");
    up.quit_requested.store(true, Ordering::Release);
    assert!(matches!(
        up.dispatch_staged_candidate_for_test(candidate, true),
        super::super::StagedDispatchOutcome::Exit
    ));
    assert_eq!(up_packets.lock().expect("Up packets").len(), 1);
    assert_eq!(up.local_metrics.precision_packet_attempts, 1);
}

#[test]
fn coalesced_up_packet_preserves_each_authored_source_in_consumer() {
    let chords = [
        super::support::test_chord(1, 0, 0x15),
        super::support::test_chord(2, 1_000, 0x16),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
    harness.attach_executor_for_test().expect("attach executor");
    let epoch = harness.resources.playback.epoch;
    let chord = harness.current_executor_chord_for_test();
    let first = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage first Down")
        .expect("first Down");
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        super::super::StagedDispatchOutcome::Dispatched
    ));
    let second_offset = super::support::qpc_offset(&harness, 1_000);
    let second_qpc = super::support::target_at(&harness, epoch, 1_000);
    let second_chord = harness.current_executor_chord_for_test();
    let second = harness
        .stage_candidate_for_test(0, second_chord, second_offset)
        .expect("stage second Down")
        .expect("second Down");
    harness.set_mock_sender_start_qpc_for_test(second_qpc);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(second, false),
        super::super::StagedDispatchOutcome::Dispatched
    ));
    let both_ups_due = TimelineTicks::from_raw(
        harness
            .resources
            .clock
            .duration_from_us(22_000)
            .expect("coalesced Up due time")
            .as_u64(),
    );
    let up = harness
        .stage_candidate_with_observer_for_test(3, None, both_ups_due, true)
        .expect("stage Up")
        .expect("Up");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, true),
        super::super::StagedDispatchOutcome::Dispatched
    ));
    let packet = match harness.observer.pop_front().expect("packet observation") {
        crate::engine::worker::dispatch::observation::DispatchObservation::Down(packet) => packet,
        other => panic!("unexpected observation: {other:?}"),
    };
    assert_eq!(packet.up_slot_count, 2);
    let mut collector =
        crate::engine::TelemetryCollector::new(crate::engine::TelemetryMode::Ring, 1);
    collector.output.accepted = 1;
    crate::engine::worker::dispatch::observer_trace::attach_up_slot_traces(&mut collector, &packet)
        .expect("consume Up evidence");
    let slots = &collector.output.up_slot_records;
    assert_eq!(slots.len(), 2);
    let hold_ticks = harness.resources.scheduler.hold_ticks().as_u64();
    let first_up_ticks = hold_ticks;
    let second_up_ticks = second_offset.as_u64() + hold_ticks;
    assert_eq!(
        (
            slots[0].source_action_index,
            slots[0].scan_code,
            slots[0].authored_offset_ticks
        ),
        (1, 0x15, first_up_ticks)
    );
    assert_eq!(
        (
            slots[1].source_action_index,
            slots[1].scan_code,
            slots[1].authored_offset_ticks
        ),
        (2, 0x16, second_up_ticks)
    );
    assert_ne!(
        slots[0].authored_offset_ticks,
        slots[1].authored_offset_ticks
    );
    for slot in slots {
        let target = packet.epoch_qpc.as_u64() + slot.authored_offset_ticks;
        let floor = (packet.epoch_qpc.as_u64() + slot.up_not_before_offset_ticks)
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
    let chords = [sky_dispatch_core::model::OnsetChordInput {
        source_action_index: 0,
        scheduled_us: 0,
        scan_codes: smallvec::smallvec![0x15, 0x16],
    }];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let first = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage Down")
        .expect("Down");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        super::super::StagedDispatchOutcome::Dispatched
    ));
    let both_ups_due = TimelineTicks::from_raw(
        harness
            .resources
            .clock
            .duration_from_us(22_000)
            .expect("coalesced Up due time")
            .as_u64(),
    );
    for index in 0..crate::engine::worker::dispatch::observation::OBSERVATION_QUEUE_CAPACITY {
        harness.observer.push(
            crate::engine::worker::dispatch::observation::DispatchObservation::Down(
                crate::engine::worker::dispatch::observation::DownObservation::test_fixture(
                    u64::try_from(index).unwrap_or(u64::MAX),
                ),
            ),
            &mut harness.local_metrics.observer_dropped_samples,
        );
    }

    let up = harness
        .stage_candidate_with_observer_for_test(3, None, both_ups_due, true)
        .expect("stage coalesced Up")
        .expect("Up");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, true),
        super::super::StagedDispatchOutcome::Dispatched
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
            nominal_up_offset_ticks: TimelineTicks::from_raw(100),
            release_not_before_qpc: sky_dispatch_win32::clock::QpcTicks::from_raw(1_120),
        },
        sky_dispatch_core::executor::UpSlotTiming {
            slot: 1,
            scan_code: 22,
            source_action_index: 6,
            nominal_up_offset_ticks: TimelineTicks::from_raw(110),
            release_not_before_qpc: sky_dispatch_win32::clock::QpcTicks::from_raw(1_145),
        },
    ]);
    observation.epoch_qpc = sky_dispatch_win32::clock::QpcTicks::from_raw(1_000);
    observation.pre_call_qpc = Some(sky_dispatch_win32::clock::QpcTicks::from_raw(1_150));
    observation.sendinput_completion_qpc = sky_dispatch_win32::clock::QpcTicks::from_raw(1_155);
    observation.up_slots = Some(storage);
    observation.up_slot_count = 2;

    let mut telemetry = TelemetryCollector::new(TelemetryMode::Ring, 0);
    telemetry.output.up_slot_records_expected = 2;
    crate::engine::worker::dispatch::observation::record_down_send_telemetry(
        &observation,
        &mut telemetry,
        0,
        false,
        0,
        0,
        None,
        sky_dispatch_win32::clock::QpcTicks::from_raw(1_155),
        false,
        false,
    )
    .expect("record dropped packet trace");

    assert_eq!(telemetry.output.records.len(), 0);
    assert_eq!(telemetry.output.up_slot_records.len(), 0);
    assert_eq!(
        telemetry.output.up_slot_records_dropped_by_telemetry_ring,
        2
    );
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
        false,
        0,
        0,
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
    use crate::engine::WorkerMetricsLocal;
    use crate::engine::worker::dispatch::observation::{
        DispatchObservation, DownObservation, OBSERVATION_QUEUE_CAPACITY,
    };
    use crate::engine::worker::dispatch::observer::PendingObservationQueue;

    let observer = PendingObservationQueue::default();
    let mut metrics = WorkerMetricsLocal::default();
    for index in 0..OBSERVATION_QUEUE_CAPACITY {
        observer.push(
            DispatchObservation::Down(DownObservation::test_fixture(
                u64::try_from(index).expect("queue source index"),
            )),
            &mut metrics.observer_dropped_samples,
        );
    }
    let mut observation = DownObservation::test_fixture(0);
    observation.up_slot_count = 2;
    super::super::publish_down_observation(
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
        metrics.observer_dropped_up_slot_records + metrics.observer_dropped_up_slot_pool_records
    );
}

#[test]
fn absent_observer_counts_up_slots_as_not_captured_by_mode() {
    use crate::engine::WorkerMetricsLocal;

    let mut metrics = WorkerMetricsLocal::default();
    super::super::account_unobserved_up_slots(&mut metrics, 2);

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
