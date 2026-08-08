use super::{
    WorkerHealthState, lease_bounded_ticks, observe_wait_health, wait_failure_message,
    wake_lateness_ticks,
};
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

pub(super) fn drain_wait_observation(
    observation: &WaitObservation,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
) -> Result<(), super::DispatchStep> {
    let spin_us = qpc_clock
        .duration_to_us(observation.spin_ticks)
        .map_err(|error| {
            super::DispatchStep::Terminate(format!(
                "wait observer spin conversion failure: {error:?}"
            ))
        })?;
    local_metrics.idle_wake_count = local_metrics.idle_wake_count.saturating_add(1);
    local_metrics.spin_time_us = local_metrics.spin_time_us.saturating_add(spin_us);
    if !matches!(observation.outcome, WaitOutcome::Deadline) {
        return Ok(());
    }
    let wake_qpc = observation.wake_qpc.ok_or_else(|| {
        super::DispatchStep::Terminate("wait observer missing deadline QPC evidence".to_string())
    })?;
    let wake_elapsed_ticks =
        if observation.allow_pre_epoch_startup_dispatch && wake_qpc < observation.epoch_qpc {
            TimelineTicks::ZERO
        } else {
            let elapsed = wake_qpc
                .checked_duration_since(observation.epoch_qpc)
                .map_err(|error| {
                    super::DispatchStep::Terminate(format!(
                        "wait observer QPC ordering failure: {error:?}"
                    ))
                })?;
            TimelineTicks::from_raw(elapsed.as_u64())
        };
    let wake_error_ticks = wake_lateness_ticks(wake_elapsed_ticks, observation.deadline_ticks)
        .map_err(|error| {
            super::DispatchStep::Terminate(format!(
                "wait observer target arithmetic failure: {error}"
            ))
        })?;
    let wake_error_us = qpc_clock
        .duration_to_us(wake_error_ticks)
        .map_err(|error| {
            super::DispatchStep::Terminate(format!(
                "wait observer wake conversion failure: {error:?}"
            ))
        })?;
    local_metrics.wait_target_error_us = local_metrics.wait_target_error_us.max(wake_error_us);
    let elapsed_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(wake_elapsed_ticks.as_u64()))
        .map_err(|error| {
            super::DispatchStep::Terminate(format!(
                "wait observer elapsed conversion failure: {error:?}"
            ))
        })?;
    observe_wait_health(
        wake_error_us,
        health.options.wait_warn_us,
        elapsed_us,
        health.options.window_policy(),
        &mut health.wait_window,
        local_metrics,
    );
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU64;

    #[test]
    fn wait_observation_defers_conversion_and_health_updates() {
        let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let mut health = WorkerHealthState::new(super::super::DispatchHealthOptions::default());
        let mut local_metrics = WorkerMetricsLocal::default();
        let observation = WaitObservation {
            outcome: WaitOutcome::Deadline,
            wake_qpc: Some(QpcTicks::from_raw(2_500)),
            spin_ticks: DurationTicks::from_raw(100),
            deadline_ticks: TimelineTicks::from_raw(1_000),
            epoch_qpc: QpcTicks::from_raw(1_000),
            allow_pre_epoch_startup_dispatch: false,
        };

        drain_wait_observation(&observation, &mut health, &mut local_metrics, qpc_clock)
            .expect("raw wait observation should be drainable");

        assert_eq!(local_metrics.idle_wake_count, 1);
        assert_eq!(local_metrics.spin_time_us, 100);
        assert_eq!(local_metrics.wait_target_error_us, 500);
        assert_eq!(local_metrics.wait_degraded_samples, 1);
        assert_eq!(local_metrics.wait_window_sample_count, 1);
    }
}
