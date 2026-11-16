use crate::engine::worker::StagedDispatchOutcome;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::QpcTicks;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn timing_action(index: u32, kind: ActionKind, scheduled_us: u64) -> KeyActionInput {
    KeyActionInput {
        source_action_index: index,
        kind,
        scheduled_us,
        scan_codes: vec![0x15].into(),
        reason: format!("replan-timing-{index}").into(),
    }
}

fn timing_owner_harness(
    actions: &[KeyActionInput],
    sender_latencies_us: &[u64],
) -> (
    ProductionDispatchTestHarness,
    crate::engine::test_support::PacketCapture,
    QpcTicks,
) {
    let mut harness = ProductionDispatchTestHarness::create_harness(actions);
    let epoch = harness.qpc_now_for_test().expect("test QPC epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback clock");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(sender_latencies_us);
    harness
        .attach_executor_for_test()
        .expect("production executor invariant");
    (harness, packets, epoch)
}

fn timing_harness_without_focus(
    actions: &[KeyActionInput],
    sender_latencies_us: &[u64],
) -> (
    ProductionDispatchTestHarness,
    crate::engine::test_support::PacketCapture,
    QpcTicks,
) {
    let mut harness = ProductionDispatchTestHarness::create_harness(actions);
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

fn stage_from_worker_outer_evaluation(
    harness: &mut ProductionDispatchTestHarness,
    now_qpc: QpcTicks,
) -> (crate::engine::worker::StagedCandidate, u64) {
    super::reset_scheduler_evaluation_count_for_test();
    let epoch = harness.resources.playback.epoch;
    let (action, _scheduler_wait_target_qpc, action_to_stage) =
        super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            now_qpc,
            epoch,
        )
        .expect("worker scheduler evaluation");
    assert!(matches!(
        action,
        super::SchedulerAction::Send { .. }
    ));
    let send_action = action_to_stage.expect("due scheduler action stages a packet");
    let candidate = super::stage_scheduler_action_preserving_due_up(
        send_action,
        super::SchedulerStageAdmission {
            target: &harness.target,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            capture_up_slot_timings: false,
            control_signals: super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: Some(&harness.system_power),
            },
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage worker packet")
    .expect("scheduler candidate is stageable");
    let count = super::scheduler_evaluation_count_for_test();
    (candidate, count)
}

struct OuterIterationEvidence {
    eval_qpc: QpcTicks,
    eval_count: u64,
    dispatch_return_qpc: QpcTicks,
    outcome: super::DispatchAttemptOutcome,
    replan_action: Option<super::DispatchReplanAction>,
}

fn apply_outer_replan_policy(
    harness: &mut ProductionDispatchTestHarness,
    outcome: super::DispatchAttemptOutcome,
    expected_target: Option<super::TargetStamp>,
    retry_deadline_qpc: QpcTicks,
    permit_busy_retry_allowed: bool,
    permit_busy_retry_pending: &mut bool,
) -> super::DispatchReplanAction {
    super::wait_after_dispatch_replan(
        outcome,
        permit_busy_retry_allowed,
        permit_busy_retry_pending,
        super::DispatchReplanWaitContext {
            target: &harness.target,
            expected_target,
            retry_deadline_qpc,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            control_signals: super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: Some(&harness.system_power),
            },
            qpc_clock: harness.resources.clock,
            waiter: &harness.resources.waiter,
            interrupt: &harness.interrupt,
            local_metrics: &mut harness.local_metrics,
            terminal_error: &mut harness.runtime.terminal_error,
        },
    )
}

fn run_worker_outer_iteration(
    harness: &mut ProductionDispatchTestHarness,
    permit_busy_retry_pending: &mut bool,
) -> OuterIterationEvidence {
    let permit_busy_retry_allowed = std::mem::take(permit_busy_retry_pending);
    let eval_qpc = harness.qpc_now_for_test().expect("outer-loop QPC");
    let (candidate, eval_count) = stage_from_worker_outer_evaluation(harness, eval_qpc);
    assert!(
        candidate.timing_window.packet_not_before_qpc <= eval_qpc,
        "the due-onset regression must enter final admission without a deadline wait"
    );
    let candidate_target = candidate.target_stamp;
    let candidate_retry_deadline = candidate.timing_window.packet_not_before_qpc;
    harness.set_mock_sender_start_qpc_for_test(
        harness.qpc_now_for_test().expect("mock sender start QPC"),
    );
    let outcome = harness.dispatch_staged_candidate_with_reason_for_test(candidate, false);
    let dispatch_return_qpc = harness.qpc_now_for_test().expect("dispatch return QPC");
    let replan_action = if matches!(
        outcome,
        super::DispatchAttemptOutcome::Replan | super::DispatchAttemptOutcome::PermitBusy
    ) {
        Some(apply_outer_replan_policy(
            harness,
            outcome,
            candidate_target,
            candidate_retry_deadline,
            permit_busy_retry_allowed,
            permit_busy_retry_pending,
        ))
    } else {
        None
    };
    OuterIterationEvidence {
        eval_qpc,
        eval_count,
        dispatch_return_qpc,
        outcome,
        replan_action,
    }
}

fn elapsed_us(
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

struct ForegroundReset;

impl Drop for ForegroundReset {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
    }
}

#[test]
fn due_down_owner_permit_contention_uses_outer_loop_retry_and_bounded_wait() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let actions = [
        timing_action(6660, ActionKind::Down, 0),
        timing_action(6661, ActionKind::Up, 1_000_000),
    ];
    let (mut harness, packets, _epoch) = timing_owner_harness(&actions, &[]);

    // The hook runs after fresh foreground/PID/process identity admission and
    // immediately before the final writer/authority permit. The supervisor
    // takes the authority mutex only in that suffix, then releases it after
    // each failed permit so the next iteration can acquire a fresh proof.
    let (permit_gate_tx, permit_gate_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let continue_rx = Arc::new(Mutex::new(continue_rx));
    let continue_rx_for_hook = Arc::clone(&continue_rx);
    harness.set_final_gate_before_owner_permit_hook(move || {
        permit_gate_tx
            .send(())
            .expect("supervisor remains at final owner permit");
        continue_rx_for_hook
            .lock()
            .expect("test permit barrier lock")
            .recv_timeout(Duration::from_secs(5))
            .expect("supervisor releases final permit barrier");
    });

    let target_for_supervisor = Arc::clone(&harness.target);
    let (release_attempt_tx, release_attempt_rx) = std::sync::mpsc::channel();
    let (attempt_released_tx, attempt_released_rx) = std::sync::mpsc::channel();
    let (supervisor_ready_tx, supervisor_ready_rx) = std::sync::mpsc::channel();
    let supervisor = std::thread::spawn(move || {
        supervisor_ready_tx
            .send(())
            .expect("worker receives supervisor readiness");
        for attempt in 1..=3 {
            if permit_gate_rx
                .recv_timeout(Duration::from_secs(5))
                .is_err()
            {
                return false;
            }
            if attempt <= 2 {
                let authority_guard = target_for_supervisor.lock_owner_authority_for_test();
                if continue_tx.send(()).is_err() {
                    return false;
                }
                if release_attempt_rx
                    .recv_timeout(Duration::from_secs(5))
                    .is_err()
                {
                    return false;
                }
                drop(authority_guard);
                if attempt_released_tx.send(()).is_err() {
                    return false;
                }
            } else if continue_tx.send(()).is_err() {
                return false;
            }
        }
        true
    });
    supervisor_ready_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("supervisor setup completes before worker admission");

    let mut permit_busy_retry_pending = false;
    let first = run_worker_outer_iteration(&mut harness, &mut permit_busy_retry_pending);
    assert_eq!(first.eval_count, 1, "due Send uses one scheduler evaluation");
    assert_eq!(first.outcome, super::DispatchAttemptOutcome::PermitBusy);
    assert_eq!(
        first.replan_action,
        Some(super::DispatchReplanAction::ContinueImmediately),
        "the first stable-target permit Busy re-enters the scheduler without waiting"
    );
    assert!(permit_busy_retry_pending);
    assert!(packets.lock().expect("packet capture").is_empty());
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.active_mask, 0);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    release_attempt_tx
        .send(())
        .expect("release first owner authority contention");
    attempt_released_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first owner authority lock is released before re-evaluation");

    let second = run_worker_outer_iteration(&mut harness, &mut permit_busy_retry_pending);
    assert_eq!(second.eval_count, 1);
    assert_eq!(second.outcome, super::DispatchAttemptOutcome::PermitBusy);
    let second_wait = second
        .replan_action
        .expect("repeated contention takes a bounded wait");
    if second_wait != super::DispatchReplanAction::WaitCompleted {
        panic!("repeated stable-target Busy did not back off: {second_wait:?}");
    }
    let trace = super::last_dispatch_replan_wait_for_test()
        .expect("bounded wait test trace");
    let super::DispatchReplanWaitTrace {
        retry_from_qpc,
        deadline_qpc,
        interrupted,
    } = trace;
    assert!(!interrupted);
    let expected_backoff = harness
        .resources
        .clock
        .duration_from_us(1_000)
        .expect("1 ms test backoff");
    assert_eq!(
        deadline_qpc.checked_duration_since(retry_from_qpc),
        Ok(expected_backoff),
        "the persistent Busy backoff is exactly 1 ms in QPC ticks"
    );
    assert!(packets.lock().expect("packet capture").is_empty());
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    release_attempt_tx
        .send(())
        .expect("release second owner authority contention after the bounded wait");
    attempt_released_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("second owner authority lock is released before re-evaluation");

    let third = run_worker_outer_iteration(&mut harness, &mut permit_busy_retry_pending);
    assert_eq!(third.eval_count, 1);
    assert_eq!(third.outcome, super::DispatchAttemptOutcome::Dispatched);
    assert_eq!(third.replan_action, None);
    assert!(!permit_busy_retry_pending);
    let supervisor_released = supervisor.join().expect("supervisor thread joins");
    assert!(supervisor_released, "supervisor observed all three final gates");

    let sent = packets.lock().expect("packet capture");
    assert_eq!(sent.len(), 1, "the authored Down onset is attempted once");
    assert_eq!(sent[0].0, sky_dispatch_win32::input::PhysicalPacket::new(0, 1));
    assert_eq!(sent[0].1.attempts, 1);
    assert_eq!(sent[0].1.first_inserted, 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert!(harness.runtime.terminal_error.is_none());

    let clock = harness.resources.clock;
    let sender_start = sent[0]
        .1
        .started_ticks
        .expect("mock sender records its real production-path start stamp");
    let sender_completion = sent[0]
        .1
        .completed_ticks
        .expect("mock sender records its completion stamp");
    println!(
        "outer-replan production trace: scheduler_eval=[{}@{},{}@{},{}@{}], dispatch_outcome=[PermitBusy@{},PermitBusy@{},Dispatched@{}], busy1_to_eval2={}ticks/{}us, backoff=[selected_deadline@{},duration={}us,wake_re_evaluate@{},busy2_to_re_evaluate={}us], busy2_to_sender_pre_call={}us, sender=[start@{},attempts=1,raw_inserted=1,completion@{}], journal=[before=(active:0,possibly:0,release:0,inflight:0),after_commit=(active:1,possibly:0,release:1,inflight:0)], terminal=None",
        first.eval_count, first.eval_qpc.as_u64(),
        second.eval_count, second.eval_qpc.as_u64(),
        third.eval_count, third.eval_qpc.as_u64(),
        first.dispatch_return_qpc.as_u64(),
        second.dispatch_return_qpc.as_u64(),
        third.dispatch_return_qpc.as_u64(),
        second.eval_qpc.as_u64() - first.dispatch_return_qpc.as_u64(),
        elapsed_us(clock, first.dispatch_return_qpc, second.eval_qpc),
        deadline_qpc.as_u64(), elapsed_us(clock, retry_from_qpc, deadline_qpc),
        third.eval_qpc.as_u64(), elapsed_us(clock, second.dispatch_return_qpc, third.eval_qpc),
        elapsed_us(clock, second.dispatch_return_qpc, sender_start),
        sender_start.as_u64(), sender_completion.as_u64(),
    );
}

#[test]
fn writer_mutex_busy_is_typed_as_pre_call_replan() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let actions = [
        timing_action(6665, ActionKind::Down, 0),
        timing_action(6666, ActionKind::Up, 1_000_000),
    ];
    let (mut harness, packets, _) = timing_owner_harness(&actions, &[]);
    let now = harness.qpc_now_for_test().expect("outer-loop QPC");
    let (candidate, _) = stage_from_worker_outer_evaluation(&mut harness, now);
    let (permit_gate_tx, permit_gate_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let continue_rx = Arc::new(Mutex::new(continue_rx));
    let continue_rx_for_hook = Arc::clone(&continue_rx);
    harness.set_final_gate_before_owner_permit_hook(move || {
        permit_gate_tx
            .send(())
            .expect("writer supervisor receives final gate");
        continue_rx_for_hook
            .lock()
            .expect("writer permit barrier lock")
            .recv_timeout(Duration::from_secs(5))
            .expect("writer supervisor releases final gate");
    });

    let target_for_supervisor = Arc::clone(&harness.target);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let supervisor = std::thread::spawn(move || {
        if permit_gate_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            let _ = continue_tx.send(());
            return false;
        }
        let writer_guard = target_for_supervisor.lock_writer_for_test();
        let _ = continue_tx.send(());
        let released_by_worker = release_rx
            .recv_timeout(Duration::from_secs(2))
            .is_ok();
        drop(writer_guard);
        released_by_worker
    });

    harness.set_mock_sender_start_qpc_for_test(now);
    let outcome = harness.dispatch_staged_candidate_with_reason_for_test(candidate, false);
    assert_eq!(outcome, super::DispatchAttemptOutcome::PermitBusy);
    assert!(packets.lock().expect("packet capture").is_empty());
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    release_tx.send(()).expect("release writer mutex after assertion");
    assert!(supervisor.join().expect("writer supervisor joins"));

    harness.set_final_gate_before_owner_permit_hook(|| {});
    let retry_at = harness.qpc_now_for_test().expect("fresh writer retry QPC");
    let (retry, _) = stage_from_worker_outer_evaluation(&mut harness, retry_at);
    harness.set_mock_sender_start_qpc_for_test(retry_at);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(retry, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
}
