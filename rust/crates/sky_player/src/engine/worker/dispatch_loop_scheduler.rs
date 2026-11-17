#[derive(Clone, Copy, Debug)]
enum SchedulerAction {
    Finished,
    Wait {
        deadline_qpc: sky_dispatch_win32::clock::QpcTicks,
    },
    Send {
        up_mask: u16,
        down_chord: Option<sky_dispatch_core::executor::DownChordMetadata>,
        timing_window: sky_dispatch_core::executor::DispatchTimingWindow,
    },
}

const UNSTABLE_TARGET_REPLAN_INTERVAL_US: u64 = 1_000;
const MODIFIER_REPLAN_INTERVAL_US: u64 = 1_000;

/// Sleep for one modifier re-evaluation quantum without changing musical
/// deadlines or using the precision spin phase. The shared interrupt event
/// wakes this wait for controls, lifecycle changes, or owner invalidation.
fn wait_after_modifier_held(
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
    next_active_up_deadline: Option<sky_dispatch_win32::clock::QpcTicks>,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
) -> Result<bool, String> {
    let Some(deadline) = modifier_replan_deadline(
        now_qpc,
        next_active_up_deadline,
        qpc_clock,
    )? else {
        // Re-enter the normal scheduler immediately for an already-due Up.
        // It will either dispatch that Up or take its usual final gate; no
        // zero-length waiter loop is armed here.
        return Ok(false);
    };
    let result = waiter.wait_until_ticks_with_metrics_typed(
        qpc_clock,
        deadline,
        DurationTicks::ZERO,
        interrupt,
    );
    match result.outcome {
        sky_dispatch_win32::wait::WaitOutcome::Deadline => Ok(false),
        sky_dispatch_win32::wait::WaitOutcome::Interrupted => Ok(true),
        sky_dispatch_win32::wait::WaitOutcome::Failed(failure) => {
            Err(format!("modifier-held replan wait failed: {failure:?}"))
        }
    }
}

fn modifier_replan_deadline(
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
    next_active_up_deadline: Option<sky_dispatch_win32::clock::QpcTicks>,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
) -> Result<Option<sky_dispatch_win32::clock::QpcTicks>, String> {
    let interval = qpc_clock
        .duration_from_us(MODIFIER_REPLAN_INTERVAL_US)
        .map_err(|error| format!("modifier replan interval conversion failed: {error:?}"))?;
    let poll_deadline = now_qpc
        .checked_add_duration(interval)
        .map_err(|error| format!("modifier replan deadline overflow: {error}"))?;
    let deadline = match next_active_up_deadline {
        Some(up_deadline) if up_deadline <= now_qpc => return Ok(None),
        Some(up_deadline) => poll_deadline.min(up_deadline),
        None => poll_deadline,
    };
    Ok(Some(deadline))
}

#[cfg(test)]
thread_local! {
    static SCHEDULER_EVALUATION_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_scheduler_evaluation_count_for_test() {
    SCHEDULER_EVALUATION_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn scheduler_evaluation_count_for_test() -> u64 {
    SCHEDULER_EVALUATION_COUNT.with(std::cell::Cell::get)
}

fn wait_after_unstable_target(
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
    packet_not_before_qpc: sky_dispatch_win32::clock::QpcTicks,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    local_metrics: &mut super::WorkerMetricsLocal,
    terminal_error: &mut Option<String>,
) -> WaitBoundary {
    let retry_deadline = if packet_not_before_qpc > now_qpc {
        packet_not_before_qpc
    } else {
        let interval = match qpc_clock.duration_from_us(UNSTABLE_TARGET_REPLAN_INTERVAL_US) {
            Ok(interval) => interval,
            Err(error) => {
                *terminal_error = Some(format!(
                    "target replan interval conversion failed: {error:?}"
                ));
                return WaitBoundary::Exit;
            }
        };
        match now_qpc.checked_add_duration(interval) {
            Ok(deadline) => deadline,
            Err(error) => {
                *terminal_error = Some(format!("target replan deadline overflow: {error}"));
                return WaitBoundary::Exit;
            }
        }
    };

    wait_for_next_boundary(WaitBoundaryInput {
        deadline: WaitDeadline {
            physical_target_qpc: Some(retry_deadline),
            spin_threshold_ticks: DurationTicks::ZERO,
            qpc_clock,
        },
        signals: WaitSignals { waiter, interrupt },
        mutable: WaitMutable {
            local_metrics,
            terminal_error,
        },
    })
}

fn evaluate_scheduler_action(
    executor: &sky_dispatch_core::executor::DispatchExecutor,
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
    epoch_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Result<SchedulerAction, String> {
    #[cfg(test)]
    SCHEDULER_EVALUATION_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    match executor
        .evaluate_at(now_qpc, epoch_qpc)
        .map_err(|error| format!("scheduler eligibility arithmetic failure: {error}"))?
    {
        sky_dispatch_core::executor::DispatchDecision::Finished => Ok(SchedulerAction::Finished),
        sky_dispatch_core::executor::DispatchDecision::Wait { deadline_qpc } => {
            Ok(SchedulerAction::Wait { deadline_qpc })
        }
        sky_dispatch_core::executor::DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => Ok(SchedulerAction::Send {
            up_mask,
            down_chord: down_chord.map(Into::into),
            timing_window,
        }),
    }
}

fn evaluate_scheduler_action_for_staging(
    executor: &sky_dispatch_core::executor::DispatchExecutor,
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
    epoch_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Result<
    (
        SchedulerAction,
        sky_dispatch_win32::clock::QpcTicks,
        Option<SchedulerAction>,
    ),
    String,
> {
    let action = evaluate_scheduler_action(executor, now_qpc, epoch_qpc)?;
    match action {
        send @ SchedulerAction::Send { .. } => Ok((action, now_qpc, Some(send))),
        SchedulerAction::Finished => Ok((action, now_qpc, None)),
        SchedulerAction::Wait { deadline_qpc } if deadline_qpc > now_qpc => {
            let at_deadline = evaluate_scheduler_action(executor, deadline_qpc, epoch_qpc)?;
            let action_to_stage = match at_deadline {
                send @ SchedulerAction::Send { .. } => Some(send),
                SchedulerAction::Wait { .. } | SchedulerAction::Finished => None,
            };
            Ok((action, deadline_qpc, action_to_stage))
        }
        SchedulerAction::Wait { .. } => Ok((action, now_qpc, None)),
    }
}

struct SchedulerStageAdmission<'a> {
    target: &'a SessionTarget,
    require_focus: bool,
    focus_active: &'a AtomicBool,
    capture_up_slot_timings: bool,
    control_signals: FinalControlSignals<'a>,
}

fn stage_up_only_after_mixed_replan(
    up_mask: u16,
    timing_window: sky_dispatch_core::executor::DispatchTimingWindow,
    admission: SchedulerStageAdmission<'_>,
    resources: &mut super::WorkerResources,
    runtime: &mut super::WorkerRuntime,
) -> Result<Option<StagedCandidate>, String> {
    if up_mask == 0
        || !focus_matches(admission.require_focus, admission.focus_active)
        || !matches!(
            final_control_precheck(admission.control_signals),
            FinalControlAdmission::Allowed
        )
    {
        return Ok(None);
    }

    let Some(mut candidate) = stage_candidate(
        up_mask,
        None,
        timing_window,
        admission.capture_up_slot_timings,
        admission.target,
        resources,
        runtime,
    )?
    else {
        return Ok(None);
    };
    candidate.timing_window = up_only_timing_window(
        candidate.timing_window,
        candidate.up_slot_timing_summary,
    )?;
    candidate.authored_target_qpc = candidate.timing_window.authored_target_qpc;
    candidate.authored_target_ticks = sky_dispatch_core::time::TimelineTicks::from_raw(
        candidate
            .authored_target_qpc
            .checked_duration_since(resources.playback.epoch)
            .map_err(|error| {
                format!("Up-only authored target precedes playback epoch: {error}")
            })?
            .as_u64(),
    );
    Ok(Some(candidate))
}

fn up_only_timing_window(
    mut timing_window: sky_dispatch_core::executor::DispatchTimingWindow,
    summary: Option<sky_dispatch_core::executor::UpSlotTimingSummary>,
) -> Result<sky_dispatch_core::executor::DispatchTimingWindow, String> {
    let summary = summary
        .filter(|summary| summary.slot_count != 0)
        .ok_or_else(|| "Up-only fallback has no active scheduler Up slots".to_string())?;
    timing_window.authored_target_qpc = summary.authored_target_qpc;
    timing_window.musical_up_not_before_qpc = summary.musical_up_not_before_qpc;
    timing_window.down_not_before_qpc = summary.authored_target_qpc;
    timing_window.packet_not_before_qpc = summary.musical_up_not_before_qpc;
    timing_window.hold_floor_mask = summary.hold_floor_mask;
    timing_window.release_floor_mask = 0;
    Ok(timing_window)
}

fn stage_scheduler_action_preserving_due_up(
    action: SchedulerAction,
    admission: SchedulerStageAdmission<'_>,
    resources: &mut super::WorkerResources,
    runtime: &mut super::WorkerRuntime,
) -> Result<Option<StagedCandidate>, String> {
    let SchedulerAction::Send {
        up_mask,
        down_chord,
        timing_window,
    } = action
    else {
        return Ok(None);
    };
    let has_down = down_chord.is_some();
    match stage_candidate(
        up_mask,
        down_chord,
        timing_window,
        admission.capture_up_slot_timings,
        admission.target,
        resources,
        runtime,
    )? {
        Some(candidate) => Ok(Some(candidate)),
        None if has_down && up_mask != 0 => stage_up_only_after_mixed_replan(
            up_mask,
            timing_window,
            admission,
            resources,
            runtime,
        ),
        None => Ok(None),
    }
}

fn stage_up_only_for_mixed_replan(
    candidate: &StagedCandidate,
    staging: SchedulerStageAdmission<'_>,
    resources: &mut super::WorkerResources,
    runtime: &mut super::WorkerRuntime,
) -> Result<Option<StagedCandidate>, String> {
    if candidate.target_stamp.is_none() {
        return Ok(None);
    }
    stage_up_only_after_mixed_replan(
        candidate.up_mask,
        candidate.timing_window,
        staging,
        resources,
        runtime,
    )
}

pub(crate) fn pre_wait_candidate_admission(
    candidate: &StagedCandidate,
    target: &SessionTarget,
    require_focus: bool,
    focus_active: &AtomicBool,
    control_signals: FinalControlSignals<'_>,
    runtime: &mut super::WorkerRuntime,
) -> StagedDispatchOutcome {
    if candidate.packet_masks.down_mask != 0 || candidate.packet_masks.up_mask != 0 {
        let expected = candidate
            .target_stamp
            .expect("target stamp checked in staging");
        if !target_stamp_still_current(target, expected)
            || !focus_matches(require_focus, focus_active)
        {
            runtime.verified_target = None;
            return StagedDispatchOutcome::Replan;
        }
    }

    match final_control_precheck(control_signals) {
        FinalControlAdmission::Allowed => StagedDispatchOutcome::Dispatched,
        FinalControlAdmission::PauseRequested => StagedDispatchOutcome::Replan,
        FinalControlAdmission::SystemSuspendRequested => {
            runtime.terminal_trigger = Some(super::control::TerminalTrigger::SystemSuspend);
            StagedDispatchOutcome::Exit
        }
        FinalControlAdmission::PanicRequested
        | FinalControlAdmission::QuitRequested
        | FinalControlAdmission::SkipRequested => StagedDispatchOutcome::Exit,
    }
}

fn qpc_evidence(value: Option<sky_dispatch_win32::clock::QpcTicks>) -> String {
    value.map_or_else(
        || "unavailable".to_string(),
        |ticks| ticks.as_u64().to_string(),
    )
}

#[cfg(test)]
mod scheduler_tests {
    use super::{
        SchedulerAction, evaluate_scheduler_action_for_staging, modifier_replan_deadline,
    };
    use crate::engine::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::TimelineTicks;
    use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
    use std::num::NonZeroU64;

    #[test]
    fn modifier_backoff_targets_independent_up_at_controlled_qpc_deadline() {
        use sky_dispatch_core::executor::DispatchDecision;

        let actions = [
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "modifier-deadline-a-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 19_500,
                scan_codes: vec![0x16].into(),
                reason: "modifier-deadline-b-down".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: vec![0x15].into(),
                reason: "modifier-deadline-a-up".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 40_000,
                scan_codes: vec![0x16].into(),
                reason: "modifier-deadline-b-up".into(),
            },
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let clock = harness.resources.clock;
        let epoch = QpcTicks::ZERO;
        harness
            .resources
            .scheduler
            .on_send_completed_at(0, 1, epoch)
            .expect("complete A Down at controlled QPC zero");

        let at_19_5ms = epoch
            .checked_add_duration(clock.duration_from_us(19_500).unwrap())
            .unwrap();
        let active_up_deadline = harness
            .resources
            .scheduler
            .earliest_active_up_due_qpc(epoch)
            .expect("active Up deadline arithmetic")
            .expect("active A slot");
        assert_eq!(
            active_up_deadline,
            epoch
                .checked_add_duration(clock.duration_from_us(20_000).unwrap())
                .unwrap()
        );
        assert!(matches!(
            harness
                .resources
                .scheduler
                .evaluate_at(at_19_5ms, epoch)
                .expect("19.5-ms scheduler decision"),
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(_),
                ..
            }
        ));

        let deadline = modifier_replan_deadline(at_19_5ms, Some(active_up_deadline), clock)
            .expect("modifier deadline calculation")
            .expect("future Up needs an interruptible wait");
        assert_eq!(deadline, active_up_deadline);

        let at_20ms = active_up_deadline;
        assert!(matches!(
            harness
                .resources
                .scheduler
                .evaluate_at(at_20ms, epoch)
                .expect("20-ms scheduler decision"),
            DispatchDecision::Send {
                up_mask: 1,
                down_chord: Some(_),
                ..
            }
        ));
        assert_eq!(
            modifier_replan_deadline(at_20ms, Some(active_up_deadline), clock)
                .expect("already-due Up calculation"),
            None,
            "an already-due Up replans immediately without arming a busy wait"
        );
    }

    #[test]
    fn modifier_backoff_uses_poll_deadline_without_active_up_and_keeps_earlier_up() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let now = QpcTicks::from_raw(19_500);
        let poll = QpcTicks::from_raw(20_500);
        let due = QpcTicks::from_raw(20_000);
        assert_eq!(
            modifier_replan_deadline(now, Some(QpcTicks::from_raw(21_000)), clock)
                .expect("later Up deadline"),
            Some(poll)
        );
        assert_eq!(
            modifier_replan_deadline(now, None, clock).expect("no active Up"),
            Some(poll)
        );
        assert_eq!(
            modifier_replan_deadline(now, Some(due), clock).expect("Up inside poll quantum"),
            Some(due)
        );
    }

    #[test]
    fn independent_up_deadline_keeps_hold_and_same_key_release_gap_floors() {
        use sky_dispatch_core::executor::DispatchDecision;

        let independent_actions = [
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "hold-floor-a-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 19_500,
                scan_codes: vec![0x16].into(),
                reason: "hold-floor-b-down".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: vec![0x15].into(),
                reason: "hold-floor-a-up".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 40_000,
                scan_codes: vec![0x16].into(),
                reason: "hold-floor-b-up".into(),
            },
        ];
        let hold = ProductionDispatchTestHarness::create_harness(&independent_actions);
        let clock = hold.resources.clock;
        let epoch = QpcTicks::ZERO;
        let down_completion = epoch
            .checked_add_duration(clock.duration_from_us(5_000).unwrap())
            .unwrap();
        let mut hold_scheduler = hold.resources.scheduler;
        hold_scheduler
            .on_send_completed_at(0, 1, down_completion)
            .expect("complete held A Down");
        let hold_floor_deadline = epoch
            .checked_add_duration(clock.duration_from_us(22_167).unwrap())
            .unwrap();
        assert_eq!(
            hold_scheduler
                .earliest_active_up_due_qpc(epoch)
                .expect("H-floor deadline arithmetic"),
            Some(hold_floor_deadline),
            "the independent Up waits for completion + H when that exceeds authored Up"
        );

        let repeat_actions = [
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "release-gap-first-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: vec![0x15].into(),
                reason: "release-gap-first-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 37_167,
                scan_codes: vec![0x15].into(),
                reason: "release-gap-repeat-down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 60_000,
                scan_codes: vec![0x15].into(),
                reason: "release-gap-repeat-up".into(),
            },
        ];
        let repeat = ProductionDispatchTestHarness::create_harness(&repeat_actions);
        let clock = repeat.resources.clock;
        let release_gap_us = repeat
            .dispatch_timing_config_for_test()
            .expect("release-gap timing configuration")
            .min_release_gap_us;
        let mut scheduler = repeat.resources.scheduler;
        scheduler
            .on_send_completed_at(0, 1, epoch)
            .expect("complete first Down");
        let authored_up = epoch
            .checked_add_duration(clock.duration_from_us(20_000).unwrap())
            .unwrap();
        assert!(matches!(
            scheduler
                .evaluate_at(authored_up, epoch)
                .expect("first Up becomes eligible"),
            DispatchDecision::Send { up_mask: 1, .. }
        ));
        let late_up_completion = epoch
            .checked_add_duration(clock.duration_from_us(25_000).unwrap())
            .unwrap();
        scheduler
            .on_send_completed_at(1, 0, late_up_completion)
            .expect("complete first Up late");
        let authored_repeat = epoch
            .checked_add_duration(clock.duration_from_us(37_167).unwrap())
            .unwrap();
        let release_gap_deadline = late_up_completion
            .checked_add_duration(
                clock
                    .duration_from_us(release_gap_us)
                    .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            scheduler
                .evaluate_at(authored_repeat, epoch)
                .expect("repeat waits for completion-based G"),
            DispatchDecision::Wait { deadline_qpc } if deadline_qpc == release_gap_deadline
        ));
    }

    #[test]
    fn scheduler_deadline_reevaluation_carries_down_metadata_from_evaluation() {
        let actions = [
            KeyActionInput {
                source_action_index: 17,
                kind: ActionKind::Down,
                scheduled_us: 5_000,
                scan_codes: vec![0x15].into(),
                reason: "deadline-down".into(),
            },
            KeyActionInput {
                source_action_index: 18,
                kind: ActionKind::Up,
                scheduled_us: 25_000,
                scan_codes: vec![0x15].into(),
                reason: "deadline-up".into(),
            },
        ];
        let harness = ProductionDispatchTestHarness::create_harness(&actions);
        let epoch = harness.resources.playback.epoch;
        let (initial, staging_target, staged) = evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            epoch,
            epoch,
        )
        .expect("scheduler staging decision");
        let SchedulerAction::Wait { deadline_qpc } = initial else {
            panic!("expected a wait before the authored Down deadline");
        };
        let expected = harness
            .current_executor_chord_for_test()
            .expect("authored Down metadata");

        assert_eq!(staging_target, deadline_qpc);
        let Some(SchedulerAction::Send { down_chord, .. }) = staged else {
            panic!("deadline reevaluation must stage its Down");
        };
        assert_eq!(down_chord, Some(expected));
    }

    #[test]
    fn staging_captures_up_slots_only_when_observer_is_enabled() {
        let actions = [
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "observer-off-down".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: vec![0x15].into(),
                reason: "observer-off-up".into(),
            },
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        harness.configure_mock_packet_sender_for_test(&[0]);
        harness.attach_executor_for_test().expect("attach executor");
        let down = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage Down")
            .expect("Down candidate");
        harness.set_mock_sender_start_qpc_for_test(
            harness.qpc_now_for_test().expect("Down sender QPC"),
        );
        assert!(matches!(
            harness.dispatch_staged_candidate_for_test(down, false),
            super::StagedDispatchOutcome::Dispatched
        ));

        let without_observer = harness
            .stage_candidate_with_observer_for_test(1, None, TimelineTicks::ZERO, false)
            .expect("stage Up without observer")
            .expect("Up candidate without observer");
        let with_observer = harness
            .stage_candidate_with_observer_for_test(1, None, TimelineTicks::ZERO, true)
            .expect("stage Up with observer")
            .expect("Up candidate with observer");

        assert!(without_observer.up_slot_timings.is_none());
        let summary = without_observer
            .up_slot_timing_summary
            .expect("scalar Up summary remains available");
        let captured = with_observer
            .up_slot_timings
            .as_ref()
            .expect("observer per-slot attribution");
        assert_eq!(captured.len(), usize::from(summary.slot_count));
        assert_eq!(captured[0].source_action_index, 4);
        assert_eq!(captured[0].source_action_index, summary.first_source_action_index);
        assert_eq!(without_observer.timing_window, with_observer.timing_window);
    }
}

fn sender_failure_record(
    candidate: &StagedCandidate,
    outcome: &sky_dispatch_win32::input::SendTransactionOutcome,
) -> String {
    let evidence = outcome.evidence;
    let source_chord = candidate
        .down_chord
        .as_ref()
        .map_or_else(|| "none".to_string(), |chord| chord.chord_index.to_string());
    format!(
        "source_action={} source_chord={} up_mask=0x{:04x} down_mask=0x{:04x} requested={} confirmed_count={} first_inserted={} attempts={} status={:?} retry_reason={:?} win32_error={:?} pre_call_qpc={} completion_qpc={} timing_error={:?}",
        candidate.source_action_index,
        source_chord,
        candidate.packet_masks.up_mask,
        candidate.packet_masks.down_mask,
        candidate.packet_masks.event_count(),
        evidence.confirmed_mask.count_ones(),
        evidence.first_inserted,
        evidence.attempts,
        outcome.status,
        evidence.retry_reason,
        evidence.last_win32_error.or(evidence.first_win32_error),
        qpc_evidence(evidence.started_ticks),
        qpc_evidence(evidence.completed_ticks),
        evidence.timing_error,
    )
}

fn gate_rejection_record(candidate: &StagedCandidate, reason: &str) -> String {
    let source_chord = candidate
        .down_chord
        .as_ref()
        .map_or_else(|| "none".to_string(), |chord| chord.chord_index.to_string());
    format!(
        "dispatch gate rejected before SendInput: source_action={} source_chord={} up_mask=0x{:04x} down_mask=0x{:04x} requested={} confirmed_count=unavailable first_inserted=unavailable attempts=0 status=gate_rejected win32_error=unavailable pre_call_qpc=unavailable completion_qpc=unavailable: {reason}",
        candidate.source_action_index,
        source_chord,
        candidate.packet_masks.up_mask,
        candidate.packet_masks.down_mask,
        candidate.packet_masks.event_count(),
    )
}
