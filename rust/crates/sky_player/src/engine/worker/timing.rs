use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimeArithmeticError, TimelineTicks};
#[cfg(test)]
use sky_dispatch_win32::clock::qpc_us_to_ticks;
use sky_dispatch_win32::clock::{QpcClock, QpcError};
use sky_dispatch_win32::wait::{WaitFailure, WakeErrorStats};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn lease_bounded_ticks(
    target: QpcTicks,
    timeout_ticks: DurationTicks,
    heartbeat_ticks: &AtomicU64,
) -> Result<QpcTicks, QpcError> {
    if timeout_ticks == DurationTicks::ZERO {
        return Ok(target);
    }
    let heartbeat = heartbeat_ticks.load(Ordering::Acquire);
    if heartbeat == 0 {
        return Ok(target);
    }
    let lease_deadline = QpcTicks::from_raw(heartbeat)
        .checked_add_duration(timeout_ticks)
        .map_err(|_| QpcError::DeadlineOverflow)?;
    Ok(target.min(lease_deadline))
}

pub(crate) fn supervisor_lease_expired(
    now_ticks: QpcTicks,
    timeout_ticks: DurationTicks,
    heartbeat_ticks: &AtomicU64,
) -> Result<bool, QpcError> {
    if timeout_ticks == DurationTicks::ZERO {
        return Ok(false);
    }
    let heartbeat = heartbeat_ticks.load(Ordering::Acquire);
    if heartbeat == 0 {
        return Ok(false);
    }
    // The supervisor may publish a heartbeat after the worker sampled `now`.
    // A heartbeat at or beyond that sample is fresh, not a QPC underflow or
    // counter-corruption signal.
    if heartbeat >= now_ticks.as_u64() {
        return Ok(false);
    }
    let elapsed = now_ticks
        .checked_duration_since(QpcTicks::from_raw(heartbeat))
        .map_err(|_| QpcError::CounterUnavailable)?;
    Ok(elapsed > timeout_ticks)
}

pub(crate) fn signed_delta(lhs: u64, rhs: u64) -> i64 {
    let delta = lhs as i128 - rhs as i128;
    delta.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

pub(crate) fn signed_timeline_delta_ticks(
    completed: TimelineTicks,
    deadline: TimelineTicks,
) -> Result<i64, TimeArithmeticError> {
    let (negative, duration) = if completed >= deadline {
        (false, completed.checked_duration_since(deadline)?)
    } else {
        (true, deadline.checked_duration_since(completed)?)
    };
    let magnitude = duration.as_u64();
    if magnitude <= i64::MAX as u64 {
        let magnitude = i64::try_from(magnitude).map_err(|_| TimeArithmeticError::Overflow)?;
        return Ok(if negative { -magnitude } else { magnitude });
    }
    if negative && magnitude == (i64::MAX as u64) + 1 {
        return Ok(i64::MIN);
    }
    Err(TimeArithmeticError::Overflow)
}

pub(crate) fn wake_lateness_ticks(
    wake: TimelineTicks,
    deadline: TimelineTicks,
) -> Result<DurationTicks, TimeArithmeticError> {
    match wake.checked_duration_since(deadline) {
        Ok(duration) => Ok(duration),
        Err(TimeArithmeticError::NegativeOrder) => Ok(DurationTicks::ZERO),
        Err(error) => Err(error),
    }
}

pub(crate) fn signed_ticks_to_us(qpc_clock: QpcClock, delta_ticks: i64) -> Result<i64, String> {
    let magnitude = delta_ticks.unsigned_abs();
    let microseconds = qpc_clock
        .duration_to_us(DurationTicks::from_raw(magnitude))
        .map_err(|error| format!("{error:?}"))?;
    let signed = if delta_ticks < 0 {
        -i128::from(microseconds)
    } else {
        i128::from(microseconds)
    };
    i64::try_from(signed).map_err(|_| "signed timing delta exceeds i64 range".to_string())
}

/// Preserve the distinction between a logical operation and one SendInput
/// syscall. The operation spans the first call entry through the final call
/// return; a single-call duration is only valid for exactly one non-rollback
/// call.
#[cfg(test)]
pub(crate) fn exact_sender_durations(
    qpc_clock: QpcClock,
    started_ticks: Option<QpcTicks>,
    completed_ticks: Option<QpcTicks>,
    send_attempts: u8,
    rollback_call: bool,
) -> Result<(Option<u64>, Option<u64>), QpcError> {
    if send_attempts == 0 {
        return Ok((None, None));
    }
    let started = started_ticks.ok_or(QpcError::CounterUnavailable)?;
    let completed = completed_ticks.ok_or(QpcError::CounterUnavailable)?;
    let duration = completed
        .checked_duration_since(started)
        .map_err(|_| QpcError::CounterUnavailable)
        .and_then(|ticks| {
            qpc_clock
                .duration_to_us(ticks)
                .map_err(|_| QpcError::ConversionOverflow)
        })?;
    let single_call = (send_attempts == 1 && !rollback_call).then_some(duration);
    Ok((Some(duration), single_call))
}

#[cfg(test)]
pub(crate) fn anchored_dispatch_target_ticks_typed(
    _now_ticks: QpcTicks,
    anchor_ticks: QpcTicks,
    scheduled_ticks: TimelineTicks,
    lead_ticks: DurationTicks,
) -> Result<QpcTicks, QpcError> {
    let authored_target = anchor_ticks
        .checked_add_duration(DurationTicks::from_raw(scheduled_ticks.as_u64()))
        .map_err(|_| QpcError::DeadlineOverflow)?;
    let target = authored_target
        .as_u64()
        .checked_sub(lead_ticks.as_u64())
        .map(QpcTicks::from_raw)
        .ok_or(QpcError::DeadlineOverflow)?;
    Ok(target)
}

/// Map an authored timestamp minus lead to its exact absolute QPC boundary.
#[cfg(test)]
#[allow(clippy::manual_unwrap_or, clippy::manual_unwrap_or_default)]
pub(crate) fn anchored_dispatch_target_ticks(
    qpc_clock: QpcClock,
    now_ticks: QpcTicks,
    now_qpc_us: u64,
    anchor_us: u64,
    scheduled_us: u64,
    lead_us: u64,
) -> Result<QpcTicks, QpcError> {
    let target_us = match anchor_us
        .checked_add(scheduled_us)
        .ok_or(QpcError::DeadlineOverflow)?
        .checked_sub(lead_us)
    {
        Some(value) => value,
        None => 0,
    };
    if target_us >= now_qpc_us {
        let delta = qpc_clock
            .duration_from_us(
                target_us
                    .checked_sub(now_qpc_us)
                    .ok_or(QpcError::DeadlineOverflow)?,
            )
            .map_err(|_| QpcError::DeadlineOverflow)?;
        return now_ticks
            .checked_add_duration(delta)
            .map_err(|_| QpcError::DeadlineOverflow);
    }
    let delta = qpc_clock
        .duration_from_us(
            now_qpc_us
                .checked_sub(target_us)
                .ok_or(QpcError::DeadlineOverflow)?,
        )
        .map_err(|_| QpcError::DeadlineOverflow)?;
    now_ticks
        .as_u64()
        .checked_sub(delta.as_u64())
        .map(QpcTicks::from_raw)
        .ok_or(QpcError::DeadlineOverflow)
}

/// Legacy relative helper retained for unit-test compatibility.
#[cfg(test)]
pub(crate) fn deadline_target_ticks(
    now_ticks: QpcTicks,
    logical_now_us: u64,
    deadline_us: u64,
) -> QpcTicks {
    QpcTicks::from_raw(now_ticks.as_u64().saturating_add(
        qpc_us_to_ticks(deadline_us.saturating_sub(logical_now_us)).expect("test QPC conversion"),
    ))
}

pub(crate) fn wait_failure_message(failure: WaitFailure) -> String {
    match failure {
        WaitFailure::TimerCreate { win32_error } => {
            format!("high-resolution waitable timer creation failed (Win32 error {win32_error})")
        }
        WaitFailure::TimerArm { win32_error } => {
            format!("high-resolution waitable timer arm failed (Win32 error {win32_error})")
        }
        WaitFailure::TimerWait { win32_error } => {
            format!("high-resolution waitable timer wait failed (Win32 error {win32_error})")
        }
        WaitFailure::MultiWait { win32_error } => {
            format!("interruptible wait failed (Win32 error {win32_error})")
        }
        WaitFailure::Clock => "QPC failed during real-time wait".to_string(),
    }
}

/// Choose the one precision-spin threshold for a production session.
///
/// The threshold is derived once from the startup probe.  The cap preserves
/// the conservative fixed-spin fallback, while the floor is the lowest
/// candidate retained by the fixed-spin acceptance matrix.
pub(crate) fn calibrated_spin_threshold_us(stats: WakeErrorStats) -> u64 {
    stats
        .p99_us
        .max(stats.robust_us)
        .saturating_add(super::super::config::CALIBRATION_SAFETY_MARGIN_US)
        .clamp(
            super::super::config::MIN_CALIBRATED_SPIN_US,
            super::super::config::DEFAULT_SPIN_THRESHOLD_US,
        )
}

pub(crate) fn select_spin_threshold_us(stats: Option<WakeErrorStats>) -> u64 {
    stats
        .map(calibrated_spin_threshold_us)
        .unwrap_or(super::super::config::DEFAULT_SPIN_THRESHOLD_US)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn derive_spin_threshold_us(wake_error_us: u64, spin_floor_us: u64) -> u64 {
    wake_error_us
        .saturating_add(200)
        .clamp(spin_floor_us, 3_000)
}

#[cfg(test)]
pub(crate) fn adjust_spin_threshold(current_us: u64, candidate_us: u64) -> u64 {
    if candidate_us >= current_us {
        candidate_us
    } else {
        current_us.saturating_sub(current_us.saturating_sub(candidate_us).min(50))
    }
}

#[cfg(test)]
mod tests {
    use super::{calibrated_spin_threshold_us, select_spin_threshold_us};
    use crate::engine::config::{DEFAULT_SPIN_THRESHOLD_US, MIN_CALIBRATED_SPIN_US};
    use sky_dispatch_win32::wait::WakeErrorStats;

    #[test]
    fn low_wake_error_selects_the_floor() {
        assert_eq!(
            calibrated_spin_threshold_us(WakeErrorStats {
                p99_us: 80,
                robust_us: 120,
                ..WakeErrorStats::default()
            }),
            MIN_CALIBRATED_SPIN_US
        );
    }

    #[test]
    fn high_wake_error_saturates_at_the_fallback() {
        assert_eq!(
            calibrated_spin_threshold_us(WakeErrorStats {
                p99_us: 900,
                robust_us: 1_500,
                ..WakeErrorStats::default()
            }),
            DEFAULT_SPIN_THRESHOLD_US
        );
    }

    #[test]
    fn failed_calibration_uses_the_fallback() {
        assert_eq!(select_spin_threshold_us(None), DEFAULT_SPIN_THRESHOLD_US);
    }

    #[test]
    fn calibration_stays_within_the_policy_bounds_and_is_frozen_for_same_stats() {
        for stats in [
            WakeErrorStats::default(),
            WakeErrorStats {
                p99_us: 250,
                robust_us: 400,
                ..WakeErrorStats::default()
            },
            WakeErrorStats {
                p99_us: 10_000,
                robust_us: 20_000,
                ..WakeErrorStats::default()
            },
        ] {
            let first = select_spin_threshold_us(Some(stats));
            let second = select_spin_threshold_us(Some(stats));
            assert_eq!(first, second);
            assert!((MIN_CALIBRATED_SPIN_US..=DEFAULT_SPIN_THRESHOLD_US).contains(&first));
        }
    }
}
