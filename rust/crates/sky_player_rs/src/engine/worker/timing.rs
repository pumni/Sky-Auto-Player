use crate::engine::telemetry::WorkerMetricsLocal;
#[cfg(test)]
use sky_dispatch_core::estimator::LatencyClass;
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
pub(crate) fn classify_latency_class(
    last_send_qpc_ticks: Option<QpcTicks>,
    now_qpc_ticks: QpcTicks,
    cold_threshold_ticks: DurationTicks,
) -> Result<LatencyClass, TimeArithmeticError> {
    let Some(last) = last_send_qpc_ticks else {
        return Ok(LatencyClass::Cold);
    };
    let gap = now_qpc_ticks.checked_duration_since(last)?;
    Ok(if gap > cold_threshold_ticks {
        LatencyClass::Cold
    } else {
        LatencyClass::Hot
    })
}

pub(crate) fn anchored_dispatch_target_ticks_typed(
    now_ticks: QpcTicks,
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
    Ok(target.max(now_ticks))
}

/// Map an authored timestamp minus lead, including the negative interval that
/// is intentionally needed for a first note at authored t=0.
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
    if target_us <= now_qpc_us {
        return Ok(now_ticks);
    }
    let delta = qpc_clock
        .duration_from_us(
            target_us
                .checked_sub(now_qpc_us)
                .ok_or(QpcError::DeadlineOverflow)?,
        )
        .map_err(|_| QpcError::DeadlineOverflow)?;
    now_ticks
        .checked_add_duration(delta)
        .map_err(|_| QpcError::DeadlineOverflow)
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

pub(crate) fn publish_wake_error_stats(
    stats: WakeErrorStats,
    local_metrics: &mut WorkerMetricsLocal,
) {
    local_metrics.wake_error_p50_us = stats.p50_us;
    local_metrics.wake_error_p95_us = stats.p95_us;
    local_metrics.wake_error_p99_us = stats.p99_us;
    local_metrics.wake_error_max_us = stats.max_us;
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
