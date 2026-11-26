use super::support::ProductionDispatchTestHarness;
use sky_dispatch_core::model::OnsetChordInput;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct ForegroundOverrideReset;

impl Drop for ForegroundOverrideReset {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
    }
}

pub(super) fn timing_chord(index: u32, scheduled_us: u64) -> OnsetChordInput {
    OnsetChordInput {
        source_action_index: index,
        scheduled_us,
        scan_codes: smallvec::smallvec![0x15],
    }
}

pub(super) fn timing_harness_without_focus(
    chords: &[OnsetChordInput],
    sender_latencies_us: &[u64],
) -> (
    ProductionDispatchTestHarness,
    crate::engine::test_support::PacketCapture,
    QpcTicks,
) {
    let mut harness = ProductionDispatchTestHarness::create_harness(chords);
    let epoch = harness.qpc_now_for_test().expect("test QPC epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback clock");
    let packets = harness.configure_mock_packet_sender_for_test(sender_latencies_us);
    harness
        .attach_executor_for_test()
        .expect("production executor invariant");
    (harness, packets, epoch)
}

pub(super) fn stage_from_worker_outer_evaluation(
    harness: &mut ProductionDispatchTestHarness,
    now_qpc: QpcTicks,
) -> (crate::engine::worker::StagedCandidate, u64) {
    super::super::reset_scheduler_evaluation_count_for_test();
    let epoch = harness.resources.playback.epoch;
    let (action, _scheduler_wait_target_qpc, action_to_stage) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            now_qpc,
            epoch,
        )
        .expect("worker scheduler evaluation");
    assert!(matches!(action, super::super::SchedulerAction::Send { .. }));
    let send_action = action_to_stage.expect("due scheduler action stages a packet");
    let candidate = super::super::stage_scheduler_send_candidate(
        send_action,
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage worker packet")
    .expect("scheduler candidate is stageable");
    let count = super::super::scheduler_evaluation_count_for_test();
    (candidate, count)
}

pub(super) fn elapsed_us(
    clock: sky_dispatch_win32::clock::QpcClock,
    start: QpcTicks,
    end: QpcTicks,
) -> u64 {
    let ticks = end
        .checked_duration_since(start)
        .expect("trace timestamps are monotonic");
    clock
        .duration_to_us(sky_dispatch_core::time::DurationTicks::from_raw(
            ticks.as_u64(),
        ))
        .expect("trace duration converts to microseconds")
}

#[test]
fn immediate_resume_worker_decision_sends_due_down_before_retired_cleanup_floor() {
    let chords = [timing_chord(900, 0), timing_chord(902, 50_000)];
    let (mut harness, packets, _) = timing_harness_without_focus(&chords, &[0]);
    let cleanup_capture = harness.configure_mock_cleanup_emitter_for_test();
    let qpc_clock = harness.resources.clock;
    let current_qpc = harness.qpc_now_for_test().expect("current test QPC");
    let epoch_offset = qpc_clock
        .duration_from_us(100_000)
        .expect("place playback epoch before due Down");
    let epoch = QpcTicks::from_raw(
        current_qpc
            .as_u64()
            .checked_sub(epoch_offset.as_u64())
            .expect("QPC has room for test epoch offset"),
    );
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor controllable worker playback clock");

    let first_down_now = harness.qpc_now_for_test().expect("first Down QPC");
    let first_down = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage active first Down")
        .expect("first Down candidate");
    assert_eq!(first_down.source_action_index(), 900);
    harness.set_mock_sender_start_qpc_for_test(first_down_now);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first_down, false),
        super::super::StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(packets.lock().expect("first Down capture").len(), 1);

    let pause_request_qpc = harness.qpc_now_for_test().expect("Pause request QPC");
    let authored_down_due = epoch
        .checked_add_duration(
            qpc_clock
                .duration_from_us(50_000)
                .expect("same-key Down authored offset"),
        )
        .expect("same-key Down deadline");
    assert!(
        pause_request_qpc >= authored_down_due,
        "Pause must start after the same-key authored Down is due"
    );
    harness
        .commit_pause_for_test(
            sky_dispatch_core::clock::PauseReason::Manual,
            pause_request_qpc,
        )
        .expect("commit manual Pause with the key active");
    harness
        .apply_scheduler_resumable_lifecycle_transition_for_test()
        .expect("verified active-key cleanup before Resume");
    assert_eq!(
        cleanup_capture.lock().expect("cleanup capture").as_slice(),
        &[(vec![0x15], true)],
        "Pause cleanup must emit Up for the active key before Resume"
    );
    assert_eq!(harness.resources.backend.active_mask, 0);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.generation_accounting_for_test().cancelled, 1);

    let cleanup_ack_qpc = harness
        .qpc_now_for_test()
        .expect("QPC after verified cleanup acknowledgment");
    let resume_commit_qpc = QpcTicks::from_raw(
        cleanup_ack_qpc
            .as_u64()
            .checked_add(1)
            .expect("Resume QPC tick increment"),
    );
    harness
        .commit_resume_for_test(
            sky_dispatch_core::clock::PauseReason::Manual,
            resume_commit_qpc,
        )
        .expect("commit immediate Resume at controlled QPC");
    assert!(!harness.is_paused_for_test());

    let retired_floor_ticks = qpc_clock
        .duration_from_us(17_167)
        .expect("historical verified-cleanup floor duration");
    let retired_floor_deadline = cleanup_ack_qpc
        .checked_add_duration(retired_floor_ticks)
        .expect("historical verified-cleanup floor deadline");
    assert!(
        resume_commit_qpc < retired_floor_deadline,
        "the controlled Resume QPC must remain before the historical cleanup floor"
    );

    let (first_action, _, action_to_stage) = super::super::evaluate_scheduler_action_for_staging(
        &harness.resources.scheduler,
        resume_commit_qpc,
        harness.resources.playback.epoch,
    )
    .expect("first worker scheduler decision after Resume");
    assert!(matches!(
        first_action,
        super::super::SchedulerAction::Send {
            up_mask: 0,
            down_chord: Some(chord),
            timing_window,
            ..
        } if chord.source_action_index == 902
            && timing_window.packet_not_before_qpc <= resume_commit_qpc
    ));
    assert!(matches!(
        action_to_stage,
        Some(super::super::SchedulerAction::Send {
            up_mask: 0,
            down_chord: Some(chord),
            ..
        }) if chord.source_action_index == 902
    ));
}

fn wait_for_deadline(
    harness: &mut ProductionDispatchTestHarness,
    deadline_qpc: QpcTicks,
) -> super::super::WaitBoundary {
    super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
        deadline: super::super::WaitDeadline {
            physical_target_qpc: Some(deadline_qpc),
            spin_threshold_ticks: DurationTicks::ZERO,
            qpc_clock: harness.resources.clock,
        },
        signals: super::super::WaitSignals {
            waiter: &harness.resources.waiter,
            interrupt: &harness.interrupt,
        },
        mutable: super::super::WaitMutable {
            local_metrics: &mut harness.local_metrics,
            terminal_error: &mut harness.runtime.terminal_error,
        },
    })
}

fn prepare_due_mixed_fallback_candidate() -> (
    ProductionDispatchTestHarness,
    crate::engine::test_support::PacketCapture,
    crate::engine::worker::StagedCandidate,
) {
    let chords = [
        OnsetChordInput {
            source_action_index: 820,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
        },
        OnsetChordInput {
            source_action_index: 822,
            scheduled_us: 20_000,
            scan_codes: smallvec::smallvec![0x16],
        },
    ];
    let (mut harness, packets, epoch) = timing_harness_without_focus(&chords, &[0, 0]);
    harness.configure_focus_owner_for_test(1, 101);
    harness.config.wait.supervisor_lease_timeout_us = 1_000_000;
    harness.timing.lease_timeout_ticks = harness
        .qpc_duration_from_us_for_test(1_000_000)
        .expect("fresh supervisor lease timeout");
    harness.supervisor_expired.reset(epoch);

    let first_down_qpc = harness.qpc_now_for_test().expect("first Down QPC");
    let first_down = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            sky_dispatch_core::time::TimelineTicks::ZERO,
        )
        .expect("stage initial Down")
        .expect("scheduler-produced initial Down");
    harness.set_mock_sender_start_qpc_for_test(first_down_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first_down, false),
        super::super::StagedDispatchOutcome::Dispatched
    );
    let down_completion = packets
        .lock()
        .expect("initial Down packet capture")
        .first()
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("initial Down completion evidence");

    let up_authored_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            harness
                .qpc_duration_from_us_for_test(20_000)
                .expect("Up authored duration")
                .as_u64(),
        ))
        .expect("Up authored target");
    let hold_floor = down_completion
        .checked_add_duration(DurationTicks::from_raw(
            harness
                .qpc_duration_from_us_for_test(17_167)
                .expect("H floor duration")
                .as_u64(),
        ))
        .expect("Up H floor");
    while harness.qpc_now_for_test().expect("wait for Up floor")
        < up_authored_target.max(hold_floor)
    {
        std::thread::sleep(Duration::from_micros(100));
    }

    let mixed_now = harness.qpc_now_for_test().expect("due Mixed QPC");
    let elapsed = mixed_now
        .checked_duration_since(epoch)
        .expect("Mixed QPC follows epoch");
    let mixed = harness
        .stage_candidate_with_observer_for_test(
            1,
            harness.current_executor_chord_for_test(),
            sky_dispatch_core::time::TimelineTicks::from_raw(elapsed.as_u64()),
            true,
        )
        .expect("stage scheduler-due Mixed packet")
        .expect("scheduler-produced Mixed candidate");
    assert_eq!(
        mixed.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(1, 2)
    );
    assert!(mixed.up_only_timing_window().is_some());
    packets.lock().expect("packet capture").clear();
    (harness, packets, mixed)
}

fn install_held_modifier_and_gate_trace(
    harness: &mut ProductionDispatchTestHarness,
    markers: Arc<Mutex<Vec<super::super::FinalAdmissionGateMarker>>>,
) {
    harness.set_modifier_key_state_query_for_test(|_| i16::MIN);
    harness.runtime.final_admission_gate_hook_for_test = Some(Arc::new(move |marker| {
        markers
            .lock()
            .expect("final admission marker lock")
            .push(marker);
    }));
}

fn assert_fresh_up_admission_was_used(harness: &ProductionDispatchTestHarness) {
    assert!(harness.config.focus.require_focus);
    assert!(
        harness
            .resources
            .backend
            .session_release_owner_proof()
            .is_some()
    );
    assert_eq!(harness.runtime.final_lease_qpc_sample_count_for_test, 1);
    assert!(harness.target_proof_generation_revalidations_for_test() > 0);
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 1);
    assert_eq!(
        sky_dispatch_win32::focus::process_identity_check_count_for_test(),
        1
    );
    assert_eq!(
        sky_dispatch_win32::focus::process_liveness_check_count_for_test(),
        1
    );
}

fn up_only_send_action(
    action: Option<super::super::SchedulerAction>,
) -> super::super::SchedulerAction {
    let Some(action @ super::super::SchedulerAction::Send { .. }) = action else {
        panic!("scheduler must stage an Up-only packet");
    };
    assert!(matches!(
        action,
        super::super::SchedulerAction::Send {
            up_mask: 1,
            down_chord: None,
            ..
        }
    ));
    action
}

fn stage_scheduler_action(
    harness: &mut ProductionDispatchTestHarness,
    action: super::super::SchedulerAction,
) -> crate::engine::worker::StagedCandidate {
    super::super::stage_scheduler_send_candidate(
        action,
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage scheduler-selected packet")
    .expect("scheduler candidate is stageable")
}

#[test]
fn scheduler_selects_independent_up_before_down_and_replans_after_interrupt() {
    let chords = [
        timing_chord(810, 0),
        OnsetChordInput {
            source_action_index: 812,
            scheduled_us: 600_000,
            scan_codes: smallvec::smallvec![0x16],
        },
    ];
    let (mut harness, packets, epoch) = timing_harness_without_focus(&chords, &[0, 0]);
    let initial_now = harness.qpc_now_for_test().expect("initial Down QPC");
    let initial = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            sky_dispatch_core::time::TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    harness.set_mock_sender_start_qpc_for_test(initial_now);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(initial, false),
        super::super::StagedDispatchOutcome::Dispatched
    );
    let down_completion = packets
        .lock()
        .expect("initial Down packet capture")
        .first()
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("initial Down completion evidence");

    let now = harness.qpc_now_for_test().expect("pre-wait QPC");
    let up_hold_floor = down_completion
        .checked_add_duration(DurationTicks::from_raw(
            harness
                .qpc_duration_from_us_for_test(
                    crate::engine::test_support::dispatch_harness::DISPATCH_EFFECTIVE_HOLD_US,
                )
                .expect("Up hold floor conversion")
                .as_u64(),
        ))
        .expect("Up hold floor QPC");
    let down_authored_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            harness
                .qpc_duration_from_us_for_test(600_000)
                .expect("Down floor conversion")
                .as_u64(),
        ))
        .expect("Down target QPC");
    assert!(now < up_hold_floor);

    let (action, up_deadline, action_to_stage) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            now,
            harness.resources.playback.epoch,
        )
        .expect("production scheduler evaluation before Up deadline");
    assert!(matches!(
        action,
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == up_deadline
    ));
    assert_eq!(up_deadline, up_hold_floor);
    assert!(up_deadline < down_authored_target);
    let first_candidate =
        stage_scheduler_action(&mut harness, up_only_send_action(action_to_stage));
    assert_eq!(
        first_candidate.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert_eq!(
        first_candidate.timing_window.packet_not_before_qpc,
        up_deadline
    );
    assert!(
        first_candidate.up_only_timing_window().is_none(),
        "a scheduler-native UpOnly candidate keeps its authored timing window"
    );
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&first_candidate),
        super::super::StagedDispatchOutcome::Dispatched
    );

    // Exercise the real interrupt event and production wait boundary. The
    // worker drops this candidate and re-evaluates the scheduler on wake.
    harness.interrupt.try_take();
    assert!(harness.interrupt.signal());
    let interrupted = wait_for_deadline(&mut harness, up_deadline);
    assert!(matches!(
        interrupted,
        super::super::WaitBoundary::Interrupted(..)
    ));
    assert_eq!(harness.local_metrics.wait_interrupted_count, 1);
    harness.interrupt.try_take();
    drop(first_candidate);
    assert_eq!(
        packets.lock().expect("packet capture").len(),
        1,
        "an interrupt before the Up floor must not send the Up"
    );

    let replanned_now = harness.qpc_now_for_test().expect("QPC after interrupt");
    assert!(replanned_now < up_deadline);
    let (replanned_action, replanned_deadline, replanned_to_stage) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            replanned_now,
            harness.resources.playback.epoch,
        )
        .expect("scheduler re-evaluation after interrupt");
    assert!(matches!(
        replanned_action,
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == replanned_deadline
    ));
    assert_eq!(replanned_deadline, up_deadline);
    let replanned_candidate =
        stage_scheduler_action(&mut harness, up_only_send_action(replanned_to_stage));
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&replanned_candidate),
        super::super::StagedDispatchOutcome::Dispatched
    );
    let due = wait_for_deadline(&mut harness, replanned_deadline);
    assert!(matches!(
        due,
        super::super::WaitBoundary::Due { target_qpc, .. } if target_qpc == replanned_deadline
    ));
    let send_start = harness.qpc_now_for_test().expect("Up-only sender QPC");
    assert!(send_start >= replanned_deadline);
    harness.set_mock_sender_start_qpc_for_test(send_start);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(replanned_candidate, false),
        super::super::StagedDispatchOutcome::Dispatched
    );
    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 2);
    assert_eq!(
        packets[1].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert!(
        packets[1]
            .1
            .started_ticks
            .is_some_and(|started| started >= replanned_deadline)
    );
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.key_mask),
        Some(2)
    );
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
}

#[test]
fn scheduler_reachable_mixed_modifier_fallback_trace_spans_both_gates_without_wake() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    let (mut harness, packets, mixed) = prepare_due_mixed_fallback_candidate();
    let qpc_clock = harness.resources.clock;
    let up_window = mixed
        .up_only_timing_window()
        .expect("scheduler-derived Mixed Up timing window");
    assert!(mixed.packet_masks().down_mask != 0);
    assert!(mixed.packet_masks().up_mask != 0);
    assert!(up_window.musical_up_not_before_qpc <= harness.qpc_now_for_test().unwrap());
    assert!(up_window.packet_not_before_qpc <= harness.qpc_now_for_test().unwrap());

    let markers = Arc::new(Mutex::new(Vec::with_capacity(4)));
    install_held_modifier_and_gate_trace(&mut harness, Arc::clone(&markers));
    sky_dispatch_win32::focus::reset_foreground_query_count();
    sky_dispatch_win32::focus::reset_process_identity_counts_for_test();
    harness.reset_target_proof_counts_for_test();
    harness.runtime.final_lease_qpc_sample_count_for_test = 0;
    harness
        .supervisor_expired
        .reset(harness.qpc_now_for_test().unwrap());
    let send_start = harness
        .qpc_now_for_test()
        .expect("Mixed fallback send start");
    harness.set_mock_sender_start_qpc_for_test(send_start);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(mixed, true),
        super::super::StagedDispatchOutcome::Dispatched
    );
    let packets = packets.lock().expect("fallback packet capture");
    assert_eq!(
        packets.len(),
        1,
        "one physical attempt for the selected view"
    );
    assert_eq!(
        packets[0].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0),
        "a Mixed modifier rejection may send only its prepared Up prefix"
    );
    assert_eq!(packets[0].1.attempts, 1);
    assert_eq!(packets[0].1.requested_mask, 1);
    drop(packets);

    let Some(crate::engine::worker::dispatch::observation::DispatchObservation::Down(observation)) =
        harness.observer.pop_front()
    else {
        panic!("successful Up-prefix dispatch publishes its existing Down observation");
    };
    let handoff = observation
        .precision_handoff
        .expect("observer retains final-admission QPC boundaries");
    let gate_start = handoff
        .target_crossing_qpc
        .expect("single gate-start sample");
    let gate_end = observation
        .final_policy_qpc
        .expect("final Up-only gate-end sample");
    let markers = markers.lock().expect("final gate marker capture");
    assert_eq!(markers.len(), 4);
    assert_eq!(
        markers[0].boundary,
        super::super::FinalAdmissionGateBoundary::Entered
    );
    assert_eq!(
        markers[0].packet,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 2)
    );
    assert_eq!(
        markers[1].boundary,
        super::super::FinalAdmissionGateBoundary::Returned
    );
    assert_eq!(
        markers[1].packet,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 2)
    );
    assert_eq!(
        markers[2].boundary,
        super::super::FinalAdmissionGateBoundary::Entered
    );
    assert_eq!(
        markers[2].packet,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert_eq!(
        markers[3].boundary,
        super::super::FinalAdmissionGateBoundary::Returned
    );
    assert_eq!(
        markers[3].packet,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert!(gate_start <= markers[0].qpc.expect("Mixed gate entry marker"));
    assert!(markers[3].qpc.expect("Up-only gate return marker") <= gate_end);
    assert_eq!(
        handoff.final_policy_qpc,
        Some(gate_end),
        "trace gate-end is the sample after the last final admission"
    );
    assert!(observation.wake_qpc.is_none());
    assert!(handoff.admission_wake_qpc.is_none());
    assert_eq!(
        observation.requested_packet,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert_eq!(observation.confirmed_mask, 1);
    assert_eq!(observation.trace.send_attempts, 1);
    assert_eq!(
        observation.physical_timing_window, up_window,
        "the selected prefix uses its independent H/authored Up timing window"
    );
    assert_eq!(
        harness.runtime.precision_gate_qpc_sample_count_for_test, 2,
        "one start and one final end sample cover both admission attempts"
    );
    drop(markers);

    assert_fresh_up_admission_was_used(&harness);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.in_flight_mask_for_test(), 0);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.key_mask),
        Some(2),
        "the blocked Down remains pending and receives no onset before admission"
    );
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().released, 1);
    assert_eq!(
        qpc_clock.frequency_hz(),
        harness.resources.clock.frequency_hz()
    );
    let _ = harness.finalize_for_test();
}

#[test]
fn missing_optional_gate_qpc_evidence_does_not_change_authorization() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;

    for failure_index in [0, 1] {
        let (mut harness, packets, mixed) = prepare_due_mixed_fallback_candidate();
        let markers = Arc::new(Mutex::new(Vec::with_capacity(4)));
        install_held_modifier_and_gate_trace(&mut harness, Arc::clone(&markers));
        sky_dispatch_win32::focus::reset_foreground_query_count();
        sky_dispatch_win32::focus::reset_process_identity_counts_for_test();
        harness.reset_target_proof_counts_for_test();
        harness.runtime.final_lease_qpc_sample_count_for_test = 0;
        harness
            .runtime
            .precision_gate_qpc_sample_failure_index_for_test = Some(failure_index);
        harness
            .supervisor_expired
            .reset(harness.qpc_now_for_test().unwrap());
        harness.set_mock_sender_start_qpc_for_test(
            harness.qpc_now_for_test().expect("fallback sender QPC"),
        );

        assert_eq!(
            harness.dispatch_staged_candidate_for_test(mixed, true),
            super::super::StagedDispatchOutcome::Dispatched,
            "optional gate evidence failure {failure_index} does not alter admission"
        );
        let packets = packets.lock().expect("fallback packet capture");
        assert_eq!(packets.len(), 1);
        assert_eq!(
            packets[0].0,
            sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
        );
        assert_eq!(packets[0].1.attempts, 1);
        drop(packets);

        let Some(crate::engine::worker::dispatch::observation::DispatchObservation::Down(
            observation,
        )) = harness.observer.pop_front()
        else {
            panic!("successful dispatch still publishes its observer record");
        };
        let handoff = observation
            .precision_handoff
            .expect("one surviving optional QPC boundary retains the handoff");
        match failure_index {
            0 => {
                assert_eq!(handoff.target_crossing_qpc, None);
                assert!(observation.final_policy_qpc.is_some());
            }
            1 => {
                assert!(handoff.target_crossing_qpc.is_some());
                assert_eq!(observation.final_policy_qpc, None);
            }
            _ => unreachable!(),
        }
        assert_eq!(harness.runtime.precision_gate_qpc_sample_count_for_test, 2);
        assert_fresh_up_admission_was_used(&harness);
        assert_eq!(harness.release_obligation_mask_for_test(), 0);
        drop(markers);
        let _ = harness.finalize_for_test();
    }

    // Missing optional trace evidence cannot turn an invalid target into Up
    // permission. The same scheduler-derived candidate remains fail-closed.
    let (mut invalid, packets, mixed) = prepare_due_mixed_fallback_candidate();
    let markers = Arc::new(Mutex::new(Vec::with_capacity(2)));
    install_held_modifier_and_gate_trace(&mut invalid, Arc::clone(&markers));
    invalid
        .runtime
        .precision_gate_qpc_sample_failure_index_for_test = Some(0);
    invalid.runtime.final_lease_qpc_sample_count_for_test = 0;
    invalid.target.invalidate_for_test();
    invalid.set_mock_sender_start_qpc_for_test(
        invalid.qpc_now_for_test().expect("invalid target QPC"),
    );
    assert_eq!(
        invalid.dispatch_staged_candidate_for_test(mixed, true),
        super::super::StagedDispatchOutcome::Replan
    );
    assert!(packets.lock().expect("blocked packet capture").is_empty());
    assert_eq!(invalid.runtime.final_lease_qpc_sample_count_for_test, 0);
    let _ = invalid.finalize_for_test();
}

#[test]
fn observer_disabled_full_mixed_dispatch_takes_no_optional_gate_qpc_samples() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    let (mut harness, packets, mixed) = prepare_due_mixed_fallback_candidate();
    harness.set_mock_sender_start_qpc_for_test(
        harness.qpc_now_for_test().expect("full Mixed sender QPC"),
    );

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(mixed, false),
        super::super::StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.runtime.precision_gate_qpc_sample_count_for_test, 0);
    let packets = packets.lock().expect("full Mixed packet capture");
    assert_eq!(packets.len(), 1);
    assert_eq!(
        packets[0].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 2)
    );
    assert_eq!(packets[0].1.attempts, 1);
    drop(packets);
    assert_eq!(harness.release_obligation_mask_for_test(), 2);
    assert!(harness.current_executor_chord_for_test().is_none());
    let _ = harness.finalize_for_test();
}
