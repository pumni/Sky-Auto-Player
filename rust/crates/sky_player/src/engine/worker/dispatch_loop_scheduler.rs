enum SchedulerAction {
    Finished,
    Wait {
        deadline_ticks: TimelineTicks,
    },
    Send {
        up_mask: u16,
        down_chord_index: Option<usize>,
    },
}

fn evaluate_scheduler_action(
    executor: &sky_dispatch_core::executor::DispatchExecutor,
    now: TimelineTicks,
) -> SchedulerAction {
    match executor.evaluate(now) {
        sky_dispatch_core::executor::DispatchAction::Finished => SchedulerAction::Finished,
        sky_dispatch_core::executor::DispatchAction::Wait { deadline_ticks } => {
            SchedulerAction::Wait { deadline_ticks }
        }
        sky_dispatch_core::executor::DispatchAction::Send {
            up_mask,
            down_chord,
        } => SchedulerAction::Send {
            up_mask,
            down_chord_index: down_chord.map(|chord| chord.chord_index),
        },
    }
}

fn resolve_scheduler_down_chord(
    coordinator: &sky_dispatch_core::coordinator::RuntimeDispatchCoordinator,
    expected_index: Option<usize>,
) -> Result<Option<sky_dispatch_core::executor::DownChordTicks>, String> {
    let Some(expected_index) = expected_index else {
        return Ok(None);
    };
    let chord = coordinator
        .executor
        .as_ref()
        .and_then(|executor| executor.current_chord())
        .ok_or_else(|| "scheduler selected a Down chord that is no longer current".to_string())?;
    if chord.chord_index != expected_index {
        return Err(format!(
            "scheduler Down chord changed after selection: expected {expected_index}, found {}",
            chord.chord_index
        ));
    }
    Ok(Some(chord.clone()))
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
            runtime.invalidate_down_authorization();
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
        "source_action={} source_chord={} up_mask=0x{:04x} down_mask=0x{:04x} requested={} inserted={} first_inserted={} attempts={} status={:?} retry_reason={:?} win32_error={:?} pre_call_qpc={} completion_qpc={} timing_error={:?}",
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
        "dispatch gate rejected before SendInput: source_action={} source_chord={} up_mask=0x{:04x} down_mask=0x{:04x} requested={} inserted=unavailable attempts=0 status=gate_rejected win32_error=unavailable pre_call_qpc=unavailable completion_qpc=unavailable: {reason}",
        candidate.source_action_index,
        source_chord,
        candidate.packet_masks.up_mask,
        candidate.packet_masks.down_mask,
        candidate.packet_masks.event_count(),
    )
}
