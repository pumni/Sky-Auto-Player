use crate::engine::config::WorkerConfig;
use crate::engine::worker::WorkerTimingState;
use crate::engine::{DurationTicks, PAUSED_POLL_US, QpcClock};

pub(super) struct ConvertedTiming {
    pub(super) effective_min_hold_us: u64,
    pub(super) min_hold_ticks: DurationTicks,
    pub(super) frame_ticks: DurationTicks,
    pub(super) worker_timing: WorkerTimingState,
}

/// Converts the complete worker timing policy once in its captured QPC domain.
/// No conversion depends on a later playback epoch or caller-side clock.
pub(super) fn convert(
    config: &WorkerConfig,
    qpc_clock: QpcClock,
) -> Result<ConvertedTiming, String> {
    let effective_min_hold_us = config.timing.min_hold_us;
    let min_hold_ticks = qpc_clock
        .duration_from_us(effective_min_hold_us)
        .map_err(|error| format!("min-hold conversion failed: {error:?}"))?;
    let frame_ticks = qpc_clock
        .duration_from_us(config.timing.frame_us)
        .map_err(|error| format!("frame-gap conversion failed: {error:?}"))?;
    let timing_margin_ticks = qpc_clock
        .duration_from_us(config.timing.timing_margin_us)
        .map_err(|error| format!("timing-margin conversion failed: {error:?}"))?;
    let strict_down_completion_late_ticks = qpc_clock
        .duration_from_us(config.timing.strict_down_completion_late_us)
        .map_err(|error| format!("strict note-on threshold conversion failed: {error:?}"))?;
    let strict_up_completion_late_ticks = qpc_clock
        .duration_from_us(config.timing.strict_up_completion_late_us)
        .map_err(|error| format!("strict note-off threshold conversion failed: {error:?}"))?;
    let focus_restore_grace_ticks = qpc_clock
        .duration_from_us(config.focus.focus_restore_grace_us)
        .map_err(|error| format!("focus grace conversion failed: {error:?}"))?;
    let paused_poll_ticks = qpc_clock
        .duration_from_us(PAUSED_POLL_US)
        .map_err(|error| format!("paused polling conversion failed: {error:?}"))?;
    let lease_timeout_ticks = qpc_clock
        .duration_from_us(config.wait.supervisor_lease_timeout_us)
        .map_err(|error| format!("lease timeout conversion failed: {error:?}"))?;

    let pre_call_250us_ticks = qpc_clock
        .duration_from_us(250)
        .map_err(|error| format!("SendInput 250us threshold conversion failed: {error:?}"))?;
    let pre_call_500us_ticks = qpc_clock
        .duration_from_us(500)
        .map_err(|error| format!("SendInput 500us threshold conversion failed: {error:?}"))?;
    let pre_call_750us_ticks = qpc_clock
        .duration_from_us(750)
        .map_err(|error| format!("SendInput 750us threshold conversion failed: {error:?}"))?;
    let pre_call_1000us_ticks = qpc_clock
        .duration_from_us(1_000)
        .map_err(|error| format!("SendInput 1000us threshold conversion failed: {error:?}"))?;
    let pre_call_1500us_ticks = qpc_clock
        .duration_from_us(1_500)
        .map_err(|error| format!("SendInput 1500us threshold conversion failed: {error:?}"))?;
    let pre_call_2000us_ticks = qpc_clock
        .duration_from_us(2_000)
        .map_err(|error| format!("SendInput 2000us threshold conversion failed: {error:?}"))?;
    // Preserve the existing floored-microsecond strict-over boundaries using
    // the first QPC tick whose public duration is greater than each bucket.
    let pre_call_2ms_ticks = qpc_clock
        .duration_from_us(2_001)
        .map_err(|error| format!("SendInput 2ms threshold conversion failed: {error:?}"))?;
    let pre_call_5ms_ticks = qpc_clock
        .duration_from_us(5_001)
        .map_err(|error| format!("SendInput 5ms threshold conversion failed: {error:?}"))?;
    let pre_call_10ms_ticks = qpc_clock
        .duration_from_us(10_001)
        .map_err(|error| format!("SendInput 10ms threshold conversion failed: {error:?}"))?;

    Ok(ConvertedTiming {
        effective_min_hold_us,
        min_hold_ticks,
        frame_ticks,
        worker_timing: WorkerTimingState {
            strict_timing: config.timing.strict_timing,
            timing_margin_ticks,
            strict_down_completion_late_ticks,
            strict_up_completion_late_ticks,
            focus_restore_grace_ticks,
            paused_poll_ticks,
            lease_timeout_ticks,
            effective_spin_threshold_ticks: DurationTicks::ZERO,
            pre_call_250us_ticks,
            pre_call_500us_ticks,
            pre_call_750us_ticks,
            pre_call_1000us_ticks,
            pre_call_1500us_ticks,
            pre_call_2000us_ticks,
            pre_call_2ms_ticks,
            pre_call_5ms_ticks,
            pre_call_10ms_ticks,
            start_wall_time_us: 0,
            start_thread_cpu_us: 0,
            start_process_cpu_us: 0,
            last_cpu_metrics_sample_us: 0,
        },
    })
}
