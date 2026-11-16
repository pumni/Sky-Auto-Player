fn dispatch_while_final_authority_mutex_is_held(
    harness: &mut ProductionDispatchTestHarness,
    candidate: StagedCandidate,
) -> (bool, StagedDispatchOutcome) {
    let (gate_reached_tx, gate_reached_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let continue_rx = Arc::new(Mutex::new(continue_rx));
    let continue_rx_for_hook = Arc::clone(&continue_rx);
    harness.set_final_gate_before_owner_permit_hook(move || {
        gate_reached_tx
            .send(())
            .expect("test receiver remains alive at final permit gate");
        continue_rx_for_hook
            .lock()
            .expect("final permit test gate lock")
            .recv()
            .expect("test releases final permit gate");
    });

    let target = Arc::clone(&harness.target);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let supervisor = std::thread::spawn(move || {
        // The final-gate hook is reached only after fresh foreground/PID/
        // process verification. Acquire the authority mutex after that signal.
        if gate_reached_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_err()
        {
            let _ = continue_tx.send(());
            return false;
        }
        let authority_guard = target.lock_owner_authority_for_test();
        let _ = continue_tx.send(());

        // This timeout only bounds deadlock detection. Normally the worker
        // returns Replan and asks this supervisor thread to release the lock.
        let released_by_worker = release_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .is_ok();
        drop(authority_guard);
        released_by_worker
    });

    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    let _ = release_tx.send(());
    let returned_without_waiting = supervisor
        .join()
        .expect("authority supervisor test thread joins");
    (returned_without_waiting, outcome)
}

struct ForegroundTestReset;

impl Drop for ForegroundTestReset {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
    }
}

fn owner_harness_after_first_down(
    actions: &[KeyActionInput],
) -> (
    ProductionDispatchTestHarness,
    crate::engine::test_support::PacketCapture,
    QpcTicks,
) {
    let mut harness = ProductionDispatchTestHarness::create_harness(actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");
    let first = candidate_at(&mut harness, epoch, epoch);
    assert_eq!(first.packet_masks, PhysicalPacket::new(0, 1));
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));
    (harness, packets, epoch)
}

fn assert_musical_up_rejected_after_rebind(republish_same_hwnd: bool) {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(10, ActionKind::Down, 0, 0x15),
        test_action(11, ActionKind::Up, 20_000, 0x15),
    ];
    let (mut harness, packets, epoch) = owner_harness_after_first_down(&actions);
    let new_hwnd = if republish_same_hwnd { 1 } else { 2 };
    let new_pid = if republish_same_hwnd { 101 } else { 202 };
    harness.republish_focus_owner_for_test(new_hwnd, new_pid);

    let up_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_at);
    let up = candidate_at(&mut harness, up_at, epoch);
    assert_eq!(up.packet_masks, PhysicalPacket::new(1, 0));
    harness.set_mock_sender_start_qpc_for_test(up_at);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Exit
    ));

    let captured = packets.lock().expect("mock sender capture");
    assert_eq!(captured.len(), 1, "no Up was sent to the new owner");
    assert_eq!(captured[0].0, PhysicalPacket::new(0, 1));
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert!(harness
        .runtime
        .terminal_error
        .as_deref()
        .is_some_and(|error| error.contains("musical Up origin owner differs")));
}

#[test]
fn musical_up_from_a_is_withheld_after_rebind_to_b() {
    assert_musical_up_rejected_after_rebind(false);
}

#[test]
fn musical_up_from_a_is_withheld_after_same_hwnd_generation_aba() {
    assert_musical_up_rejected_after_rebind(true);
}

#[test]
fn mixed_up_a_down_b_is_withheld_without_consuming_down_b() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(20, ActionKind::Down, 0, 0x15),
        test_action(21, ActionKind::Up, 20_000, 0x15),
        test_action(22, ActionKind::Down, 20_000, 0x16),
        test_action(23, ActionKind::Up, 40_000, 0x16),
    ];
    let (mut harness, packets, epoch) = owner_harness_after_first_down(&actions);
    harness.republish_focus_owner_for_test(2, 202);

    let mixed_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, mixed_at);
    let mixed = candidate_at(&mut harness, mixed_at, epoch);
    assert_eq!(mixed.packet_masks, PhysicalPacket::new(1, 2));
    harness.set_mock_sender_start_qpc_for_test(mixed_at);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(mixed, false),
        StagedDispatchOutcome::Exit
    ));

    let captured = packets.lock().expect("mock sender capture");
    assert_eq!(captured.len(), 1, "Down(B) was not attempted");
    assert_eq!(captured[0].0, PhysicalPacket::new(0, 1));
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.scheduler.current_chord().map(|chord| chord.source_action_index), Some(22));
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert!(harness
        .runtime
        .terminal_error
        .as_deref()
        .is_some_and(|error| error.contains("musical Up origin owner differs")));
}

#[test]
fn mixed_to_up_only_modifier_replan_keeps_origin_a_and_withholds_up_on_b() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(30, ActionKind::Down, 0, 0x15),
        test_action(31, ActionKind::Up, 20_000, 0x15),
        test_action(32, ActionKind::Down, 20_000, 0x16),
        test_action(33, ActionKind::Up, 40_000, 0x16),
    ];
    let (mut harness, packets, epoch) = owner_harness_after_first_down(&actions);
    let mixed_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, mixed_at);
    let mixed = candidate_at(&mut harness, mixed_at, epoch);
    assert_eq!(mixed.packet_masks, PhysicalPacket::new(1, 2));

    let target = Arc::clone(&harness.target);
    let swapped = Arc::new(AtomicBool::new(false));
    let hook_swapped = Arc::clone(&swapped);
    harness.set_final_gate_before_syscall_hook(move || {
        if !hook_swapped.swap(true, Ordering::AcqRel) {
            assert!(target.publish_for_test(2));
            let (hwnd, generation) = target.load_stable().expect("new binding snapshot");
            assert!(target.bind_owner_identity_authority(
                hwnd,
                generation,
                202,
                Arc::new(
                    sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                        hwnd, 202,
                    ),
                ),
            ));
            sky_dispatch_win32::focus::set_foreground_window_for_test(Some(hwnd));
            sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
        }
    });
    harness
        .resources
        .backend
        .set_modifier_key_state_query_for_test(|_| i16::MIN);
    harness.set_mock_sender_start_qpc_for_test(mixed_at);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(mixed, false),
        StagedDispatchOutcome::Exit
    ));

    let captured = packets.lock().expect("mock sender capture");
    assert_eq!(captured.len(), 1, "neither mixed Up nor fallback Up was sent");
    assert_eq!(captured[0].0, PhysicalPacket::new(0, 1));
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert!(harness
        .runtime
        .terminal_error
        .as_deref()
        .is_some_and(|error| error.contains("musical Up origin owner differs")));
}

#[test]
fn generation_change_after_final_identity_query_fails_before_sender() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(40, ActionKind::Down, 0, 0x15),
        test_action(41, ActionKind::Up, 20_000, 0x15),
    ];
    let (mut harness, packets, epoch) = owner_harness_after_first_down(&actions);
    let up_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_at);
    let up = candidate_at(&mut harness, up_at, epoch);
    let target = Arc::clone(&harness.target);
    harness.set_final_gate_before_owner_permit_hook(move || {
        assert!(target.publish_for_test(2));
        let (hwnd, generation) = target.load_stable().expect("new target generation");
        assert!(target.bind_owner_identity_authority(
            hwnd,
            generation,
            202,
            Arc::new(
                sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                    hwnd, 202,
                ),
            ),
        ));
    });
    harness.set_mock_sender_start_qpc_for_test(up_at);

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Replan
    ));
    assert_eq!(packets.lock().expect("mock sender capture").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
}

#[test]
fn same_owner_independent_ups_keep_separate_origin_obligations() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(50, ActionKind::Down, 0, 0x15),
        test_action(51, ActionKind::Down, 5_000, 0x16),
        test_action(52, ActionKind::Up, 20_000, 0x15),
        test_action(53, ActionKind::Up, 30_000, 0x16),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let down_a = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(down_a, false),
        StagedDispatchOutcome::Dispatched
    ));

    let down_b_at = target_at(&harness, epoch, 5_000);
    wait_until_qpc(&harness, down_b_at);
    let down_b = candidate_at(&mut harness, down_b_at, epoch);
    assert_eq!(down_b.packet_masks, PhysicalPacket::new(0, 2));
    harness.set_mock_sender_start_qpc_for_test(down_b_at);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(down_b, false),
        StagedDispatchOutcome::Dispatched
    ));

    let up_a_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_a_at);
    let up_a = candidate_at(&mut harness, up_a_at, epoch);
    assert_eq!(up_a.packet_masks, PhysicalPacket::new(1, 0));
    harness.set_mock_sender_start_qpc_for_test(up_a_at);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up_a, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(harness.release_obligation_mask_for_test(), 2);

    let up_b_at = target_at(&harness, epoch, 30_000);
    wait_until_qpc(&harness, up_b_at);
    let up_b = candidate_at(&mut harness, up_b_at, epoch);
    assert_eq!(up_b.packet_masks, PhysicalPacket::new(2, 0));
    harness.set_mock_sender_start_qpc_for_test(up_b_at);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up_b, false),
        StagedDispatchOutcome::Dispatched
    ));

    let captured = packets.lock().expect("mock sender capture");
    assert_eq!(captured.iter().map(|entry| entry.0).collect::<Vec<_>>(), [
        PhysicalPacket::new(0, 1),
        PhysicalPacket::new(0, 2),
        PhysicalPacket::new(1, 0),
        PhysicalPacket::new(2, 0),
    ]);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
}

#[test]
fn musical_down_final_permit_replans_without_waiting_for_owner_authority_mutex() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(60, ActionKind::Down, 0, 0x15),
        test_action(61, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");
    let down = candidate_at(&mut harness, epoch, epoch);
    assert_eq!(down.packet_masks, PhysicalPacket::new(0, 1));
    harness.set_mock_sender_start_qpc_for_test(epoch);

    let (returned_while_locked, outcome) =
        dispatch_while_final_authority_mutex_is_held(&mut harness, down);
    assert!(returned_while_locked, "musical permit waited for authority mutex");
    assert_eq!(
        outcome,
        StagedDispatchOutcome::Replan
    );
    assert!(packets.lock().expect("mock packet capture").is_empty());
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.active_mask, 0);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);

    // Re-stage and re-prove after releasing the mutex; no cached proof is
    // carried from the rejected final permit attempt.
    harness.set_final_gate_before_owner_permit_hook(|| {});
    let retry = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(retry, false),
        StagedDispatchOutcome::Dispatched
    );
    let sent = packets.lock().expect("mock packet capture");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, PhysicalPacket::new(0, 1));
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
}

#[test]
fn musical_up_final_permit_replans_without_waiting_then_sends_once() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(62, ActionKind::Down, 0, 0x15),
        test_action(63, ActionKind::Up, 20_000, 0x15),
    ];
    let (mut harness, packets, epoch) = owner_harness_after_first_down(&actions);
    let up_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_at);
    let up = candidate_at(&mut harness, up_at, epoch);
    assert_eq!(up.packet_masks, PhysicalPacket::new(1, 0));
    harness.set_mock_sender_start_qpc_for_test(up_at);

    let (returned_while_locked, outcome) =
        dispatch_while_final_authority_mutex_is_held(&mut harness, up);
    assert!(returned_while_locked, "musical permit waited for authority mutex");
    assert_eq!(
        outcome,
        StagedDispatchOutcome::Replan
    );
    assert_eq!(packets.lock().expect("mock packet capture").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);

    harness.set_final_gate_before_owner_permit_hook(|| {});
    let retry = candidate_at(&mut harness, up_at, epoch);
    harness.set_mock_sender_start_qpc_for_test(up_at);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(retry, false),
        StagedDispatchOutcome::Dispatched
    );
    let sent = packets.lock().expect("mock packet capture");
    assert_eq!(sent.len(), 2, "healthy Up is emitted once after replan");
    assert_eq!(sent[1].0, PhysicalPacket::new(1, 0));
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
}

#[test]
fn owner_generation_publication_during_authority_contention_never_borrows_new_proof() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(64, ActionKind::Down, 0, 0x15),
        test_action(65, ActionKind::Up, 20_000, 0x15),
    ];
    let (mut harness, packets, epoch) = owner_harness_after_first_down(&actions);
    let up_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_at);
    let up = candidate_at(&mut harness, up_at, epoch);
    assert_eq!(up.packet_masks, PhysicalPacket::new(1, 0));
    harness.set_mock_sender_start_qpc_for_test(up_at);

    let (gate_reached_tx, gate_reached_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let continue_rx = Arc::new(Mutex::new(continue_rx));
    let continue_rx_for_hook = Arc::clone(&continue_rx);
    harness.set_final_gate_before_owner_permit_hook(move || {
        gate_reached_tx
            .send(())
            .expect("test receiver remains alive at final permit gate");
        continue_rx_for_hook
            .lock()
            .expect("final permit test gate lock")
            .recv()
            .expect("test releases final permit gate");
    });

    let target_for_supervisor = Arc::clone(&harness.target);
    let (release_supervisor_tx, release_supervisor_rx) = std::sync::mpsc::channel();
    let supervisor = std::thread::spawn(move || {
        if gate_reached_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_err()
        {
            let _ = continue_tx.send(());
            return false;
        }
        let authority_guard = target_for_supervisor.lock_owner_authority_for_test();
        let _ = continue_tx.send(());
        let released_by_test = release_supervisor_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_ok();
        drop(authority_guard);
        released_by_test
    });

    // The test worker stays on this thread because its scheduling guard is
    // deliberately !Send. Only the control-plane lock holder is concurrent.
    let first_attempt = harness.dispatch_staged_candidate_for_test(up, false);

    let target_for_publisher = Arc::clone(&harness.target);
    let (before_authority_reset_tx, before_authority_reset_rx) =
        std::sync::mpsc::channel();
    let (publish_complete_tx, publish_complete_rx) = std::sync::mpsc::channel();
    let publisher = std::thread::spawn(move || {
        let published = target_for_publisher.publish_with_hook_for_test(2, |point| {
            if point == crate::engine::target::TargetPublicationWritePoint::BeforeOwnerAuthorityReset
            {
                before_authority_reset_tx
                    .send(())
                    .expect("publication test receiver remains alive");
            }
        });
        let owner_bound = if published {
            let (hwnd, generation) = target_for_publisher
                .load_stable()
                .expect("new target generation is stable");
            target_for_publisher.bind_owner_identity_authority(
                hwnd,
                generation,
                202,
                Arc::new(
                    sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                        hwnd, 202,
                    ),
                ),
            )
        } else {
            false
        };
        publish_complete_tx
            .send((published, owner_bound))
            .expect("publication test receiver remains alive");
    });

    let publisher_reached_authority_reset = before_authority_reset_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .is_ok();
    // Publication owns the writer and is now stopped immediately before it
    // must acquire the held authority mutex; no stable B generation exists.
    let target_was_unstable = harness.target.load_stable().is_none();
    let _ = release_supervisor_tx.send(());
    let supervisor_released = supervisor
        .join()
        .expect("authority supervisor test thread joins");
    let publication = publish_complete_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("target publication completes after authority unlock");
    publisher.join().expect("target publisher test thread joins");

    assert_eq!(first_attempt, StagedDispatchOutcome::Replan);
    assert!(supervisor_released, "final permit did not return while authority was held");
    assert!(publisher_reached_authority_reset);
    assert!(target_was_unstable, "B was not held behind authority reset");
    assert_eq!(publication, (true, true));
    assert_eq!(harness.target.load_stable(), Some((2, 1)));
    assert_eq!(packets.lock().expect("mock packet capture").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);

    // A newly staged authored Up still carries A's ledger origin. Fresh B
    // admission cannot substitute B's authority for that release obligation.
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
    harness.set_final_gate_before_owner_permit_hook(|| {});
    let rebound_up = candidate_at(&mut harness, up_at, epoch);
    harness.set_mock_sender_start_qpc_for_test(up_at);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(rebound_up, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(packets.lock().expect("mock packet capture").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert!(harness
        .runtime
        .terminal_error
        .as_deref()
        .is_some_and(|error| error.contains("musical Up origin owner differs")));
}

fn assert_missing_or_conflicted_up_origin_fails_closed(conflicted: bool) {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let actions = [
        test_action(60, ActionKind::Down, 0, 0x15),
        test_action(61, ActionKind::Up, 20_000, 0x15),
    ];
    let (mut harness, packets, epoch) = owner_harness_after_first_down(&actions);
    if conflicted {
        harness
            .resources
            .backend
            .conflict_release_owner_proof_for_test(1);
    } else {
        harness
            .resources
            .backend
            .forget_release_owner_proof_for_test(1);
    }

    let up_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_at);
    let up = candidate_at(&mut harness, up_at, epoch);
    harness.set_mock_sender_start_qpc_for_test(up_at);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Exit
    ));
    assert_eq!(packets.lock().expect("mock sender capture").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    let error = harness
        .runtime
        .terminal_error
        .as_deref()
        .expect("clear terminal owner diagnostic");
    assert!(error.contains("musical Up owner proof is missing or conflicted"));
    assert!(error.contains(if conflicted { "ambiguous" } else { "missing" }));
}

#[test]
fn missing_up_owner_proof_fails_closed_with_terminal_diagnostic() {
    assert_missing_or_conflicted_up_origin_fails_closed(false);
}

#[test]
fn conflicted_up_owner_proof_fails_closed_with_terminal_diagnostic() {
    assert_missing_or_conflicted_up_origin_fails_closed(true);
}
