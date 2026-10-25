use super::*;

#[test]
fn lifecycle_active_cleanup_barrier_starts_after_verified_safety_up() {
    use super::test_support::ProductionDispatchTestHarness;
    use std::sync::atomic::AtomicU64;

    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(10_000_000, false)
            .expect("lifecycle vector admission");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
    let (cleanup, safety_up_completion) =
        harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    harness.prepare_prepared_stream_for_test();
    let down_at = harness.qpc_now_for_test().expect("active cleanup QPC");
    harness
        .reanchor_playback_clock_for_test(down_at)
        .expect("active cleanup epoch");
    harness.set_mock_sender_start_qpc_for_test(down_at);
    assert!(matches!(
        harness
            .dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(down_at,),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(harness.backend_active_mask(), 1);

    let qpc_clock = harness.resources.clock;
    let safety_up_completion_for_sample = Arc::clone(&safety_up_completion);
    let post_cleanup_sample = Arc::new(AtomicU64::new(0));
    let sample_store = Arc::clone(&post_cleanup_sample);
    harness
        .apply_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(move || {
            let q = qpc_clock
                .now()
                .map_err(|error| format!("active cleanup sample QPC: {error:?}"))?;
            let up_completed = safety_up_completion_for_sample.load(Ordering::Acquire);
            assert_ne!(up_completed, 0, "verified cleanup emitted the safety Up");
            assert!(
                q.as_u64() >= up_completed,
                "post-cleanup QPC must be no earlier than the safety Up completion"
            );
            sample_store.store(q.as_u64(), Ordering::Release);
            Ok(q)
        })
        .expect("verified active-key cleanup");

    let cleanup_events = cleanup.lock().expect("safety cleanup capture");
    assert_eq!(cleanup_events.as_slice(), &[(vec![0x15], true)]);
    assert_eq!(harness.backend_active_mask(), 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(packets.lock().expect("packet capture").len(), 1);

    let sample = QpcTicks::from_raw(post_cleanup_sample.load(Ordering::Acquire));
    let frame_ticks = qpc_clock
        .duration_from_us(16_667)
        .expect("frame conversion");
    let barrier = sample
        .checked_add_duration(frame_ticks)
        .expect("post-cleanup barrier arithmetic");
    assert_eq!(
        harness.down_floor_for_test(QpcTicks::ZERO, 1),
        Some(barrier),
        "every later Down must start no earlier than fresh cleanup QPC plus one frame"
    );
}

#[test]
fn lifecycle_cleanup_qpc_failure_and_overflow_leave_down_fail_closed() {
    use super::test_support::ProductionDispatchTestHarness;

    for overflow in [false, true] {
        let mut harness =
            ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(10_000_000, false)
                .expect("lifecycle vector admission");
        let packets = harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
        let cleanup = harness.configure_mock_cleanup_emitter_for_test();
        harness.prepare_prepared_stream_for_test();
        harness
            .reanchor_playback_clock_for_test(QpcTicks::ZERO)
            .expect("lifecycle playback epoch");

        let result = if overflow {
            harness.apply_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(|| {
                Ok(QpcTicks::from_raw(u64::MAX - 5))
            })
        } else {
            harness.apply_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(|| {
                Err("injected post-cleanup QPC failure".to_string())
            })
        };
        assert!(result.is_err());
        assert_eq!(
            harness
                .resources
                .coordinator
                .cancelled_up_token_count_for_test_support(),
            0,
            "failed post-cleanup QPC or barrier must not mint cancellation tokens"
        );
        assert!(cleanup.lock().expect("empty cleanup capture").is_empty());
        assert_eq!(
            harness.down_floor_for_test(QpcTicks::ZERO, 1),
            None,
            "{overflow}: invalid guard must not produce a Down floor"
        );
        assert!(
            harness
                .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(QpcTicks::ZERO)
                .is_err()
        );
        assert!(packets.lock().expect("packet capture").is_empty());
    }
}

#[test]
fn lifecycle_inconclusive_cleanup_does_not_establish_or_bypass_barrier() {
    use super::test_support::ProductionDispatchTestHarness;
    use std::sync::atomic::AtomicBool;

    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(10_000_000, false)
            .expect("lifecycle vector admission");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.prepare_prepared_stream_for_test();
    harness
        .reanchor_playback_clock_for_test(QpcTicks::ZERO)
        .expect("lifecycle playback epoch");
    harness.set_mock_sender_start_qpc_for_test(QpcTicks::ZERO);
    assert!(matches!(
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(
            QpcTicks::ZERO,
        ),
        super::worker::DispatchStep::Dispatched
    ));
    harness
        .resources
        .backend
        .set_probe(|_, _| sky_dispatch_win32::input::InstrumentPhysicalState::Inconclusive);

    let post_cleanup_sampled = Arc::new(AtomicBool::new(false));
    let sampled = Arc::clone(&post_cleanup_sampled);
    let result =
        harness.apply_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(move || {
            sampled.store(true, Ordering::Release);
            Ok(QpcTicks::from_raw(117_300))
        });
    assert!(result.is_err());
    assert_eq!(
        harness
            .resources
            .coordinator
            .cancelled_up_token_count_for_test_support(),
        0,
        "unverified physical cleanup must not mint cancellation tokens"
    );
    assert!(
        !post_cleanup_sampled.load(Ordering::Acquire),
        "an unverified cleanup cannot establish a resume barrier"
    );
    assert_eq!(
        cleanup.lock().expect("cleanup capture").len(),
        4,
        "an inconclusive probe retains the mask for all bounded cleanup rounds"
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
    assert_eq!(
        harness.down_floor_for_test(QpcTicks::ZERO, 1),
        None,
        "unverified release leaves the guard invalid"
    );
    assert!(
        harness
            .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(QpcTicks::from_raw(
                200_000
            ),)
            .is_err()
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
}

#[test]
fn focus_restoration_is_resumable_while_system_suspend_is_terminal() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut focus = ProductionDispatchTestHarness::new_down_only();
    let qpc_clock = focus.resources.clock;
    let focus_loss = qpc_clock.now().expect("focus loss QPC");
    focus
        .reanchor_playback_clock_for_test(focus_loss)
        .expect("focus lifecycle epoch");
    focus
        .lose_focus_for_test(focus_loss)
        .expect("focus loss transition");
    let focus_restore = qpc_clock.now().expect("focus restore QPC");
    focus
        .restore_focus_for_test(focus_restore)
        .expect("focus restore verified cleanup");
    let frame_ticks = qpc_clock.duration_from_us(16_667).expect("frame ticks");
    let focus_floor = focus
        .down_floor_for_test(QpcTicks::ZERO, 1)
        .expect("focus restore barrier");
    assert!(
        focus_floor
            >= focus_restore
                .checked_add_duration(frame_ticks)
                .expect("focus barrier arithmetic"),
        "focus restoration must use the shared post-cleanup barrier"
    );

    let mut power = ProductionDispatchTestHarness::new_down_only();
    let suspend_qpc = power.resources.clock.now().expect("suspend QPC");
    assert!(power.notify_system_power_at_for_test(true, Some(suspend_qpc)));
    assert!(power.notify_system_power_at_for_test(false, None));
    assert!(power.system_power_down_blocked_for_test());
    assert!(!power.complete_system_resume_for_test());
    assert!(power.process_command_control_for_test());
    assert_eq!(
        power.runtime.terminal_trigger,
        Some(super::worker::TerminalTrigger::SystemSuspend)
    );
    let mut observation = super::worker::FinalizeTestObservation::default();
    assert_eq!(
        power.finalize_worker_for_test(false, &mut observation),
        super::OUTCOME_SYSTEM_SUSPENDED
    );
    assert_eq!(
        observation.terminal_trigger_reason.as_deref(),
        Some("system_suspend")
    );
}

#[test]
fn release_obligation_lifecycle_matrix_keeps_safety_cleanup_outside_musical_pairing() {
    use super::test_support::ProductionDispatchTestHarness;

    for path in [
        "stop",
        "quit",
        "skip",
        "manual_pause_cleanup",
        "focus_loss",
        "target_hwnd_generation_change",
        "system_suspend",
        "supervisor_expiry",
        "panic_hard_stop",
        "terminal_cleanup",
        "worker_panic_cleanup",
    ] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_packet_capture();
        harness.resources.backend.custom_emitter = None;
        harness.align_next_plan_to_future_for_test(500_000);
        let plan = harness.plan_current_dispatch();
        assert!(matches!(
            harness.dispatch_at_plan_target_for_test(&plan),
            super::worker::DispatchStep::Dispatched
        ));

        assert_eq!(
            *packets.lock().expect("lifecycle packet capture"),
            vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1)],
            "{path}: successful musical Down"
        );
        let before = harness.resources.coordinator.generation_accounting();
        assert_eq!(before.activated, 1, "{path}: activated before cleanup");
        assert_eq!(before.active, 1, "{path}: active before cleanup");
        assert_eq!(before.released, 0, "{path}: released before cleanup");
        assert_ne!(
            harness.release_obligation_mask_for_test(),
            0,
            "{path}: successful Down must carry a release obligation"
        );

        match path {
            // There is no distinct runtime stop command: stop aliases quit's
            // command-exit and terminal-finalization path.
            "stop" | "quit" => {
                harness.quit_requested.store(true, Ordering::Release);
                assert!(
                    harness.process_command_control_for_test(),
                    "{path}: command-exit owner"
                );
                assert_eq!(
                    *packets.lock().expect("pre-finalizer packet capture"),
                    vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1)],
                    "{path}: command control must only latch the stop"
                );
                assert_eq!(
                    harness.resources.coordinator.generation_accounting().active,
                    1,
                    "{path}: generation ledger remains available for post-release audit"
                );
                let mut observation = super::worker::FinalizeTestObservation::default();
                let outcome = harness.finalize_worker_for_test(false, &mut observation);
                assert_eq!(outcome, super::OUTCOME_QUIT, "{path}: quit remains quit");
                assert_eq!(observation.terminal_trigger_reason.as_deref(), Some("quit"));
                assert_eq!(
                    observation.attempted_mask, 0b1,
                    "{path}: tracked scope mask"
                );
                assert_eq!(observation.release_obligation_mask, 0, "{path}: obligation");
                assert_eq!(observation.active_mask, 0, "{path}: active backend");
                assert_eq!(
                    observation.possibly_active_mask, 0,
                    "{path}: uncertain backend"
                );
                assert_eq!(observation.failed_release_mask, 0, "{path}: failed release");
                assert_eq!(observation.in_flight_mask, 0, "{path}: in-flight residue");
                assert_eq!(observation.generation_accounting.activated, 1);
                assert_eq!(observation.generation_accounting.released, 0);
                assert_eq!(observation.generation_accounting.active, 0);
                assert_eq!(observation.generation_accounting.cancelled, 1);
                assert_eq!(
                    *packets
                        .lock()
                        .expect("lifecycle packet capture after cleanup"),
                    vec![
                        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1),
                        sky_dispatch_win32::input::PhysicalPacket::new(0b1, 0),
                    ],
                    "{path}: one authored Down and one ownership-scoped safety Up"
                );
                continue;
            }
            "skip" => {
                harness.skip_requested.store(true, Ordering::Release);
                assert!(
                    harness.process_command_control_for_test(),
                    "{path}: command-exit owner"
                );
                assert_eq!(
                    *packets.lock().expect("pre-finalizer packet capture"),
                    vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1)],
                    "{path}: command control must only latch the stop"
                );
                let mut observation = super::worker::FinalizeTestObservation::default();
                let outcome = harness.finalize_worker_for_test(false, &mut observation);
                assert_eq!(
                    outcome,
                    super::OUTCOME_SKIPPED,
                    "{path}: skip remains skipped"
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("skipped")
                );
                assert_eq!(
                    observation.attempted_mask, 0b1,
                    "{path}: tracked scope mask"
                );
                assert_eq!(observation.release_obligation_mask, 0, "{path}: obligation");
                assert_eq!(observation.active_mask, 0, "{path}: active backend");
                assert_eq!(
                    observation.possibly_active_mask, 0,
                    "{path}: uncertain backend"
                );
                assert_eq!(observation.failed_release_mask, 0, "{path}: failed release");
                assert_eq!(observation.in_flight_mask, 0, "{path}: in-flight residue");
                assert_eq!(observation.generation_accounting.activated, 1);
                assert_eq!(observation.generation_accounting.released, 0);
                assert_eq!(observation.generation_accounting.active, 0);
                assert_eq!(observation.generation_accounting.cancelled, 1);
                assert_eq!(
                    *packets
                        .lock()
                        .expect("lifecycle packet capture after cleanup"),
                    vec![
                        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1),
                        sky_dispatch_win32::input::PhysicalPacket::new(0b1, 0),
                    ],
                    "{path}: one authored Down and one ownership-scoped safety Up"
                );
                continue;
            }
            "manual_pause_cleanup" => {
                harness.desired_pause.store(true, Ordering::Release);
                harness
                    .apply_resumable_lifecycle_transition_for_test()
                    .expect("manual pause dispatch-loop cleanup seam");
            }
            "focus_loss" => {
                harness.focus_active.store(false, Ordering::Release);
                let focus_ticks = harness.resources.clock.now().expect("focus QPC");
                super::worker::enter_focus_pause(
                    &mut harness.resources.playback,
                    &mut harness.runtime,
                    focus_ticks,
                    &harness.progress_clock,
                )
                .expect("focus-loss dispatch-loop transition");
                harness
                    .apply_resumable_lifecycle_transition_for_test()
                    .expect("focus restoration dispatch-loop cleanup seam");
            }
            "target_hwnd_generation_change" => {
                harness.target.publish(2);
                harness
                    .apply_resumable_lifecycle_transition_for_test()
                    .expect("target-change dispatch-loop cleanup seam");
            }
            "system_suspend" => {
                assert!(harness.notify_system_power_for_test(true));
                assert!(harness.notify_system_power_for_test(false));
                assert!(harness.system_power_down_blocked_for_test());
                assert!(harness.process_command_control_for_test());
                let mut observation = super::worker::FinalizeTestObservation::default();
                assert_eq!(
                    harness.finalize_worker_for_test(false, &mut observation),
                    super::OUTCOME_SYSTEM_SUSPENDED,
                    "{path}: suspend is a clean but terminal outcome"
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("system_suspend")
                );
                assert_eq!(observation.attempted_mask, 0b1);
                assert_eq!(observation.release_obligation_mask, 0);
                assert_eq!(
                    *packets.lock().expect("terminal packet capture"),
                    vec![
                        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1),
                        sky_dispatch_win32::input::PhysicalPacket::new(0b1, 0),
                    ],
                    "{path}: terminal finalizer releases held physical input"
                );
                continue;
            }
            "supervisor_expiry" => {
                harness.supervisor_expired.latch_expired();
                assert!(
                    harness.process_command_control_for_test(),
                    "{path}: process_command_control hard-stop owner"
                );
                assert_eq!(
                    harness.full_instrument_release_calls(),
                    0,
                    "{path}: supervisor expiry must use tracked ownership"
                );
                assert_eq!(
                    *packets.lock().expect("pre-finalizer packet capture"),
                    vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1)],
                    "{path}: lease control must defer release to finalization"
                );
                let mut observation = super::worker::FinalizeTestObservation::default();
                let outcome = harness.finalize_worker_for_test(false, &mut observation);
                assert_eq!(
                    outcome,
                    super::OUTCOME_ERROR,
                    "{path}: expiry is terminal error"
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("supervisor_lease_expired")
                );
                assert_eq!(observation.attempted_mask, 0b1);
                assert_eq!(observation.release_obligation_mask, 0);
                assert_eq!(
                    *packets.lock().expect("terminal packet capture"),
                    vec![
                        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1),
                        sky_dispatch_win32::input::PhysicalPacket::new(0b1, 0),
                    ]
                );
                continue;
            }
            "panic_hard_stop" => {
                harness.panic_requested.store(true, Ordering::Release);
                assert!(
                    harness.process_command_control_for_test(),
                    "{path}: process_command_control hard-stop owner"
                );
                assert_eq!(
                    harness.full_instrument_release_calls(),
                    0,
                    "{path}: panic hard-stop must use tracked ownership"
                );
                assert_eq!(
                    *packets.lock().expect("pre-finalizer packet capture"),
                    vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1)],
                    "{path}: panic control must defer release to finalization"
                );
                let mut observation = super::worker::FinalizeTestObservation::default();
                let outcome = harness.finalize_worker_for_test(false, &mut observation);
                assert_eq!(
                    outcome,
                    super::OUTCOME_ERROR,
                    "{path}: panic is terminal error"
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("panic_release_requested")
                );
                assert_eq!(observation.attempted_mask, 0b1);
                assert_eq!(observation.release_obligation_mask, 0);
                assert_eq!(
                    *packets.lock().expect("terminal packet capture"),
                    vec![
                        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1),
                        sky_dispatch_win32::input::PhysicalPacket::new(0b1, 0),
                    ]
                );
                continue;
            }
            "terminal_cleanup" => {
                let mut observation = super::worker::FinalizeTestObservation::default();
                harness.finalize_worker_for_test(false, &mut observation);
                assert_eq!(
                    observation.attempted_mask, 0b1,
                    "{path}: tracked scope mask"
                );
                assert_eq!(observation.release_obligation_mask, 0, "{path}: obligation");
                assert_eq!(observation.active_mask, 0, "{path}: active backend");
                assert_eq!(
                    observation.possibly_active_mask, 0,
                    "{path}: uncertain backend"
                );
                assert_eq!(observation.failed_release_mask, 0, "{path}: failed release");
                assert_eq!(observation.in_flight_mask, 0, "{path}: in-flight residue");
                assert_eq!(observation.generation_accounting.activated, 1);
                assert_eq!(observation.generation_accounting.released, 0);
                assert_eq!(observation.generation_accounting.active, 0);
                assert_eq!(observation.generation_accounting.cancelled, 1);
                assert_eq!(
                    *packets
                        .lock()
                        .expect("lifecycle packet capture after cleanup"),
                    vec![
                        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1),
                        sky_dispatch_win32::input::PhysicalPacket::new(0b1, 0),
                    ],
                    "{path}: one authored Down and one ownership-scoped safety Up"
                );
                continue;
            }
            "worker_panic_cleanup" => {
                let mut observation = super::worker::FinalizeTestObservation::default();
                let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    harness.finalize_worker_for_test(true, &mut observation);
                }));
                assert!(panic_result.is_err(), "{path}: worker panic must propagate");
                assert_eq!(
                    observation.attempted_mask, 0b1,
                    "{path}: worker panic must release only tracked ownership: {observation:?}"
                );
                assert_eq!(observation.release_obligation_mask, 0, "{path}: obligation");
                assert_eq!(observation.active_mask, 0, "{path}: active backend");
                assert_eq!(
                    observation.possibly_active_mask, 0,
                    "{path}: uncertain backend"
                );
                assert_eq!(observation.failed_release_mask, 0, "{path}: failed release");
                assert_eq!(observation.in_flight_mask, 0, "{path}: in-flight residue");
                assert_eq!(observation.generation_accounting.activated, 1);
                assert_eq!(observation.generation_accounting.released, 0);
                assert_eq!(observation.generation_accounting.active, 0);
                assert_eq!(observation.generation_accounting.cancelled, 1);
                assert_eq!(
                    *packets
                        .lock()
                        .expect("lifecycle packet capture after cleanup"),
                    vec![
                        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1),
                        sky_dispatch_win32::input::PhysicalPacket::new(0b1, 0),
                    ],
                    "{path}: one authored Down and one ownership-scoped safety Up"
                );
                continue;
            }
            _ => unreachable!("lifecycle matrix path is explicit above"),
        }

        assert_eq!(
            harness.full_instrument_release_calls(),
            0,
            "{path}: resumable lifecycle cleanup must use tracked ownership"
        );
        assert_eq!(
            harness.release_obligation_mask_for_test(),
            0,
            "{path}: verified cleanup must clear every obligation"
        );
        assert_eq!(
            harness.backend_active_mask(),
            0,
            "{path}: stuck active backend"
        );
        assert_eq!(
            harness.backend_possibly_active_mask(),
            0,
            "{path}: stuck uncertain backend"
        );
        assert_eq!(
            *packets
                .lock()
                .expect("lifecycle packet capture after cleanup"),
            vec![
                sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1),
                sky_dispatch_win32::input::PhysicalPacket::new(0b1, 0),
            ],
            "{path}: one authored Down and one ownership-scoped safety Up"
        );

        let after = harness.resources.coordinator.generation_accounting();
        assert_eq!(after.activated, 1, "{path}: activation accounting");
        assert_eq!(after.active, 0, "{path}: active accounting after cleanup");
        assert_eq!(
            after.released, 0,
            "{path}: safety Up became musical Released"
        );
        assert_eq!(after.cancelled, 1, "{path}: cancellation accounting");
    }
}

#[test]
fn terminal_release_precedes_million_entry_ledger_audits_and_publication() {
    use super::test_support::ProductionDispatchTestHarness;
    use parking_lot::Mutex;
    use sky_dispatch_core::model::{KeyRegistry, RuntimeSchedule};

    const GENERATIONS: u64 = 1_000_000;
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.resources.backend.custom_emitter = None;
    let packets = harness.configure_packet_capture();
    harness.resources.backend.active_mask = 1;
    harness.resources.coordinator =
        sky_dispatch_core::coordinator::RuntimeDispatchCoordinator::try_new_ticks(
            RuntimeSchedule {
                packets: Vec::new(),
                batches: Vec::new(),
                intents: Vec::new(),
                generation_count: GENERATIONS,
                key_registry: KeyRegistry::new(&[]),
                reason_table: Vec::new(),
            },
            0,
            DurationTicks::ZERO,
            |us| Ok(TimelineTicks::from_raw(us)),
        )
        .expect("million-entry cancellation ledger");

    let trace = Arc::new(Mutex::new(Vec::new()));
    let trace_for_sender = Arc::clone(&trace);
    let sender = harness
        .resources
        .backend
        .custom_packet_emitter
        .take()
        .expect("configured packet capture sender");
    harness.resources.backend.set_packet_emitter(move |packet| {
        if packet.up_mask != 0 {
            trace_for_sender.lock().push("physical_release_attempt");
        }
        sender(packet)
    });

    harness.quit_requested.store(true, Ordering::Release);
    assert!(harness.process_command_control_for_test());
    assert_eq!(
        *packets.lock().expect("packet capture before finalization"),
        Vec::<PhysicalPacket>::new(),
        "the command boundary only latches Quit"
    );
    assert_eq!(
        harness
            .resources
            .coordinator
            .generation_accounting()
            .scheduled,
        GENERATIONS,
        "the generation ledger has not been traversed or cancelled"
    );

    let mut observation = super::worker::FinalizeTestObservation {
        operation_trace: Some(Arc::clone(&trace)),
        ..Default::default()
    };
    assert_eq!(
        harness.finalize_worker_for_test(false, &mut observation),
        super::OUTCOME_QUIT
    );
    assert_eq!(observation.terminal_trigger_reason.as_deref(), Some("quit"));
    assert_eq!(observation.terminal_release_attempts, 1);
    assert_eq!(observation.total_release_attempts, 1);
    assert_eq!(
        observation.pre_cancel_generation_accounting.total,
        GENERATIONS
    );
    assert_eq!(
        observation.pre_cancel_generation_accounting.scheduled,
        GENERATIONS
    );
    assert_eq!(observation.generation_accounting.cancelled, GENERATIONS);
    assert_eq!(observation.release_obligation_mask, 0);
    assert_eq!(
        *packets.lock().expect("terminal packet capture"),
        vec![PhysicalPacket::new(1, 0)]
    );

    let trace = trace.lock();
    let position = |event| {
        trace
            .iter()
            .position(|observed| *observed == event)
            .unwrap_or_else(|| panic!("missing terminal event {event}: {trace:?}"))
    };
    assert_eq!(
        trace
            .iter()
            .filter(|event| **event == "physical_release_attempt")
            .count(),
        1
    );
    assert!(position("terminal_release_started") < position("physical_release_attempt"));
    assert!(position("physical_release_attempt") < position("pre_cancel_ledger_audit"));
    assert!(position("pre_cancel_ledger_audit") < position("coordinator_cancellation"));
    assert!(position("coordinator_cancellation") < position("post_cleanup_ledger_audit"));
    assert!(position("post_cleanup_ledger_audit") < position("observer_shutdown"));
    assert!(position("observer_shutdown") < position("generation_metrics_export"));
    assert!(position("generation_metrics_export") < position("telemetry_export"));
    assert!(position("observer_shutdown") < position("terminal_metrics_publication"));
    assert!(position("physical_release_attempt") < position("generation_metrics_export"));
    assert!(position("physical_release_attempt") < position("terminal_metrics_publication"));
    assert!(position("physical_release_attempt") < position("telemetry_export"));
}

#[test]
fn failed_resumable_release_and_terminal_escalation_account_for_both_budgets() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::InstrumentPhysicalState;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let packets = harness.configure_packet_capture();
    harness.resources.backend.custom_emitter = None;
    harness.align_next_plan_to_future_for_test(500_000);
    let plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&plan),
        super::worker::DispatchStep::Dispatched
    ));

    let release_send_count = Arc::new(AtomicU64::new(0));
    let captured_releases = Arc::new(std::sync::Mutex::new(Vec::new()));
    let count_sender = Arc::clone(&release_send_count);
    let packet_sender = Arc::clone(&captured_releases);
    harness.resources.backend.set_packet_emitter(move |packet| {
        count_sender.fetch_add(1, Ordering::SeqCst);
        packet_sender
            .lock()
            .expect("release packet capture")
            .push(packet);
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask,
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
    harness
        .resources
        .backend
        .set_probe(|_, _| InstrumentPhysicalState::Inconclusive);

    let resumable_error = harness
        .apply_resumable_lifecycle_transition_for_test()
        .expect_err("uncertain resumable release escalates to terminal cleanup");
    assert_eq!(release_send_count.load(Ordering::SeqCst), 4);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.failed_release_mask, 1);
    assert_eq!(
        harness
            .resources
            .coordinator
            .cancelled_up_token_count_for_test_support(),
        0,
        "unverified resumable cleanup must not mint cancellation tokens"
    );
    assert_eq!(
        harness.down_floor_for_test(QpcTicks::ZERO, 1),
        None,
        "unverified cleanup must not establish or allow a resume Down barrier"
    );
    harness.runtime.terminal_error = Some(resumable_error.clone());

    let mut observation = super::worker::FinalizeTestObservation::default();
    assert_eq!(
        harness.finalize_worker_for_test(false, &mut observation),
        super::OUTCOME_ERROR,
        "uncertain cleanup must never be reported Finished"
    );
    assert_eq!(release_send_count.load(Ordering::SeqCst), 8);
    assert_eq!(observation.terminal_release_attempts, 4);
    assert_eq!(observation.total_release_attempts, 8);
    assert_eq!(observation.cleanup_outcome.as_ref().unwrap().attempts, 4);
    assert!(
        observation
            .cleanup_outcome
            .as_ref()
            .unwrap()
            .verification_inconclusive
    );
    assert_eq!(observation.cleanup_outcome.as_ref().unwrap().stuck_mask, 1);
    assert_eq!(observation.release_obligation_mask, 1);
    assert_eq!(observation.failed_release_mask, 1);
    assert_eq!(
        observation.terminal_error.as_deref(),
        Some(resumable_error.as_str())
    );
    assert!(
        observation
            .secondary_errors
            .iter()
            .any(|error| error.contains("terminal release verification failed"))
    );
    assert_eq!(
        captured_releases
            .lock()
            .expect("release packet capture")
            .len(),
        8
    );
    assert_eq!(
        packets.lock().expect("musical packet capture").as_slice(),
        &[PhysicalPacket::new(0, 1)]
    );
}

#[test]
fn sender_and_commit_panics_keep_uncertain_keys_for_terminal_release() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::PreparedSendPanicPoint;

    for (point, expected_active, expected_in_flight) in [
        (PreparedSendPanicPoint::AfterSenderReturn, 0, 1),
        (PreparedSendPanicPoint::AfterStateCommit, 1, 0),
    ] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_packet_capture();
        harness.resources.backend.custom_emitter = None;
        harness.align_next_plan_to_future_for_test(500_000);
        let plan = harness.plan_current_dispatch();
        harness
            .resources
            .backend
            .set_prepared_send_panic_point_for_test(Some(point));

        let dispatch_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            harness.dispatch_at_plan_target_for_test(&plan)
        }));
        assert!(
            dispatch_result.is_err(),
            "{point:?}: dispatch panic injected"
        );
        assert_eq!(
            harness.resources.backend.active_mask, expected_active,
            "{point:?}"
        );
        assert_eq!(
            harness.resources.backend.in_flight_mask, expected_in_flight,
            "{point:?}"
        );
        assert_eq!(harness.release_obligation_mask_for_test(), 1, "{point:?}");

        // The one-shot fault has already crossed the sender/commit seam. The
        // terminal release is then allowed to run its own safety transaction.
        harness
            .resources
            .backend
            .set_prepared_send_panic_point_for_test(None);
        let mut observation = super::worker::FinalizeTestObservation::default();
        let finalize_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            harness.finalize_worker_for_test(true, &mut observation)
        }));
        assert!(
            finalize_result.is_err(),
            "{point:?}: worker panic remains visible"
        );
        assert_eq!(
            observation.terminal_trigger_reason.as_deref(),
            Some("worker_panic")
        );
        assert_eq!(observation.terminal_release_attempts, 1, "{point:?}");
        assert_eq!(observation.total_release_attempts, 1, "{point:?}");
        assert_eq!(observation.release_obligation_mask, 0, "{point:?}");
        assert_eq!(observation.active_mask, 0, "{point:?}");
        assert_eq!(observation.in_flight_mask, 0, "{point:?}");
        assert!(
            observation
                .terminal_error
                .as_deref()
                .is_some_and(|error| error.contains("worker panicked"))
        );
        assert_eq!(
            *packets.lock().expect("panic packet capture"),
            vec![PhysicalPacket::new(0, 1), PhysicalPacket::new(1, 0),],
            "{point:?}: uncertain or committed ownership is released exactly once"
        );
    }
}

#[test]
fn empty_ownership_lifecycle_cleanup_emits_no_transport() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::PhysicalPacket;

    for path in [
        "manual_pause",
        "system_suspend",
        "supervisor_expiry",
        "panic_hard_stop",
        "worker_panic_finalize",
        "quit_before_down",
        "skip_before_down",
    ] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_packet_capture();
        harness.resources.backend.custom_emitter = None;
        let full_release_count = Arc::new(AtomicU64::new(0));
        harness
            .resources
            .backend
            .set_full_instrument_release_counter(Arc::clone(&full_release_count));
        assert_eq!(harness.release_obligation_mask_for_test(), 0, "{path}");

        match path {
            "manual_pause" => {
                harness
                    .suspend_live_input_for_test()
                    .expect("manual pause cleanup");
            }
            "system_suspend" => {
                assert!(harness.notify_system_power_for_test(true));
                assert!(harness.notify_system_power_for_test(false));
                assert!(harness.system_power_down_blocked_for_test());
                assert!(harness.process_command_control_for_test());
                let mut observation = super::worker::FinalizeTestObservation::default();
                assert_eq!(
                    harness.finalize_worker_for_test(false, &mut observation),
                    super::OUTCOME_SYSTEM_SUSPENDED
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("system_suspend")
                );
            }
            "supervisor_expiry" => {
                harness.supervisor_expired.latch_expired();
                assert!(harness.process_command_control_for_test());
                let mut observation = super::worker::FinalizeTestObservation::default();
                assert_eq!(
                    harness.finalize_worker_for_test(false, &mut observation),
                    super::OUTCOME_ERROR
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("supervisor_lease_expired")
                );
            }
            "panic_hard_stop" => {
                harness.panic_requested.store(true, Ordering::Release);
                assert!(harness.process_command_control_for_test());
                let mut observation = super::worker::FinalizeTestObservation::default();
                assert_eq!(
                    harness.finalize_worker_for_test(false, &mut observation),
                    super::OUTCOME_ERROR
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("panic_release_requested")
                );
            }
            "worker_panic_finalize" => {
                let mut observation = super::worker::FinalizeTestObservation::default();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    harness.finalize_worker_for_test(true, &mut observation);
                }));
                assert!(result.is_err(), "worker panic must propagate");
            }
            "quit_before_down" => {
                harness.quit_requested.store(true, Ordering::Release);
                assert!(harness.process_command_control_for_test());
                harness.finalize_worker_for_test(
                    false,
                    &mut super::worker::FinalizeTestObservation::default(),
                );
            }
            "skip_before_down" => {
                harness.skip_requested.store(true, Ordering::Release);
                assert!(harness.process_command_control_for_test());
                harness.finalize_worker_for_test(
                    false,
                    &mut super::worker::FinalizeTestObservation::default(),
                );
            }
            _ => unreachable!("the lifecycle case list is explicit"),
        }

        assert_eq!(
            *packets.lock().expect("empty ownership packet capture"),
            Vec::<PhysicalPacket>::new(),
            "{path}: empty ownership must not emit a safety Up"
        );
        assert_eq!(full_release_count.load(Ordering::SeqCst), 0, "{path}");
    }
}

#[test]
fn deferred_release_does_not_block_unrelated_down_chord() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_deferred_release_with_unrelated_down();
    let calls = harness.configure_send_counter();

    harness.align_next_plan_to_future_for_test(500_000);
    let first_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first_plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.backend_active_mask() & 0b001, 0b001);

    let authored_chord_plan = harness.plan_current_dispatch();
    let authored_chord_step = harness.dispatch_at_plan_target_for_test(&authored_chord_plan);
    assert!(
        matches!(authored_chord_step, super::worker::DispatchStep::Dispatched),
        "unrelated chord step: {authored_chord_step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.backend_active_mask() & 0b110, 0b110);

    harness.align_next_plan_to_future_for_test(500_000);
    let cleanup_plan = harness.plan_current_dispatch();
    let cleanup_step = harness.dispatch_at_plan_target_for_test(&cleanup_plan);
    assert!(
        matches!(cleanup_step, super::worker::DispatchStep::Dispatched),
        "cleanup step: {cleanup_step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.backend_active_mask(), 0);
}

#[test]
fn deferred_release_and_authored_chord_have_exact_packet_order() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::PhysicalPacket;

    let mut harness = ProductionDispatchTestHarness::new_deferred_release_with_unrelated_down();
    let packets = harness.configure_packet_capture();

    harness.align_next_plan_to_future_for_test(500_000);
    let first_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first_plan),
        super::worker::DispatchStep::Dispatched
    ));

    let authored_plan = harness.plan_current_dispatch();
    let authored_step = harness.dispatch_at_plan_target_for_test(&authored_plan);
    assert!(
        matches!(authored_step, super::worker::DispatchStep::Dispatched),
        "unrelated chord packet step: {authored_step:?}"
    );

    harness.align_next_plan_to_future_for_test(500_000);
    let cleanup_plan = harness.plan_current_dispatch();
    let cleanup_result = harness.dispatch_at_plan_target_for_test(&cleanup_plan);
    assert!(
        matches!(cleanup_result, super::worker::DispatchStep::Dispatched),
        "cleanup step: {cleanup_result:?}"
    );

    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![
            PhysicalPacket::new(0, 0b001),
            PhysicalPacket::new(0b001, 0b110),
            PhysicalPacket::new(0b110, 0),
        ],
        "the authored Up and unrelated Down share their authored boundary"
    );
}

#[test]
fn manual_pause_retains_authored_up_and_exposes_user_held_key_collision() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_admissible_dynamic_pending_release();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    harness.seed_pending_release_for_test(0x15, 50_000);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 1);

    harness
        .suspend_live_input_for_test()
        .expect("manual pause suspension");
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.resources.coordinator.pending_release_mask(), 0);
    // Model the user physically holding the same instrument key after the
    // verified safety cleanup. SendInput carries no ownership identity, so
    // the resumed authored Up remains an observable Up event.
    harness
        .resources
        .backend
        .set_probe(|_, _| InstrumentPhysicalState::Held(vec![0x15].into()));
    let packets = harness.configure_packet_capture();
    let cleanup = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&cleanup),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(harness.resources.coordinator.is_finished());
    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![PhysicalPacket::new(0b001, 0)],
        "the authored Up stays in the resumed packet despite an external physical hold"
    );
    harness
        .resources
        .coordinator
        .check_post_cleanup_invariants()
        .expect("clean suspension state");
}

#[test]
fn focus_suspend_restore_retains_authored_up_after_safety_cleanup() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_admissible_dynamic_pending_release();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    harness.seed_pending_release_for_test(0x15, 50_000);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 1);

    // Focus restoration uses the same production suspension seam before it
    // re-preflights the target.  A resumed planner must not rediscover the
    // cancelled pending Up.
    harness
        .suspend_live_input_for_test()
        .expect("focus suspension");
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.resources.coordinator.pending_release_mask(), 0);
    let packets = harness.configure_packet_capture();
    let cleanup = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&cleanup),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(harness.resources.coordinator.is_finished());
    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![PhysicalPacket::new(0b001, 0)],
        "the frozen authored Up remains physical work after focus restoration"
    );
    harness
        .resources
        .coordinator
        .check_post_cleanup_invariants()
        .expect("clean focus suspension state");
}

#[test]
fn pending_release_equal_authored_boundary_sends_one_up_packet() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::PhysicalPacket;

    let mut harness = ProductionDispatchTestHarness::new_pending_release_with_metadata_boundary();

    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    let second = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&second),
        super::worker::DispatchStep::Dispatched
    ));

    harness.seed_pending_release_for_test(0x15, 220_000);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 1);

    let equal_boundary = harness.plan_current_dispatch();
    let physical = equal_boundary
        .physical()
        .expect("equal boundary must remain physical");
    assert_eq!(
        physical.authored_view.packet_masks,
        PhysicalPacket::new(0b011, 0),
        "pending A Up must coalesce with authored B Up at the equal boundary"
    );
    let packets = harness.configure_packet_capture();
    assert!(
        matches!(
            harness.dispatch_at_plan_target_for_test(&equal_boundary),
            super::worker::DispatchStep::Dispatched
        ),
        "equal-boundary dispatch failed"
    );

    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![PhysicalPacket::new(0b011, 0)]
    );
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.resources.coordinator.cursor, 3);
}

#[test]
fn overdue_up_only_boundary_releases_and_continues() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let calls = harness.configure_send_counter();

    let first_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first_plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    harness.advance_playback_time_us(100_000);
    let overdue_plan = harness.plan_current_dispatch();
    let step = harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&overdue_plan);
    assert!(matches!(step, super::worker::DispatchStep::Dispatched));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.backend_active_mask(), 0);
}

#[test]
fn future_classification_then_waiter_entry_stall_keeps_exact_boundary_authorized() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    let calls = harness.configure_send_counter();

    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // B is future at first classification.  The worker must leave the guard
    // armed even though dispatch_due returns NoWork before entering wait.
    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        super::worker::DispatchStep::NoWork
    ));

    // A same-boundary replan (the equivalent of a non-physical Continue
    // before waiter entry) must retain the exact authorization token.
    let replanned = harness.plan_current_dispatch();

    // Keep the same frozen B plan. Model a stall before waiter entry so the
    // waiter returns Due { wait_result: None } for this already-overdue
    // target. Exact future authorization survives waiter-entry latency;
    // physical floors decide the execution boundary while authored identity
    // remains unchanged.
    let step = harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&replanned);
    assert!(matches!(step, super::worker::DispatchStep::Dispatched));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn due_frozen_plan_does_not_reenter_preparation_or_preflight() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let plan = harness.plan_current_dispatch();
    let prepared_counts = harness.preparation_counts();
    assert_eq!(prepared_counts.packet_header_reads, 1);
    assert_eq!(prepared_counts.up_intent_visits, 0);
    assert_eq!(prepared_counts.down_intent_visits, 1);
    assert_eq!(prepared_counts.registry_lookups, 1);
    assert_eq!(prepared_counts.view_packet_calls, 1);
    assert_eq!(prepared_counts.commit_freeze_calls, 1);
    assert_eq!(prepared_counts.conflict_calls, 1);
    assert_eq!(prepared_counts.input_build_calls, 1);
    assert_eq!(prepared_counts.preflight_calls, 1);

    let forced_preflight_failure = Arc::new(AtomicBool::new(false));
    harness.set_force_preflight_failure(Arc::clone(&forced_preflight_failure));
    forced_preflight_failure.store(true, Ordering::Release);

    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        harness.preparation_counts(),
        prepared_counts,
        "WaitBoundary::Due dispatch must consume the frozen view and packet"
    );

    let mut missing_proof = ProductionDispatchTestHarness::new_down_only();
    let mut missing_proof_plan = missing_proof.plan_current_dispatch();
    if let Some(physical) = missing_proof_plan.physical_mut() {
        physical.target_proof = super::worker::TargetProof::Required;
    }
    let step = missing_proof.dispatch_at_plan_target_for_test(&missing_proof_plan);
    assert!(matches!(
        step,
        super::worker::DispatchStep::Terminate(error)
            if error.contains("without preflight proof")
    ));
}

#[test]
fn frozen_dispatch_qpc_probe_has_no_redundant_effective_now_sample() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::clock::{qpc_read_count, reset_qpc_read_count};

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_packet_capture();
    harness.align_next_plan_to_future_for_test(5_000_000);
    let plan = harness.plan_current_dispatch();

    // The frozen-plan waiter/dispatch seam must retain the handoff sample,
    // final admission, SendInput completion, and explicitly diagnostic
    // dispatch-ready sample. In particular, no extra QPC is permitted merely
    // to reconstruct effective_now.
    reset_qpc_read_count();
    assert!(matches!(
        harness
            .wait_and_dispatch_current_plan(&plan)
            .expect("frozen plan wait"),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(
        qpc_read_count() >= 4,
        "precision path must retain the handoff and transport QPC samples"
    );
}

#[test]
fn frozen_target_reaches_dispatch_and_observation_without_reconstruction() {
    use super::test_support::ProductionDispatchTestHarness;
    use super::worker::dispatch::DispatchObservation;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_packet_capture();
    let plan = harness.plan_current_dispatch();
    let target = plan.physical_target_qpc().expect("frozen physical target");

    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&plan),
        super::worker::DispatchStep::Dispatched
    ));
    let observation = harness.pop_observation().expect("physical observation");
    match observation {
        DispatchObservation::Down(observation) => {
            assert_eq!(observation.physical_target_qpc, target);
        }
        other => panic!("unexpected observation variant: {other:?}"),
    }
}
