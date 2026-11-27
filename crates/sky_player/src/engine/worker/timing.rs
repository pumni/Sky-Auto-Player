use crate::engine::shared::SupervisorLeaseState;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimeArithmeticError, TimelineTicks};
#[cfg(any(test, feature = "test-support"))]
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::clock::QpcError;
#[cfg(test)]
use sky_dispatch_win32::clock::qpc_us_to_ticks;
use sky_dispatch_win32::wait::WaitFailure;

pub(crate) fn supervisor_lease_expired(
    now_ticks: QpcTicks,
    timeout_ticks: DurationTicks,
    lease: &SupervisorLeaseState,
) -> Result<bool, QpcError> {
    if timeout_ticks == DurationTicks::ZERO {
        return Ok(false);
    }
    Ok(lease.check_expired(now_ticks, timeout_ticks))
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

#[cfg(any(test, feature = "test-support"))]
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

/// Map an authored timestamp minus lead to its exact absolute QPC boundary.
#[cfg(test)]
pub(crate) fn anchored_dispatch_target_ticks(
    qpc_clock: QpcClock,
    now_ticks: QpcTicks,
    now_qpc_us: u64,
    anchor_us: u64,
    scheduled_us: u64,
    lead_us: u64,
) -> Result<QpcTicks, QpcError> {
    let target_us = anchor_us
        .checked_add(scheduled_us)
        .ok_or(QpcError::DeadlineOverflow)?
        .saturating_sub(lead_us);
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
