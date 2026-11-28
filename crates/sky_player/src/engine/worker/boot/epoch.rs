use super::super::super::{DurationTicks, PlaybackClockState, QpcClock, QpcTicks};

pub(super) fn compute_playback_epoch(
    qpc_clock: QpcClock,
    epoch_base: QpcTicks,
    pre_roll_us: u64,
    #[cfg(any(test, feature = "test-support"))] inject_epoch_conversion_failure: bool,
) -> Result<(QpcTicks, PlaybackClockState, u64), String> {
    #[cfg(any(test, feature = "test-support"))]
    if inject_epoch_conversion_failure {
        return Err("injected QPC pre-roll conversion failure".to_string());
    }
    let pre_roll_ticks = qpc_clock.duration_from_us(pre_roll_us).map_err(|error| {
        format!("pre-roll conversion failed before epoch publication: {error:?}")
    })?;
    let epoch_qpc = epoch_base
        .checked_add_duration(pre_roll_ticks)
        .map_err(|error| format!("playback epoch arithmetic failed: {error}"))?;
    let playback = PlaybackClockState::new(epoch_qpc, DurationTicks::ZERO)
        .map_err(|error| format!("playback clock initialization failed: {error}"))?;
    let epoch_wall_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(epoch_qpc.as_u64()))
        .map_err(|error| format!("playback epoch wall-time conversion failed: {error:?}"))?;
    Ok((epoch_qpc, playback, epoch_wall_us))
}
