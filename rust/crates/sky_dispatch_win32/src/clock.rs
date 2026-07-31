//! Monotonic QueryPerformanceCounter clock query helper.

use std::sync::OnceLock;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct QpcTicks(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QpcError {
    FrequencyUnavailable,
    CounterUnavailable,
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
        Ok(QpcTicks(ticks as u64))
    }
    #[cfg(not(windows))]
    {
        Ok(qpc_now_ticks())
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

pub fn qpc_now_ticks() -> QpcTicks {
    #[cfg(windows)]
    {
        let mut ticks: i64 = 0;
        // SAFETY: `ticks` is a valid writable out-parameter and the API does
        // not retain its address.
        let success =
            unsafe { windows_sys::Win32::System::Performance::QueryPerformanceCounter(&mut ticks) };
        if success == 0 || ticks < 0 {
            return QpcTicks(0);
        }
        QpcTicks(ticks as u64)
    }
    #[cfg(not(windows))]
    {
        use std::time::Instant;
        static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
        QpcTicks(START.elapsed().as_nanos().min(u64::MAX as u128) as u64)
    }
}

pub fn qpc_ticks_to_us(ticks: QpcTicks) -> u64 {
    let frequency = qpc_frequency();
    if frequency == 0 {
        return 0;
    }
    ((ticks.0 as u128).saturating_mul(1_000_000) / frequency as u128).min(u64::MAX as u128) as u64
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
    qpc_ticks_to_us(qpc_now_ticks())
}

#[cfg(test)]
mod tests {
    use super::{QpcTicks, qpc_frequency, qpc_now_ticks, qpc_ticks_to_us, qpc_us_to_ticks};

    #[test]
    fn qpc_conversion_round_trip_is_monotonic() {
        let one_second = qpc_us_to_ticks(1_000_000);
        assert!(one_second > 0);
        assert!(qpc_ticks_to_us(QpcTicks(one_second)) >= 1_000_000);
        assert_eq!(qpc_ticks_to_us(QpcTicks(qpc_frequency())), 1_000_000);
    }

    #[test]
    fn qpc_ticks_advance() {
        let first = qpc_now_ticks();
        let second = qpc_now_ticks();
        assert!(second >= first);
    }
}
