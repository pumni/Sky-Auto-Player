fn sender_failure_record_for_test(
    key_count: usize,
    status: SendTransactionStatus,
    inserted_count: u8,
) -> String {
    let mut harness = if key_count == 1 {
        ProductionDispatchTestHarness::new_down_only()
    } else {
        ProductionDispatchTestHarness::new_down_chord(key_count)
    };
    let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = (status != SendTransactionStatus::ClockFailureAfterSend)
            .then_some(started);
        let confirmed_mask = if inserted_count == 0 {
            0
        } else {
            packet.down_mask & packet.down_mask.wrapping_neg()
        };
        SendTransactionOutcome {
            status,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask,
                skipped_mask: 0,
                first_inserted: inserted_count,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: match status {
                    SendTransactionStatus::ZeroProgress => PacketRetryReason::ZeroProgress,
                    SendTransactionStatus::IntegrityLost => {
                        PacketRetryReason::PartialProgress {
                            inserted_count,
                        }
                    }
                    _ => PacketRetryReason::None,
                },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started),
                completed_ticks: completed,
                timing_error: None,
            },
        }
    });

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        super::StagedDispatchOutcome::Exit
    ));
    let record = harness
        .runtime
        .terminal_error
        .as_ref()
        .expect("terminal sender error record")
        .clone();
    let cleanup = harness.finalize_for_test();
    assert_eq!(cleanup.active_mask, 0);
    assert_eq!(cleanup.possibly_active_mask, 0);
    record
}

#[test]
fn failed_sender_paths_retain_bounded_attempt_and_clock_evidence() {
    let zero = sender_failure_record_for_test(1, SendTransactionStatus::ZeroProgress, 0);
    assert!(
        zero.contains("requested=1 confirmed_count=0 first_inserted=0")
            && zero.contains("attempts=1 status=ZeroProgress"),
        "{zero}"
    );
    assert!(zero.contains("retry_reason=ZeroProgress win32_error=Some(5)"));
    assert!(zero.contains("pre_call_qpc=") && zero.contains("completion_qpc="));

    let partial = sender_failure_record_for_test(2, SendTransactionStatus::IntegrityLost, 1);
    assert!(partial.contains(
        "source_action=0 source_chord=0 up_mask=0x0000 down_mask=0x0003"
    ));
    assert!(partial.contains("requested=2 confirmed_count=1 first_inserted=1"));
    assert!(partial.contains("attempts=1 status=IntegrityLost"));
    assert!(partial.contains("retry_reason=PartialProgress { inserted_count: 1 }"));

    let clock_failure = sender_failure_record_for_test(
        1,
        SendTransactionStatus::ClockFailureAfterSend,
        1,
    );
    assert!(clock_failure.contains("status=ClockFailureAfterSend"));
    assert!(clock_failure.contains("completion_qpc=unavailable"));
}

#[test]
fn inconsistent_complete_evidence_fails_closed_before_scheduler_commit() {
    for (confirmed_mask, attempts, expected_evidence) in [
        (0b0001, 1, "confirmed_count=0 first_inserted=2 attempts=1"),
        (0b0011, 2, "confirmed_count=0 first_inserted=2 attempts=2"),
    ] {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(2);
        let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
        harness.attach_executor_for_test().expect("attach scheduler");
        let chord = harness.current_executor_chord_for_test();
        let candidate = harness
            .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
            .expect("stage candidate")
            .expect("Down candidate");
        let packet_mask = candidate.packet_masks.down_mask;
        let accounting_before = harness.resources.scheduler.accounting();
        let clock = harness.resources.clock;
        harness.resources.backend.set_packet_emitter(move |packet| {
            let started = clock.now().expect("sender start QPC");
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask: packet.up_mask | packet.down_mask,
                    confirmed_mask,
                    skipped_mask: 0,
                    first_inserted: packet.event_count(),
                    attempts,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started),
                    completed_ticks: Some(started),
                    timing_error: None,
                },
            }
        });

        assert!(matches!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            super::StagedDispatchOutcome::Exit
        ));
        assert_eq!(harness.resources.scheduler.accounting(), accounting_before);
        assert_eq!(
            harness.current_executor_chord_for_test().map(|chord| chord.chord_index),
            Some(0)
        );
        assert_eq!(harness.resources.backend.in_flight_mask, packet_mask);
        assert_eq!(
            harness.resources.backend.release_obligation_mask() & packet_mask,
            packet_mask
        );
        let error = harness
            .runtime
            .terminal_error
            .as_deref()
            .expect("terminal sender error");
        assert!(error.contains("status=IntegrityLost"), "{error}");
        assert!(error.contains(expected_evidence), "{error}");
        let _ = harness.finalize_for_test();
    }
}

#[test]
fn mixed_post_qpc_failure_keeps_transport_evidence_and_skips_scheduler_commit() {
    for (inserted, requested_win32_error) in [(0, 5), (1, 5), (2, 0)] {
        let (mut harness, _) = mixed_scheduler_after_accepted_a_down();
        let (cleanup_calls, _) =
            harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
        let action = next_mixed_action(&harness);
        let candidate = stage_scheduler_action(
            action,
            &harness.target,
            &mut harness.resources,
            &mut harness.runtime,
        )
        .expect("stage live Mixed candidate")
        .expect("live Mixed candidate");
        let packet = PhysicalPacket::new(
            candidate.packet_masks.up_mask,
            candidate.packet_masks.down_mask,
        );
        assert_ne!(packet.up_mask, 0);
        assert_ne!(packet.down_mask, 0);
        assert_eq!(packet.event_count(), 2);

        let accounting_before = harness.resources.scheduler.accounting();
        let captured_failure = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let failure_capture = std::sync::Arc::clone(&captured_failure);
        let clock = harness.resources.clock;
        let started_ticks = clock.now().expect("mock sender start QPC");
        harness.resources.backend.set_packet_emitter(move |sent| {
            failure_capture
                .lock()
                .expect("failed packet capture")
                .push(sent);
            let win32_error = if inserted < sent.event_count() {
                requested_win32_error
            } else {
                0
            };
            SendTransactionOutcome {
                status: SendTransactionStatus::ClockFailureAfterSend,
                evidence: SendEvidence {
                    requested_mask: sent.up_mask | sent.down_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: inserted,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: (win32_error != 0).then_some(win32_error),
                    last_win32_error: (win32_error != 0).then_some(win32_error),
                    started_ticks: Some(started_ticks),
                    completed_ticks: None,
                    timing_error: Some(sky_dispatch_win32::clock::QpcError::CounterUnavailable),
                },
            }
        });

        assert!(matches!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            super::StagedDispatchOutcome::Exit
        ));
        assert_eq!(
            *captured_failure.lock().expect("failed packet capture"),
            vec![packet]
        );
        assert_eq!(harness.resources.scheduler.accounting(), accounting_before);
        assert_eq!(
            harness.current_executor_chord_for_test().map(|chord| chord.chord_index),
            Some(1),
            "post-QPC failure must not commit the scheduler cursor"
        );

        let expected_obligation = if inserted == 0 {
            packet.up_mask
        } else {
            packet.up_mask | packet.down_mask
        };
        assert_eq!(
            harness.resources.backend.release_obligation_mask(),
            expected_obligation,
            "inserted={inserted}: only a definite zero insertion may discard the new Down"
        );
        assert_eq!(
            harness.resources.backend.in_flight_mask,
            if inserted == 0 { 0 } else { packet.up_mask | packet.down_mask }
        );
        let error = harness
            .runtime
            .terminal_error
            .as_deref()
            .expect("post-QPC sender failure is terminal");
        assert!(error.contains("status=ClockFailureAfterSend"), "{error}");
        assert!(
            error.contains(&format!("first_inserted={inserted}")),
            "{error}"
        );
        assert!(
            error.contains("completion_qpc=unavailable")
                && error.contains("CounterUnavailable"),
            "{error}"
        );
        if inserted < packet.event_count() && requested_win32_error != 0 {
            assert!(error.contains(&format!("win32_error=Some({requested_win32_error})")), "{error}");
        }

        let finalizer = harness.finalize_for_test();
        assert_eq!(finalizer.attempted_mask, expected_obligation);
        assert_eq!(finalizer.release_obligation_mask, 0);
        let mut cleanup_codes = cleanup_calls
            .lock()
            .expect("cleanup capture")
            .iter()
            .flat_map(|(scan_codes, key_up)| {
                assert!(*key_up, "terminal mixed cleanup must send Up only");
                scan_codes.iter().copied()
            })
            .collect::<Vec<_>>();
        cleanup_codes.sort_unstable();
        let mut expected_codes = sky_dispatch_win32::input::scan_codes_from_mask(expected_obligation)
            .into_vec();
        expected_codes.sort_unstable();
        assert_eq!(cleanup_codes, expected_codes, "inserted={inserted}");
    }
}

#[test]
fn unstable_target_replan_wait_bounds_overdue_dispatch_retries() {
    let mut harness = ProductionDispatchTestHarness::new_down_chord(1);
    harness.runtime.verified_target = None;
    let target = std::sync::Arc::clone(&harness.target);
    harness.resources.backend.set_preflight_physical_mask_probe(move || {
        let next_hwnd = match target.load_stable().map(|(hwnd, _)| hwnd) {
            Some(1) => 2,
            _ => 1,
        };
        target.publish(next_hwnd);
        0
    });
    let clock = harness.resources.clock;
    let epoch = sky_dispatch_win32::clock::QpcTicks::ZERO;
    harness.resources.playback.epoch = epoch;
    let accounting_before = harness.resources.scheduler.accounting();
    let start = clock.now().expect("test start");
    let duration = clock.duration_from_us(20_000).expect("window duration");
    let mut stage_attempts = 0usize;

    loop {
        let now = clock.now().expect("test loop QPC");
        if now
            .checked_duration_since(start)
            .expect("monotonic test QPC")
            >= duration
        {
            break;
        }
        let action = evaluate_scheduler_action(&harness.resources.scheduler, now, epoch)
            .expect("scheduler action");
        let packet_not_before = match &action {
            SchedulerAction::Send { timing_window, .. } => timing_window.packet_not_before_qpc,
            action => panic!("overdue Down must remain eligible for replan: {action:?}"),
        };
        assert!(
            stage_scheduler_action(
                action,
                &harness.target,
                &mut harness.resources,
                &mut harness.runtime,
            )
            .expect("stage with invalid target")
            .is_none(),
            "invalid target must not produce a physical candidate"
        );
        stage_attempts += 1;

        match wait_after_unstable_target(
            now,
            packet_not_before,
            clock,
            &harness.resources.waiter,
            &harness.interrupt,
            &mut harness.local_metrics,
            &mut harness.runtime.terminal_error,
        ) {
            super::WaitBoundary::Due { .. } => {}
            super::WaitBoundary::Interrupted(_, _, _) => {
                let _ = harness.interrupt.try_take();
            }
            super::WaitBoundary::Exit => panic!("bounded replan wait failed"),
        }
    }

    assert!(stage_attempts >= 3, "stage attempts={stage_attempts}");
    assert!(
        stage_attempts <= 40,
        "target churn caused unbounded retries: {stage_attempts} in 20 ms"
    );
    assert_eq!(harness.resources.scheduler.accounting(), accounting_before);
    assert_eq!(
        harness.current_executor_chord_for_test().map(|chord| chord.chord_index),
        Some(0)
    );
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
}

#[test]
fn unstable_target_prewait_and_final_gate_replans_are_rate_limited() {
    for replan_site in ["prewait", "final_gate"] {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(1);
        let packets = harness.configure_mock_packet_sender_for_test(&[]);
        if replan_site == "final_gate" {
            harness.set_post_focus_revalidation_race_hook(|_, target, _, _, _, _, _| {
                let next_hwnd = match target.load_stable().map(|(hwnd, _)| hwnd) {
                    Some(1) => 2,
                    _ => 1,
                };
                target.publish(next_hwnd);
            });
        }
        let clock = harness.resources.clock;
        let epoch = sky_dispatch_win32::clock::QpcTicks::ZERO;
        harness.resources.playback.epoch = epoch;
        let start = clock.now().expect("replan test start");
        let duration = clock.duration_from_us(20_000).expect("bounded test window");
        let accounting_before = harness.resources.scheduler.accounting();
        let mut attempts = 0usize;

        loop {
            let now = clock.now().expect("replan loop QPC");
            if now
                .checked_duration_since(start)
                .expect("monotonic replan QPC")
                >= duration
            {
                break;
            }
            let action = evaluate_scheduler_action(&harness.resources.scheduler, now, epoch)
                .expect("production scheduler action");
            let deadline = match &action {
                SchedulerAction::Send { timing_window, .. } => {
                    timing_window.packet_not_before_qpc
                }
                action => panic!("eligible Down must send during target churn: {action:?}"),
            };
            let candidate = stage_scheduler_action(
                action,
                &harness.target,
                &mut harness.resources,
                &mut harness.runtime,
            )
            .expect("stage production scheduler candidate")
            .expect("valid target before injected churn");
            let retry_deadline = candidate.timing_window.packet_not_before_qpc;
            let controls = super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: None,
            };

            let outcome = if replan_site == "prewait" {
                let next_hwnd = match harness.target.load_stable().map(|(hwnd, _)| hwnd) {
                    Some(1) => 2,
                    _ => 1,
                };
                harness.target.publish(next_hwnd);
                pre_wait_candidate_admission(
                    &candidate,
                    &harness.target,
                    harness.config.focus.require_focus,
                    &harness.focus_active,
                    controls,
                    &mut harness.runtime,
                )
            } else {
                harness.dispatch_staged_candidate_for_test(candidate, false)
            };
            assert_eq!(
                outcome,
                super::StagedDispatchOutcome::Replan,
                "{replan_site}: target churn must reject each candidate"
            );
            attempts += 1;

            match wait_after_unstable_target(
                now,
                if replan_site == "prewait" { deadline } else { retry_deadline },
                clock,
                &harness.resources.waiter,
                &harness.interrupt,
                &mut harness.local_metrics,
                &mut harness.runtime.terminal_error,
            ) {
                super::WaitBoundary::Due { .. } => {}
                super::WaitBoundary::Interrupted(_, _, _) => {
                    let _ = harness.interrupt.try_take();
                    panic!("unexpected control interrupt during {replan_site} churn");
                }
                super::WaitBoundary::Exit => panic!("{replan_site} bounded wait failed"),
            }
        }

        assert!(attempts >= 2, "{replan_site}: attempts={attempts}");
        assert!(
            attempts <= 40,
            "{replan_site}: target churn caused unbounded retries: {attempts} in 20 ms"
        );
        assert_eq!(
            packets.lock().expect("churn packet capture").len(),
            0,
            "{replan_site}: no Down is sent before stable target admission"
        );
        assert_eq!(harness.resources.scheduler.accounting(), accounting_before);
        assert_eq!(harness.resources.backend.in_flight_mask, 0);
        assert_eq!(
            harness.current_executor_chord_for_test().map(|chord| chord.chord_index),
            Some(0)
        );
    }
}

#[test]
fn unstable_target_replan_wait_keeps_stop_pause_and_lease_interruptible() {
    for signal_kind in ["stop", "pause", "lease"] {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(1);
        let clock = harness.resources.clock;
        let now = clock.now().expect("test now");
        let deadline = now
            .checked_add_duration(clock.duration_from_us(5_000_000).expect("long wait"))
            .expect("deadline");
        let accounting_before = harness.resources.scheduler.accounting();
        match signal_kind {
            "stop" => harness.quit_requested.store(true, std::sync::atomic::Ordering::Release),
            "pause" => harness
                .desired_pause
                .store(true, std::sync::atomic::Ordering::Release),
            "lease" => harness.supervisor_expired.latch_expired(),
            _ => unreachable!(),
        }
        assert!(harness.interrupt.signal());

        assert!(matches!(
            wait_after_unstable_target(
                now,
                deadline,
                clock,
                &harness.resources.waiter,
                &harness.interrupt,
                &mut harness.local_metrics,
                &mut harness.runtime.terminal_error,
            ),
            super::WaitBoundary::Interrupted(_, _, _)
        ));
        match signal_kind {
            "stop" | "lease" => assert!(harness.process_command_control_for_test()),
            "pause" => assert!(harness.desired_pause.load(std::sync::atomic::Ordering::Acquire)),
            _ => unreachable!(),
        }
        assert_eq!(harness.resources.scheduler.accounting(), accounting_before);
        assert_eq!(harness.resources.backend.in_flight_mask, 0);
    }
}

#[test]
fn mixed_preflight_target_churn_sends_due_up_only_and_leaves_down_uncommitted() {
    let (mut harness, packets) = mixed_scheduler_after_accepted_a_down();
    harness.runtime.verified_target = None;
    let target = std::sync::Arc::clone(&harness.target);
    harness.set_preflight_physical_mask_probe_for_test(move || {
        target.publish(2);
        0
    });

    let action = next_mixed_action(&harness);
    assert!(matches!(
        &action,
        SchedulerAction::Send {
            up_mask: 1,
            down_chord_index: Some(1),
            ..
        }
    ));
    let controls = super::FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: None,
    };
    let fallback = stage_scheduler_action_preserving_due_up(
        action,
        &harness.target,
        harness.config.focus.require_focus,
        &harness.focus_active,
        controls,
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("replan stale Mixed candidate")
    .expect("due A Up fallback");
    assert_eq!(fallback.packet_masks.up_mask, 1);
    assert_eq!(fallback.packet_masks.down_mask, 0);

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(fallback, false),
        super::StagedDispatchOutcome::Dispatched
    ));
    let captured = packets.lock().expect("Mixed packets");
    assert_eq!(
        captured
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1), PhysicalPacket::new(1, 0)]
    );
    drop(captured);
    let accounting = harness.resources.scheduler.accounting();
    assert_eq!(accounting.activated, 1);
    assert_eq!(accounting.released, 1);
    assert_eq!(accounting.active, 0);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.chord_index),
        Some(1),
        "the unrelated B Down remains current and uncommitted"
    );
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
}

#[test]
fn mixed_up_only_fallback_keeps_the_up_deadline_when_down_is_later() {
    let (mut harness, _) = mixed_scheduler_after_accepted_a_down();
    harness.runtime.verified_target = None;
    let target = std::sync::Arc::clone(&harness.target);
    harness.resources.backend.set_preflight_physical_mask_probe(move || {
        target.publish(2);
        0
    });

    let action = next_mixed_action(&harness);
    let (up_mask, down_chord_index, mut timing_window) = match action {
        SchedulerAction::Send {
            up_mask,
            down_chord_index: Some(down_chord_index),
            timing_window,
        } if up_mask != 0 => (up_mask, down_chord_index, timing_window),
        action => panic!("expected a production Mixed action, got {action:?}"),
    };
    let epoch_qpc = harness.resources.playback.epoch;
    let up_slots = harness.resources.scheduler.up_slot_timings(up_mask);
    assert!(!up_slots.is_empty(), "the scheduler owns a due Up slot");
    let up_deadline = up_slots
        .iter()
        .map(|slot| {
            epoch_qpc
                .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
                    slot.authored_up_offset_ticks.as_u64(),
                ))
                .expect("authored Up QPC")
                .max(slot.release_not_before_qpc)
        })
        .max()
        .expect("Up deadline");
    let authored_up_target = up_slots
        .iter()
        .map(|slot| {
            epoch_qpc
                .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
                    slot.authored_up_offset_ticks.as_u64(),
                ))
                .expect("authored Up target")
        })
        .min()
        .expect("authored Up target");
    let later_down_deadline = harness
        .resources
        .clock
        .now()
        .expect("current QPC")
        .checked_add_duration(
            harness
                .resources
                .clock
                .duration_from_us(5_000_000)
                .expect("later Down floor"),
        )
        .expect("later Down QPC");
    timing_window.down_not_before_qpc = later_down_deadline;
    timing_window.packet_not_before_qpc =
        timing_window.musical_up_not_before_qpc.max(later_down_deadline);

    let controls = super::FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: None,
    };
    let fallback = stage_scheduler_action_preserving_due_up(
        SchedulerAction::Send {
            up_mask,
            down_chord_index: Some(down_chord_index),
            timing_window,
        },
        &harness.target,
        harness.config.focus.require_focus,
        &harness.focus_active,
        controls,
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("replan stale Mixed candidate")
    .expect("due Up-only fallback");

    assert_eq!(fallback.packet_masks, PhysicalPacket::new(up_mask, 0));
    assert_eq!(fallback.timing_window.authored_target_qpc, authored_up_target);
    assert_eq!(fallback.timing_window.packet_not_before_qpc, up_deadline);
    assert_eq!(
        fallback.timing_window.packet_not_before_qpc,
        fallback.timing_window.musical_up_not_before_qpc
    );
    assert_eq!(
        fallback.timing_window.down_not_before_qpc,
        fallback.timing_window.authored_target_qpc
    );
    assert_eq!(fallback.timing_window.release_floor_mask, 0);
    assert!(fallback.timing_window.packet_not_before_qpc < later_down_deadline);
    assert_eq!(harness.resources.scheduler.accounting().activated, 1);
    assert_eq!(harness.resources.scheduler.accounting().released, 0);
    assert_eq!(
        harness.current_executor_chord_for_test().map(|chord| chord.chord_index),
        Some(down_chord_index),
        "the unrelated Down remains uncommitted"
    );
}

#[test]
fn mixed_prewait_target_churn_replans_as_up_only() {
    let (mut harness, packets) = mixed_scheduler_after_accepted_a_down();
    let candidate = stage_scheduler_action(
        next_mixed_action(&harness),
        &harness.target,
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage production Mixed candidate")
    .expect("production Mixed candidate");
    harness.target.publish(2);

    assert_eq!(
        pre_wait_candidate_admission(
            &candidate,
            &harness.target,
            harness.config.focus.require_focus,
            &harness.focus_active,
            super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: None,
            },
            &mut harness.runtime,
        ),
        super::StagedDispatchOutcome::Replan
    );
    let fallback = stage_up_only_for_stale_mixed_candidate(
        &candidate,
        &harness.target,
        harness.config.focus.require_focus,
        &harness.focus_active,
        super::FinalControlSignals {
            quit_requested: &harness.quit_requested,
            skip_requested: &harness.skip_requested,
            panic_requested: &harness.panic_requested,
            desired_pause: &harness.desired_pause,
            supervisor_expired: &harness.supervisor_expired,
            system_power: None,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("pre-wait Mixed target fallback")
    .expect("due A Up fallback");
    assert_eq!(fallback.packet_masks, PhysicalPacket::new(1, 0));
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(fallback, false),
        super::StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(
        packets
            .lock()
            .expect("pre-wait packet capture")
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1), PhysicalPacket::new(1, 0)]
    );
    assert_eq!(harness.resources.scheduler.accounting().activated, 1);
    assert_eq!(harness.resources.scheduler.accounting().released, 1);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.chord_index),
        Some(1)
    );
}

#[test]
fn mixed_final_gate_target_churn_replans_as_up_only() {
    let (mut harness, packets) = mixed_scheduler_after_accepted_a_down();
    let candidate = stage_scheduler_action(
        next_mixed_action(&harness),
        &harness.target,
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage production Mixed candidate")
    .expect("production Mixed candidate");
    harness.set_post_focus_revalidation_race_hook(|_, target, _, _, _, _, _| {
        target.publish(2);
    });

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        super::StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(
        packets
            .lock()
            .expect("final-gate packet capture")
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1), PhysicalPacket::new(1, 0)]
    );
    let accounting = harness.resources.scheduler.accounting();
    assert_eq!(accounting.activated, 1);
    assert_eq!(accounting.released, 1);
    assert_eq!(accounting.active, 0);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.chord_index),
        Some(1)
    );
}

#[test]
fn mixed_target_churn_fallback_honors_pause_stop_and_lease_controls() {
    for signal in ["pause", "stop", "lease"] {
        let (mut harness, packets) = mixed_scheduler_after_accepted_a_down();
        let candidate = stage_scheduler_action(
            next_mixed_action(&harness),
            &harness.target,
            &mut harness.resources,
            &mut harness.runtime,
        )
        .expect("stage production Mixed candidate")
        .expect("production Mixed candidate");
        harness.target.publish(2);
        match signal {
            "pause" => harness
                .desired_pause
                .store(true, std::sync::atomic::Ordering::Release),
            "stop" => harness
                .quit_requested
                .store(true, std::sync::atomic::Ordering::Release),
            "lease" => harness.supervisor_expired.latch_expired(),
            _ => unreachable!(),
        }

        assert!(
            stage_up_only_for_stale_mixed_candidate(
                &candidate,
                &harness.target,
                harness.config.focus.require_focus,
                &harness.focus_active,
                super::FinalControlSignals {
                    quit_requested: &harness.quit_requested,
                    skip_requested: &harness.skip_requested,
                    panic_requested: &harness.panic_requested,
                    desired_pause: &harness.desired_pause,
                    supervisor_expired: &harness.supervisor_expired,
                    system_power: None,
                },
                &mut harness.resources,
                &mut harness.runtime,
            )
            .expect("control-gated Mixed fallback")
            .is_none(),
            "{signal}: control state must block the Up-only dispatch"
        );
        assert_eq!(
            packets.lock().expect("pre-cleanup packet capture").len(),
            1,
            "{signal}: no fallback packet before lifecycle cleanup"
        );
        assert_eq!(harness.resources.scheduler.accounting().active, 1);
        assert_ne!(harness.release_obligation_mask_for_test(), 0);

        let cleanup = harness.configure_mock_cleanup_emitter_for_test();
        if signal == "pause" {
            harness
                .apply_scheduler_resumable_lifecycle_transition_for_test()
                .expect("pause cleanup uses production scheduler");
        } else {
            assert!(harness.process_command_control_for_test());
            let mut observation = crate::engine::FinalizeTestObservation::default();
            let outcome = harness.finalize_worker_for_test(false, &mut observation);
            if signal == "stop" {
                assert_eq!(outcome, crate::engine::OUTCOME_QUIT);
            } else {
                assert_eq!(outcome, crate::engine::OUTCOME_ERROR);
            }
            assert_eq!(observation.release_obligation_mask, 0, "{signal}");
            assert_eq!(observation.active_mask, 0, "{signal}");
            assert_eq!(observation.possibly_active_mask, 0, "{signal}");
            assert_eq!(observation.generation_accounting.active, 0, "{signal}");
            assert_eq!(
                cleanup.lock().expect("terminal safety Up capture").as_slice(),
                &[(vec![0x15], true)],
                "{signal}: terminal cleanup releases A"
            );
            continue;
        }
        assert_eq!(harness.release_obligation_mask_for_test(), 0, "{signal}");
        assert_eq!(harness.resources.backend.active_mask, 0, "{signal}");
        assert_eq!(
            cleanup.lock().expect("safety Up capture").as_slice(),
            &[(vec![0x15], true)],
            "{signal}: accepted A Down is released by lifecycle cleanup"
        );
    }
}
