use super::{lease_bounded_ticks, wait_failure_message};
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure, WaitOutcome, WaitResult};
use std::sync::atomic::AtomicU64;
use std::time::Duration;

pub(super) enum WaitBoundary {
    Ready(Option<WaitResult>),
    Continue(WaitResult),
    Exit,
}

#[derive(Clone, Copy, Debug)]
pub struct WaitObservation {
    pub outcome: WaitOutcome,
    pub wake_qpc: Option<QpcTicks>,
    pub spin_ticks: DurationTicks,
    pub deadline_ticks: TimelineTicks,
    pub epoch_qpc: QpcTicks,
    pub allow_pre_epoch_startup_dispatch: bool,
}

pub(super) struct WaitDeadline<'a> {
    pub(super) deadline_ticks: Option<TimelineTicks>,
    pub(super) qpc_clock: QpcClock,
    pub(super) clock_state: &'a mut PlaybackClockState,
    pub(super) allow_pre_epoch_startup_dispatch: bool,
    pub(super) last_send_qpc_ticks: Option<QpcTicks>,
}

pub(super) struct WaitTiming<'a> {
    pub(super) core_warmup_ticks: DurationTicks,
    pub(super) cold_threshold_ticks: DurationTicks,
    pub(super) effective_spin_threshold_ticks: DurationTicks,
    pub(super) lease_timeout_ticks: DurationTicks,
    pub(super) supervisor_heartbeat_ticks: &'a AtomicU64,
}

pub(super) struct WaitSignals<'a> {
    pub(super) waiter: &'a HybridWaiter,
    pub(super) interrupt: &'a OwnedEvent,
    pub(super) strict_timing: bool,
}

pub(super) struct WaitMutable<'a> {
    pub(super) local_metrics: &'a mut WorkerMetricsLocal,
    pub(super) pending_pre_send_spin_us: &'a mut u64,
    pub(super) force_full_cleanup: &'a mut bool,
    pub(super) terminal_error: &'a mut Option<String>,
}

pub(super) struct WaitBoundaryInput<'a> {
    pub(super) deadline: WaitDeadline<'a>,
    pub(super) timing: WaitTiming<'a>,
    pub(super) signals: WaitSignals<'a>,
    pub(super) mutable: WaitMutable<'a>,
}
pub(super) fn wait_for_next_boundary(context: WaitBoundaryInput<'_>) -> WaitBoundary {
    let WaitBoundaryInput {
        deadline,
        timing,
        signals,
        mutable,
    } = context;
    let WaitDeadline {
        deadline_ticks,
        qpc_clock,
        clock_state,
        allow_pre_epoch_startup_dispatch,
        last_send_qpc_ticks,
    } = deadline;
    let WaitTiming {
        core_warmup_ticks,
        cold_threshold_ticks,
        effective_spin_threshold_ticks,
        lease_timeout_ticks,
        supervisor_heartbeat_ticks,
    } = timing;
    let WaitSignals {
        waiter,
        interrupt,
        strict_timing,
    } = signals;
    let WaitMutable {
        local_metrics,
        pending_pre_send_spin_us,
        force_full_cleanup,
        terminal_error,
    } = mutable;

    let Some(deadline_ticks) = deadline_ticks else {
        return WaitBoundary::Exit;
    };

    // Sample QPC and logical elapsed time together to avoid shifting the target.
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
        return WaitBoundary::Ready(None);
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
    match wait_result.outcome {
        WaitOutcome::Deadline => WaitBoundary::Ready(Some(wait_result)),
        WaitOutcome::Failed(failure) => {
            if matches!(failure, WaitFailure::Clock) {
                local_metrics.wait_clock_failures =
                    local_metrics.wait_clock_failures.saturating_add(1);
            } else {
                local_metrics.wait_backend_failures =
                    local_metrics.wait_backend_failures.saturating_add(1);
            }
            if strict_timing || matches!(failure, WaitFailure::Clock) {
                *force_full_cleanup = true;
                *terminal_error = Some(wait_failure_message(failure));
                return WaitBoundary::Exit;
            }
            std::thread::sleep(Duration::from_micros(500));
            *pending_pre_send_spin_us = 0;
            WaitBoundary::Continue(wait_result)
        }
        WaitOutcome::Interrupted => {
            local_metrics.wait_interrupted_count =
                local_metrics.wait_interrupted_count.saturating_add(1);
            *pending_pre_send_spin_us = 0;
            WaitBoundary::Continue(wait_result)
        }
    }
}
