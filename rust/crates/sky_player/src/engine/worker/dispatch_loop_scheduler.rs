#[derive(Debug)]
enum SchedulerAction {
    Finished,
    Wait {
        deadline_qpc: sky_dispatch_win32::clock::QpcTicks,
    },
    Send {
        up_mask: u16,
        down_chord_index: Option<usize>,
        timing_window: sky_dispatch_core::executor::DispatchTimingWindow,
    },
}

const UNSTABLE_TARGET_REPLAN_INTERVAL_US: u64 = 1_000;

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
            down_chord_index: down_chord.map(|chord| chord.chord_index),
            timing_window,
        }),
    }
}

fn resolve_scheduler_down_chord(
    executor: &sky_dispatch_core::executor::DispatchExecutor,
    expected_index: Option<usize>,
) -> Result<Option<sky_dispatch_core::executor::DownChordTicks>, String> {
    let Some(expected_index) = expected_index else {
        return Ok(None);
    };
    let chord = executor
        .current_chord()
        .ok_or_else(|| "scheduler selected a Down chord that is no longer current".to_string())?;
    if chord.chord_index != expected_index {
        return Err(format!(
            "scheduler Down chord changed after selection: expected {expected_index}, found {}",
            chord.chord_index
        ));
    }
    Ok(Some(chord.clone()))
}

fn stage_scheduler_action(
    action: SchedulerAction,
    target: &SessionTarget,
    resources: &mut super::WorkerResources,
    runtime: &mut super::WorkerRuntime,
) -> Result<Option<StagedCandidate>, String> {
    let SchedulerAction::Send {
        up_mask,
        down_chord_index,
        timing_window,
    } = action
    else {
        return Ok(None);
    };
    let down_chord = resolve_scheduler_down_chord(&resources.scheduler, down_chord_index)?;
    stage_candidate(up_mask, down_chord, timing_window, target, resources, runtime)
}

fn pre_wait_candidate_admission(
    candidate: &StagedCandidate,
    target: &SessionTarget,
    require_focus: bool,
    focus_active: &AtomicBool,
    control_signals: FinalControlSignals<'_>,
    runtime: &mut super::WorkerRuntime,
) -> StagedDispatchOutcome {
    if candidate.packet_masks.down_mask != 0 {
        let expected = candidate
            .preflight_target
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
