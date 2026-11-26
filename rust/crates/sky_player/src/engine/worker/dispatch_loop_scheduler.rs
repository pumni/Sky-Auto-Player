#[derive(Clone, Copy, Debug)]
enum SchedulerAction {
    Finished,
    Wait {
        deadline_qpc: sky_dispatch_win32::clock::QpcTicks,
    },
    Send {
        up_mask: u16,
        down_chord: Option<sky_dispatch_core::executor::PreparedChord>,
        timing_window: sky_dispatch_core::executor::DispatchTimingWindow,
    },
}

const UNSTABLE_TARGET_REPLAN_INTERVAL_US: u64 = 1_000;
const MODIFIER_REPLAN_INTERVAL_US: u64 = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModifierWaitPurpose {
    Poll,
    ActiveUpDeadline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModifierWaitResult {
    AlreadyDue,
    Waited {
        purpose: ModifierWaitPurpose,
        deadline_qpc: sky_dispatch_win32::clock::QpcTicks,
        planned_wait_ticks: DurationTicks,
        result: sky_dispatch_win32::wait::WaitResult,
    },
}

/// Sleep for one modifier re-evaluation quantum without changing musical
/// deadlines or using the precision spin phase. The shared interrupt event
/// wakes this wait for controls, lifecycle changes, or owner invalidation.
fn wait_after_modifier_held(
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
    next_active_up_deadline: Option<sky_dispatch_win32::clock::QpcTicks>,
    max_hold_deadline: sky_dispatch_win32::clock::QpcTicks,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
    wait: impl FnOnce(
        sky_dispatch_win32::clock::QpcClock,
        sky_dispatch_win32::clock::QpcTicks,
        DurationTicks,
    ) -> sky_dispatch_win32::wait::WaitResult,
) -> Result<ModifierWaitResult, String> {
    let Some(deadline) = modifier_replan_deadline(
        now_qpc,
        next_active_up_deadline,
        max_hold_deadline,
        qpc_clock,
    )? else {
        // Re-enter the normal scheduler immediately for an already-due Up.
        // It will either dispatch that Up or take its usual final gate; no
        // zero-length waiter loop is armed here.
        return Ok(ModifierWaitResult::AlreadyDue);
    };
    let planned_wait_ticks = deadline
        .checked_duration_since(now_qpc)
        .map_err(|error| format!("modifier replan planned-wait arithmetic failed: {error}"))?;
    let purpose = if next_active_up_deadline == Some(deadline) {
        ModifierWaitPurpose::ActiveUpDeadline
    } else {
        ModifierWaitPurpose::Poll
    };
    let result = wait(qpc_clock, deadline, DurationTicks::ZERO);
    Ok(ModifierWaitResult::Waited {
        purpose,
        deadline_qpc: deadline,
        planned_wait_ticks,
        result,
    })
}

fn account_modifier_wait_result(
    wait: ModifierWaitResult,
    runtime: &mut super::WorkerRuntime,
    observer: Option<&super::dispatch::PendingObservationQueue>,
    local_metrics: &mut super::WorkerMetricsLocal,
    epoch_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Result<Option<(
    sky_dispatch_win32::clock::QpcTicks,
    sky_dispatch_win32::clock::QpcTicks,
)>, sky_dispatch_win32::wait::WaitFailure> {
    let ModifierWaitResult::Waited {
        purpose,
        deadline_qpc,
        planned_wait_ticks,
        result,
    } = wait
    else {
        return Ok(None);
    };
    let active_up_wait = purpose == ModifierWaitPurpose::ActiveUpDeadline;
    let deadline_ticks = sky_dispatch_core::time::TimelineTicks::from_raw(
        deadline_qpc
            .as_u64()
            .saturating_sub(epoch_qpc.as_u64()),
    );
    match result.outcome {
        sky_dispatch_win32::wait::WaitOutcome::Deadline => {
            if !active_up_wait {
                // Polls are maintenance waits; keep them out of the musical
                // observer stream while still inspecting their typed outcome.
                return Ok(None);
            }
            super::dispatch::publish_pending_wait_observation(runtime, observer, local_metrics);
            runtime.pending_wait_observation = super::dispatch::record_wait_attempt(
                observer,
                Some(result),
                deadline_qpc,
                planned_wait_ticks,
                deadline_ticks,
                epoch_qpc,
                None,
                local_metrics,
            );
            // The wait is complete evidence in its own right. Publishing here
            // prevents an unrelated later packet from inheriting its timer data.
            super::dispatch::publish_pending_wait_observation(runtime, observer, local_metrics);
            Ok(result
                .wake_qpc
                .map(|wake_qpc| (deadline_qpc, wake_qpc)))
        }
        sky_dispatch_win32::wait::WaitOutcome::Interrupted => {
            local_metrics.wait_interrupted_count =
                local_metrics.wait_interrupted_count.saturating_add(1);
            if active_up_wait {
                super::dispatch::publish_interrupted_physical_wait(
                    runtime,
                    observer,
                    local_metrics,
                    result,
                    deadline_qpc,
                    planned_wait_ticks,
                    deadline_ticks,
                    epoch_qpc,
                );
            }
            Ok(None)
        }
        sky_dispatch_win32::wait::WaitOutcome::Failed(failure) => Err(failure),
    }
}

fn attribute_modifier_up_deadline_wake(
    pending_wake: Option<(
        sky_dispatch_win32::clock::QpcTicks,
        sky_dispatch_win32::clock::QpcTicks,
    )>,
    candidate: Option<&StagedCandidate>,
    runtime: &mut super::WorkerRuntime,
) {
    let Some((up_deadline_qpc, wake_qpc)) = pending_wake else {
        return;
    };
    if candidate.is_some_and(|candidate| {
        candidate.packet_masks.up_mask != 0
            && candidate.timing_window.musical_up_not_before_qpc == up_deadline_qpc
    }) {
        runtime.last_dispatch_deadline_wake_qpc = Some(wake_qpc);
        runtime.last_dispatch_deadline_target_qpc = Some(up_deadline_qpc);
    }
}

fn modifier_replan_deadline(
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
    next_active_up_deadline: Option<sky_dispatch_win32::clock::QpcTicks>,
    max_hold_deadline: sky_dispatch_win32::clock::QpcTicks,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
) -> Result<Option<sky_dispatch_win32::clock::QpcTicks>, String> {
    if max_hold_deadline <= now_qpc {
        return Ok(None);
    }
    let interval = qpc_clock
        .duration_from_us(MODIFIER_REPLAN_INTERVAL_US)
        .map_err(|error| format!("modifier replan interval conversion failed: {error:?}"))?;
    let mut poll_deadline = now_qpc
        .checked_add_duration(interval)
        .map_err(|error| format!("modifier replan deadline overflow: {error}"))?;
    poll_deadline = poll_deadline.min(max_hold_deadline);
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
            down_chord,
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
    capture_up_slot_timings: bool,
}

fn up_only_timing_window(
    mut timing_window: sky_dispatch_core::executor::DispatchTimingWindow,
    summary: Option<sky_dispatch_core::executor::UpSlotTimingSummary>,
) -> Result<sky_dispatch_core::executor::DispatchTimingWindow, String> {
    let summary = summary
        .filter(|summary| summary.slot_count != 0)
        .ok_or_else(|| "Up-only selection has no active scheduler Up slots".to_string())?;
    timing_window.authored_target_qpc = summary.authored_target_qpc;
    timing_window.musical_up_not_before_qpc = summary.musical_up_not_before_qpc;
    timing_window.down_not_before_qpc = summary.authored_target_qpc;
    timing_window.packet_not_before_qpc = summary.musical_up_not_before_qpc;
    timing_window.hold_floor_mask = summary.hold_floor_mask;
    Ok(timing_window)
}

fn stage_scheduler_send_candidate(
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
    stage_candidate(
        up_mask,
        down_chord,
        timing_window,
        admission.capture_up_slot_timings,
        admission.target,
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
        ModifierWaitPurpose, ModifierWaitResult, SchedulerAction,
        account_modifier_wait_result, attribute_modifier_up_deadline_wake,
        evaluate_scheduler_action_for_staging, modifier_replan_deadline, wait_after_modifier_held,
    };
    use crate::engine::telemetry::WorkerMetricsLocal;
    use crate::engine::test_support::ProductionDispatchTestHarness;
    use crate::engine::worker::WorkerRuntime;
    use crate::engine::worker::dispatch::observation::DispatchObservation;
    use crate::engine::worker::dispatch::observer::PendingObservationQueue;
    use sky_dispatch_core::model::OnsetChordInput;
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
    use sky_dispatch_win32::wait::{WaitFailure, WaitOutcome, WaitResult};
    use std::num::NonZeroU64;

    fn controlled_wait_result(
        outcome: WaitOutcome,
        wake_qpc: Option<QpcTicks>,
    ) -> WaitResult {
        WaitResult {
            outcome,
            wake_qpc,
            spin_ticks: DurationTicks::from_raw(7),
            timer_return_qpc: Some(QpcTicks::from_raw(20_007)),
            timer_arm_target_qpc: Some(QpcTicks::from_raw(20_000)),
            timer_arm_qpc: Some(QpcTicks::from_raw(19_800)),
            timer_arm_generation: Some(4),
            timer_return_generation: Some(4),
        }
    }

    #[test]
    fn modifier_up_deadline_wait_preserves_typed_wait_evidence() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let now = QpcTicks::from_raw(19_500);
        let up_deadline = QpcTicks::from_raw(20_000);
        let expected = controlled_wait_result(
            WaitOutcome::Deadline,
            Some(QpcTicks::from_raw(20_010)),
        );
        let waited = wait_after_modifier_held(
            now,
            Some(up_deadline),
            QpcTicks::from_raw(100_000),
            clock,
            |actual_clock, deadline, spin| {
                assert_eq!(actual_clock, clock);
                assert_eq!(deadline, up_deadline);
                assert_eq!(spin, DurationTicks::ZERO);
                expected
            },
        )
        .expect("typed Up deadline wait");
        assert_eq!(
            waited,
            ModifierWaitResult::Waited {
                purpose: ModifierWaitPurpose::ActiveUpDeadline,
                deadline_qpc: up_deadline,
                planned_wait_ticks: DurationTicks::from_raw(500),
                result: expected,
            }
        );

        let queue = PendingObservationQueue::default();
        let mut runtime = WorkerRuntime::default();
        let mut metrics = WorkerMetricsLocal::default();
        assert_eq!(
            account_modifier_wait_result(
                waited,
                &mut runtime,
                Some(&queue),
                &mut metrics,
                QpcTicks::ZERO,
            )
            .expect("account Up wait"),
            Some((up_deadline, QpcTicks::from_raw(20_010)))
        );
        assert_eq!(metrics.spin_observation_expected_samples, 1);
        assert_eq!(metrics.spin_observation_missing_samples, 0);
        match queue.pop_front().expect("authored Up wait observation") {
            DispatchObservation::Wait(wait) => {
                assert_eq!(wait.outcome, WaitOutcome::Deadline);
                assert_eq!(wait.wake_qpc, Some(QpcTicks::from_raw(20_010)));
                assert_eq!(wait.physical_target_qpc, up_deadline);
                assert_eq!(wait.planned_wait_ticks, DurationTicks::from_raw(500));
                assert_eq!(wait.timer_return_qpc, Some(QpcTicks::from_raw(20_007)));
                assert_eq!(wait.timer_arm_target_qpc, Some(up_deadline));
                assert_eq!(wait.timer_arm_qpc, Some(QpcTicks::from_raw(19_800)));
                assert_eq!(wait.timer_arm_generation, Some(4));
                assert_eq!(wait.timer_return_generation, Some(4));
            }
            other => panic!("expected wait evidence, got {other:?}"),
        }
    }

    #[test]
    fn modifier_poll_waits_stay_out_of_musical_observer_but_count_interrupts_and_failures() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let now = QpcTicks::from_raw(19_500);
        let poll_deadline = QpcTicks::from_raw(20_500);
        let poll = wait_after_modifier_held(now, None, QpcTicks::from_raw(100_000), clock, |_, deadline, spin| {
            assert_eq!(deadline, poll_deadline);
            assert_eq!(spin, DurationTicks::ZERO);
            controlled_wait_result(WaitOutcome::Interrupted, None)
        })
        .expect("poll wait");
        assert!(matches!(
            poll,
            ModifierWaitResult::Waited {
                purpose: ModifierWaitPurpose::Poll,
                deadline_qpc,
                ..
            } if deadline_qpc == poll_deadline
        ));
        let queue = PendingObservationQueue::default();
        let mut runtime = WorkerRuntime::default();
        let mut metrics = WorkerMetricsLocal::default();
        assert_eq!(
            account_modifier_wait_result(
                poll,
                &mut runtime,
                Some(&queue),
                &mut metrics,
                QpcTicks::ZERO,
            ),
            Ok(None)
        );
        assert_eq!(metrics.wait_interrupted_count, 1);
        assert_eq!(metrics.spin_observation_expected_samples, 0);
        assert_eq!(queue.len(), 0, "maintenance poll must not enter wait observer");

        let failure = wait_after_modifier_held(now, None, QpcTicks::from_raw(100_000), clock, |_, deadline, _| {
            assert_eq!(deadline, poll_deadline);
            controlled_wait_result(
                WaitOutcome::Failed(WaitFailure::TimerWait { win32_error: 5 }),
                None,
            )
        })
        .expect("typed poll failure");
        assert_eq!(
            account_modifier_wait_result(
                failure,
                &mut runtime,
                Some(&queue),
                &mut metrics,
                QpcTicks::ZERO,
            ),
            Err(WaitFailure::TimerWait { win32_error: 5 })
        );
    }

    #[test]
    fn already_due_modifier_up_does_not_arm_wait_or_poll_observer() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let now = QpcTicks::from_raw(20_000);
        let waited = wait_after_modifier_held(now, Some(now), QpcTicks::from_raw(100_000), clock, |_, _, _| {
            panic!("already-due Up must not arm a zero-length wait")
        })
        .expect("already-due Up");
        assert_eq!(waited, ModifierWaitResult::AlreadyDue);
        let mut runtime = WorkerRuntime::default();
        let mut metrics = WorkerMetricsLocal::default();
        assert_eq!(
            account_modifier_wait_result(waited, &mut runtime, None, &mut metrics, now),
            Ok(None)
        );
        assert_eq!(metrics.spin_observation_expected_samples, 0);
    }

    #[test]
    fn modifier_up_wait_without_observer_counts_missing_evidence_and_interrupted_up_is_typed() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let now = QpcTicks::from_raw(19_500);
        let deadline = QpcTicks::from_raw(20_000);
        let waited = wait_after_modifier_held(now, Some(deadline), QpcTicks::from_raw(100_000), clock, |_, _, _| {
            controlled_wait_result(WaitOutcome::Deadline, Some(deadline))
        })
        .expect("observer-off Up wait");
        let mut runtime = WorkerRuntime::default();
        let mut metrics = WorkerMetricsLocal::default();
        assert_eq!(
            account_modifier_wait_result(waited, &mut runtime, None, &mut metrics, QpcTicks::ZERO),
            Ok(Some((deadline, deadline)))
        );
        assert_eq!(metrics.spin_observation_expected_samples, 1);
        assert_eq!(metrics.spin_observation_missing_samples, 1);

        let interrupted = wait_after_modifier_held(now, Some(deadline), QpcTicks::from_raw(100_000), clock, |_, _, _| {
            controlled_wait_result(WaitOutcome::Interrupted, None)
        })
        .expect("interrupted Up wait");
        assert_eq!(
            account_modifier_wait_result(
                interrupted,
                &mut runtime,
                None,
                &mut metrics,
                QpcTicks::ZERO,
            ),
            Ok(None)
        );
        assert_eq!(metrics.wait_interrupted_count, 1);
        assert_eq!(metrics.spin_observation_expected_samples, 2);
        assert_eq!(metrics.spin_observation_missing_samples, 2);
    }

    #[test]
    fn modifier_up_wake_is_attached_only_to_the_matching_up_deadline_packet() {
        let chords = [OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: vec![0x15].into(),
        }];
        let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
        let epoch = harness.resources.playback.epoch;
        harness
            .resources
            .scheduler
            .on_send_completed_at(0, 1, epoch)
            .expect("commit first Down");
        let deadline = epoch
            .checked_add_duration(harness.resources.clock.duration_from_us(17_167).unwrap())
            .unwrap();
        let candidate = harness
            .stage_candidate_for_test(
                1,
                None,
                TimelineTicks::from_raw(17_167),
            )
            .expect("stage release at its deadline")
            .expect("Up candidate");
        assert_eq!(candidate.timing_window.musical_up_not_before_qpc, deadline);

        let wake = epoch
            .checked_add_duration(harness.resources.clock.duration_from_us(17_177).unwrap())
            .unwrap();
        let mut runtime = WorkerRuntime::default();
        attribute_modifier_up_deadline_wake(Some((deadline, wake)), Some(&candidate), &mut runtime);
        assert_eq!(runtime.last_dispatch_deadline_target_qpc, Some(deadline));
        assert_eq!(runtime.last_dispatch_deadline_wake_qpc, Some(wake));

        let mut unrelated_runtime = WorkerRuntime::default();
        let mismatched_deadline = deadline
            .checked_add_duration(DurationTicks::from_raw(1))
            .unwrap();
        attribute_modifier_up_deadline_wake(
            Some((mismatched_deadline, wake)),
            Some(&candidate),
            &mut unrelated_runtime,
        );
        assert_eq!(unrelated_runtime.last_dispatch_deadline_target_qpc, None);
        assert_eq!(unrelated_runtime.last_dispatch_deadline_wake_qpc, None);
    }

    #[test]
    fn modifier_backoff_targets_independent_up_at_controlled_qpc_deadline() {
        use sky_dispatch_core::executor::DispatchDecision;

        let chords = [
            OnsetChordInput {
                source_action_index: 0,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
            },
            OnsetChordInput {
                source_action_index: 1,
                scheduled_us: 16_500,
                scan_codes: vec![0x16].into(),
            },
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
        let clock = harness.resources.clock;
        let epoch = QpcTicks::ZERO;
        harness
            .resources
            .scheduler
            .on_send_completed_at(0, 1, epoch)
            .expect("complete A Down at controlled QPC zero");

        let at_16_5ms = epoch
            .checked_add_duration(clock.duration_from_us(16_500).unwrap())
            .unwrap();
        let active_up_deadline = harness
            .resources
            .scheduler
            .earliest_active_up_due_qpc()
            .expect("active A slot");
        assert_eq!(
            active_up_deadline,
            epoch
                .checked_add_duration(clock.duration_from_us(17_167).unwrap())
                .unwrap()
        );
        assert!(matches!(
            harness
                .resources
                .scheduler
                .evaluate_at(at_16_5ms, epoch)
                .expect("16.5-ms scheduler decision"),
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(_),
                ..
            }
        ));

        let default_max_hold = at_16_5ms
            .checked_add_duration(clock.duration_from_us(2_000_000).unwrap())
            .unwrap();
        let deadline = modifier_replan_deadline(at_16_5ms, Some(active_up_deadline), default_max_hold, clock)
            .expect("modifier deadline calculation")
            .expect("future Up needs an interruptible wait");
        assert_eq!(deadline, active_up_deadline);

        let at_up = active_up_deadline;
        assert!(matches!(
            harness
                .resources
                .scheduler
                .evaluate_at(at_up, epoch)
                .expect("Up scheduler decision"),
            DispatchDecision::Send {
                up_mask: 1,
                down_chord: Some(_),
                ..
            }
        ));
        assert_eq!(
            modifier_replan_deadline(at_up, Some(active_up_deadline), default_max_hold, clock)
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
        let default_max = QpcTicks::from_raw(100_000);
        assert_eq!(
            modifier_replan_deadline(now, Some(QpcTicks::from_raw(21_000)), default_max, clock)
                .expect("later Up deadline"),
            Some(poll)
        );
        assert_eq!(
            modifier_replan_deadline(now, None, default_max, clock).expect("no active Up"),
            Some(poll)
        );
        assert_eq!(
            modifier_replan_deadline(now, Some(due), default_max, clock).expect("Up inside poll quantum"),
            Some(due)
        );

        // Bounded deadline capping:
        let max_early = QpcTicks::from_raw(20_200);
        assert_eq!(
            modifier_replan_deadline(now, None, max_early, clock).expect("capped deadline"),
            Some(max_early)
        );
        let max_expired = QpcTicks::from_raw(19_500);
        assert_eq!(
            modifier_replan_deadline(now, None, max_expired, clock).expect("expired deadline"),
            None
        );
    }

    #[test]
    fn independent_up_deadline_keeps_hold_without_adding_an_extra_same_key_wait() {
        use sky_dispatch_core::executor::DispatchDecision;

        let independent_chords = [
            OnsetChordInput {
                source_action_index: 0,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
            },
            OnsetChordInput {
                source_action_index: 1,
                scheduled_us: 19_500,
                scan_codes: vec![0x16].into(),
            },
        ];
        let hold = ProductionDispatchTestHarness::create_harness(&independent_chords);
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
            hold_scheduler.earliest_active_up_due_qpc(),
            Some(hold_floor_deadline),
            "the independent Up waits for completion + H"
        );

        let repeat_chords = [
            OnsetChordInput {
                source_action_index: 0,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
            },
            OnsetChordInput {
                source_action_index: 1,
                scheduled_us: 37_167,
                scan_codes: vec![0x15].into(),
            },
        ];
        let repeat = ProductionDispatchTestHarness::create_harness(&repeat_chords);
        let mut scheduler = repeat.resources.scheduler;
        scheduler
            .on_send_completed_at(0, 1, epoch)
            .expect("complete first Down");
        let authored_up = epoch
            .checked_add_duration(clock.duration_from_us(17_167).unwrap())
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
        assert!(matches!(
            scheduler
                .evaluate_at(authored_repeat, epoch)
                .expect("authored repeat is due after Up commit"),
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(_),
                timing_window: _,
            }
        ));
    }

    #[test]
    fn scheduler_deadline_reevaluation_carries_down_metadata_from_evaluation() {
        let chords = [OnsetChordInput {
            source_action_index: 17,
            scheduled_us: 5_000,
            scan_codes: vec![0x15].into(),
        }];
        let harness = ProductionDispatchTestHarness::create_harness(&chords);
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
        let chords = [OnsetChordInput {
            source_action_index: 3,
            scheduled_us: 0,
            scan_codes: vec![0x15].into(),
        }];
        let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
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
        assert_eq!(captured[0].source_action_index, 3);
        assert_eq!(captured[0].source_action_index, summary.first_source_action_index);
        assert_eq!(without_observer.timing_window, with_observer.timing_window);
    }
}

fn sender_failure_record(
    candidate: &EffectivePacket<'_>,
    outcome: &sky_dispatch_win32::input::SendTransactionOutcome,
) -> String {
    let evidence = outcome.evidence;
    let source_chord = candidate
        .down_chord
        .as_ref()
        .map_or_else(|| "none".to_string(), |chord| chord.chord_index.to_string());
    let packet = candidate.packet();
    format!(
        "source_action={} source_chord={} up_mask=0x{:04x} down_mask=0x{:04x} requested={} confirmed_count={} first_inserted={} attempts={} status={:?} retry_reason={:?} win32_error={:?} pre_call_qpc={} completion_qpc={} timing_error={:?}",
        candidate.source_action_index,
        source_chord,
        packet.up_mask,
        packet.down_mask,
        packet.event_count(),
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

fn gate_rejection_record(candidate: &EffectivePacket<'_>, reason: &str) -> String {
    let source_chord = candidate
        .down_chord
        .as_ref()
        .map_or_else(|| "none".to_string(), |chord| chord.chord_index.to_string());
    let packet = candidate.packet();
    format!(
        "dispatch gate rejected before SendInput: source_action={} source_chord={} up_mask=0x{:04x} down_mask=0x{:04x} requested={} confirmed_count=unavailable first_inserted=unavailable attempts=0 status=gate_rejected win32_error=unavailable pre_call_qpc=unavailable completion_qpc=unavailable: {reason}",
        candidate.source_action_index,
        source_chord,
        packet.up_mask,
        packet.down_mask,
        packet.event_count(),
    )
}
