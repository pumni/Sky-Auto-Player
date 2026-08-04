use super::*;

pub(super) enum WaitBoundary {
    Ready,
    Continue,
    Exit,
}

pub(super) struct WaitBoundaryContext<'a> {
    pub(super) deadline_ticks: Option<TimelineTicks>,
    pub(super) qpc_clock: QpcClock,
    pub(super) clock_state: &'a mut PlaybackClockState,
    pub(super) allow_pre_epoch_startup_dispatch: bool,
    pub(super) last_send_qpc_ticks: Option<QpcTicks>,
    pub(super) core_warmup_ticks: DurationTicks,
    pub(super) cold_threshold_ticks: DurationTicks,
    pub(super) effective_spin_threshold_ticks: DurationTicks,
    pub(super) waiter: &'a HybridWaiter,
    pub(super) lease_timeout_ticks: DurationTicks,
    pub(super) supervisor_heartbeat_ticks: &'a AtomicU64,
    pub(super) interrupt: &'a OwnedEvent,
    pub(super) strict_timing: bool,
    pub(super) input_path_warn_us: u64,
    pub(super) local_metrics: &'a mut WorkerMetricsLocal,
    pub(super) pending_pre_send_spin_us: &'a mut u64,
    pub(super) force_full_cleanup: &'a mut bool,
    pub(super) terminal_error: &'a mut Option<String>,
}

pub(super) fn wait_for_next_boundary(context: WaitBoundaryContext<'_>) -> WaitBoundary {
    let WaitBoundaryContext {
        deadline_ticks,
        qpc_clock,
        clock_state,
        allow_pre_epoch_startup_dispatch,
        last_send_qpc_ticks,
        core_warmup_ticks,
        cold_threshold_ticks,
        effective_spin_threshold_ticks,
        waiter,
        lease_timeout_ticks,
        supervisor_heartbeat_ticks,
        interrupt,
        strict_timing,
        input_path_warn_us,
        local_metrics,
        pending_pre_send_spin_us,
        force_full_cleanup,
        terminal_error,
    } = context;

    let Some(deadline_ticks) = deadline_ticks else {
        return WaitBoundary::Exit;
    };

    // Take the QPC tick and its logical elapsed-time sample from the same
    // instant. Reusing an older sample shifts the absolute target late by the
    // whole bookkeeping interval.
    let target_sample_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("QPC failure before dispatch wait: {error:?}"));
            return WaitBoundary::Exit;
        }
    };
    let target_sample_elapsed_ticks = match clock_state
        .get_elapsed_allow_pre_epoch(target_sample_ticks, allow_pre_epoch_startup_dispatch)
    {
        Ok(ticks) => ticks,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("playback clock failure: {error}"));
            return WaitBoundary::Exit;
        }
    };
    if deadline_ticks <= target_sample_elapsed_ticks {
        return WaitBoundary::Ready;
    }

    let target_qpc = match clock_state
        .epoch
        .checked_add_duration(DurationTicks::from_raw(deadline_ticks.as_u64()))
    {
        Ok(target) => target,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("deadline arithmetic failure: {error}"));
            return WaitBoundary::Exit;
        }
    };
    let cold_warmup_ticks = match last_send_qpc_ticks {
        None => core_warmup_ticks,
        Some(last_send_ticks) => {
            let gap = match target_sample_ticks.checked_duration_since(last_send_ticks) {
                Ok(gap) => gap,
                Err(error) => {
                    *force_full_cleanup = true;
                    *terminal_error = Some(format!("cold classification clock failure: {error}"));
                    return WaitBoundary::Exit;
                }
            };
            if gap > cold_threshold_ticks {
                core_warmup_ticks
            } else {
                DurationTicks::ZERO
            }
        }
    };
    let wait_spin_threshold_ticks =
        match effective_spin_threshold_ticks.checked_add(cold_warmup_ticks) {
            Ok(threshold) => threshold,
            Err(error) => {
                *force_full_cleanup = true;
                *terminal_error = Some(format!("spin threshold arithmetic failure: {error}"));
                return WaitBoundary::Exit;
            }
        };
    let bounded_target =
        match lease_bounded_ticks(target_qpc, lease_timeout_ticks, supervisor_heartbeat_ticks) {
            Ok(target) => target,
            Err(error) => {
                *force_full_cleanup = true;
                *terminal_error = Some(format!("lease deadline failure: {error:?}"));
                return WaitBoundary::Exit;
            }
        };
    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
        qpc_clock,
        bounded_target,
        wait_spin_threshold_ticks,
        interrupt,
    );
    local_metrics.idle_wake_count = local_metrics.idle_wake_count.saturating_add(1);
    local_metrics.spin_time_us = local_metrics
        .spin_time_us
        .saturating_add(wait_result.spin_us);
    *pending_pre_send_spin_us = wait_result.spin_us;

    let wake_qpc_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("QPC runtime failure: {error:?}"));
            return WaitBoundary::Exit;
        }
    };
    let wake_elapsed_ticks = match clock_state
        .get_elapsed_allow_pre_epoch(wake_qpc_ticks, allow_pre_epoch_startup_dispatch)
    {
        Ok(ticks) => ticks,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("playback clock failure: {error}"));
            return WaitBoundary::Exit;
        }
    };
    let wake_error_ticks = match wake_lateness_ticks(wake_elapsed_ticks, deadline_ticks) {
        Ok(ticks) => ticks,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("wait target arithmetic failure: {error}"));
            return WaitBoundary::Exit;
        }
    };
    let wake_error_us = match qpc_clock.duration_to_us(wake_error_ticks) {
        Ok(value) => value,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("QPC conversion failure: {error:?}"));
            return WaitBoundary::Exit;
        }
    };
    match wait_result.outcome {
        WaitOutcome::Deadline => {
            local_metrics.wait_target_error_us =
                local_metrics.wait_target_error_us.max(wake_error_us);
        }
        WaitOutcome::Failed(failure) => {
            local_metrics.wait_path_degraded = true;
            if strict_timing || matches!(failure, WaitFailure::Clock) {
                *force_full_cleanup = true;
                *terminal_error = Some(wait_failure_message(failure));
                return WaitBoundary::Exit;
            }
            std::thread::sleep(Duration::from_micros(500));
            *pending_pre_send_spin_us = 0;
            return WaitBoundary::Continue;
        }
        WaitOutcome::Interrupted => {}
    }
    if input_path_warn_us > 0 && wake_error_us > input_path_warn_us {
        local_metrics.wait_path_degraded = true;
    }
    if wait_result.outcome == WaitOutcome::Interrupted {
        *pending_pre_send_spin_us = 0;
        return WaitBoundary::Continue;
    }
    WaitBoundary::Ready
}
