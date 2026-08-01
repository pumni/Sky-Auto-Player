//! Monotonic QueryPerformanceCounter clock query helper.

use std::num::NonZeroU64;
use std::sync::OnceLock;

pub use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QpcError {
    FrequencyUnavailable,
    CounterUnavailable,
    DeadlineOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeConversionError {
    InvalidFrequency,
    Overflow,
}

/// Per-worker QPC conversion context. Frequency is captured once and all
/// conversions use that same domain for the lifetime of the worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QpcClock {
    frequency_hz: NonZeroU64,
}

impl QpcClock {
    pub fn initialize() -> Result<Self, QpcError> {
        NonZeroU64::new(qpc_frequency())
            .map(|frequency_hz| Self { frequency_hz })
            .ok_or(QpcError::FrequencyUnavailable)
    }

    pub fn from_frequency_hz(frequency_hz: NonZeroU64) -> Self {
        Self { frequency_hz }
    }

    pub fn frequency_hz(self) -> NonZeroU64 {
        self.frequency_hz
    }

    pub fn now(self) -> Result<QpcTicks, QpcError> {
        qpc_now_ticks_checked()
    }

    pub fn duration_from_us(self, microseconds: u64) -> Result<DurationTicks, TimeConversionError> {
        let numerator = (microseconds as u128)
            .checked_mul(self.frequency_hz.get() as u128)
            .and_then(|value| value.checked_add(999_999))
            .ok_or(TimeConversionError::Overflow)?;
        let ticks = numerator / 1_000_000;
        u64::try_from(ticks)
            .map(DurationTicks::from_raw)
            .map_err(|_| TimeConversionError::Overflow)
    }

    pub fn timeline_from_us(self, microseconds: u64) -> Result<TimelineTicks, TimeConversionError> {
        self.duration_from_us(microseconds)
            .map(|ticks| TimelineTicks::from_raw(ticks.as_u64()))
    }

    pub fn duration_to_us(self, ticks: DurationTicks) -> Result<u64, TimeConversionError> {
        let microseconds = (ticks.as_u64() as u128)
            .checked_mul(1_000_000)
            .ok_or(TimeConversionError::Overflow)?
            / self.frequency_hz.get() as u128;
        u64::try_from(microseconds).map_err(|_| TimeConversionError::Overflow)
    }
}

pub fn qpc_frequency_checked() -> Result<u64, QpcError> {
    let frequency = qpc_frequency();
    (frequency > 0)
        .then_some(frequency)
        .ok_or(QpcError::FrequencyUnavailable)
}

pub fn qpc_now_ticks_checked() -> Result<QpcTicks, QpcError> {
    #[cfg(windows)]
    {
        let mut ticks: i64 = 0;
        // SAFETY: `ticks` is a valid writable out-parameter and the API does
        // not retain its address.
        let success =
            unsafe { windows_sys::Win32::System::Performance::QueryPerformanceCounter(&mut ticks) };
        if success == 0 || ticks < 0 {
            return Err(QpcError::CounterUnavailable);
        }
        Ok(QpcTicks::from_raw(ticks as u64))
    }
    #[cfg(not(windows))]
    {
        let now = {
            use std::time::Instant;
            static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
            QpcTicks::from_raw(START.elapsed().as_nanos().try_into().unwrap_or(u64::MAX))
        };
        Ok(now)
    }
}

pub fn qpc_frequency() -> u64 {
    static FREQUENCY: OnceLock<u64> = OnceLock::new();
    *FREQUENCY.get_or_init(|| {
        #[cfg(windows)]
        {
            let mut freq: i64 = 0;
            // SAFETY: `freq` is a valid writable out-parameter and the API does
            // not retain its address.
            let success = unsafe {
                windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut freq)
            };
            if success == 0 || freq <= 0 {
                return 0;
            }
            freq as u64
        }
        #[cfg(not(windows))]
        {
            1_000_000_000
        }
    })
}

pub fn qpc_now_ticks() -> Result<QpcTicks, QpcError> {
    qpc_now_ticks_checked()
}

pub fn qpc_ticks_to_us(ticks: QpcTicks) -> u64 {
    let frequency = qpc_frequency();
    if frequency == 0 {
        return 0;
    }
    ((ticks.as_u64() as u128).saturating_mul(1_000_000) / frequency as u128).min(u64::MAX as u128)
        as u64
}

pub fn qpc_us_to_ticks(microseconds: u64) -> u64 {
    let frequency = qpc_frequency();
    if frequency == 0 {
        return 0;
    }
    ((microseconds as u128)
        .saturating_mul(frequency as u128)
        .saturating_add(999_999)
        / 1_000_000)
        .min(u64::MAX as u128) as u64
}

pub fn qpc_now_us() -> u64 {
    qpc_now_us_checked().unwrap_or(0)
}

pub fn qpc_now_us_checked() -> Result<u64, QpcError> {
    qpc_now_ticks_checked().map(qpc_ticks_to_us)
}

#[cfg(test)]
mod tests {
    use super::{
        QpcClock, QpcTicks, qpc_frequency, qpc_now_ticks, qpc_ticks_to_us, qpc_us_to_ticks,
    };
    use std::num::NonZeroU64;

    #[test]
    fn qpc_conversion_round_trip_is_monotonic() {
        let one_second = qpc_us_to_ticks(1_000_000);
        assert!(one_second > 0);
        assert!(qpc_ticks_to_us(QpcTicks(one_second)) >= 1_000_000);
        assert_eq!(qpc_ticks_to_us(QpcTicks(qpc_frequency())), 1_000_000);
    }

    #[test]
    fn qpc_ticks_advance() {
        let first = qpc_now_ticks().unwrap();
        let second = qpc_now_ticks().unwrap();
        assert!(second >= first);
    }

    #[test]
    fn checked_clock_conversion_is_monotonic_and_rejects_overflow() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        assert_eq!(clock.timeline_from_us(1).unwrap().as_u64(), 1);
        assert_eq!(
            clock.duration_to_us(super::DurationTicks::from_raw(1_000_000)),
            Ok(1_000_000)
        );
        let high_frequency = QpcClock::from_frequency_hz(NonZeroU64::new(u64::MAX).unwrap());
        assert!(high_frequency.duration_from_us(u64::MAX).is_err());

        let first = clock.timeline_from_us(10).unwrap();
        let second = clock.timeline_from_us(11).unwrap();
        assert!(second > first);
    }
}
