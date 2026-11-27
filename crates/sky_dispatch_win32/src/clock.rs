//! Monotonic QueryPerformanceCounter clock query helper.

use std::num::NonZeroU64;
use std::sync::OnceLock;

#[cfg(any(test, feature = "qpc-read-counting"))]
use std::sync::atomic::AtomicU64;
#[cfg(any(test, feature = "qpc-read-counting"))]
use std::sync::atomic::Ordering;

pub use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QpcError {
    FrequencyUnavailable,
    CounterUnavailable,
    DeadlineOverflow,
    ConversionOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeConversionError {
    InvalidFrequency,
    Overflow,
}

#[cfg(any(test, feature = "qpc-read-counting"))]
static QPC_READ_COUNT: AtomicU64 = AtomicU64::new(0);

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    static QPC_NOW_FAULT_INJECTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static QPC_NOW_OVERRIDE_TICKS: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
/// Faults QPC reads on the calling test thread only.
pub fn set_qpc_now_fault_for_test(fault: bool) {
    QPC_NOW_FAULT_INJECTED.with(|injected| injected.set(fault));
}

#[cfg(any(test, feature = "test-support"))]
/// Overrides QPC now reads on the calling test thread only with a fixed tick value.
pub fn set_qpc_now_override_for_test(ticks: Option<QpcTicks>) {
    QPC_NOW_OVERRIDE_TICKS.with(|cell| cell.set(ticks.map(|t| t.as_u64())));
}

#[cfg(any(test, feature = "qpc-read-counting"))]
#[inline]
fn record_qpc_read() {
    QPC_READ_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Reset the test-support QPC probe.  It is deliberately absent from normal
/// builds so the production clock path remains a single platform query.
#[cfg(any(test, feature = "qpc-read-counting"))]
pub fn reset_qpc_read_count() {
    QPC_READ_COUNT.store(0, Ordering::Relaxed);
}

/// Return the number of `QueryPerformanceCounter`/clock-seam reads since the
/// last reset.  Tests use this to catch a redundant post-wake sample.
#[cfg(any(test, feature = "qpc-read-counting"))]
pub fn qpc_read_count() -> u64 {
    QPC_READ_COUNT.load(Ordering::Relaxed)
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

    pub fn timeline_to_us(self, ticks: TimelineTicks) -> Result<u64, TimeConversionError> {
        self.duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
    }

    /// Convert 100-nanosecond intervals to QPC duration ticks with ceiling rounding.
    ///
    /// Ceiling rounding guarantees that physical timing policies such as the
    /// minimum hold are never undercut.
    pub fn duration_from_100ns(
        self,
        intervals_100ns: u64,
    ) -> Result<DurationTicks, TimeConversionError> {
        let numerator = (intervals_100ns as u128)
            .checked_mul(self.frequency_hz.get() as u128)
            .and_then(|value| value.checked_add(9_999_999))
            .ok_or(TimeConversionError::Overflow)?;
        let ticks = numerator / 10_000_000;
        u64::try_from(ticks)
            .map(DurationTicks::from_raw)
            .map_err(|_| TimeConversionError::Overflow)
    }

    /// Convert 100-nanosecond intervals to QPC duration ticks with floor rounding.
    pub fn duration_from_100ns_floor(
        self,
        intervals_100ns: u64,
    ) -> Result<DurationTicks, TimeConversionError> {
        let numerator = (intervals_100ns as u128)
            .checked_mul(self.frequency_hz.get() as u128)
            .ok_or(TimeConversionError::Overflow)?;
        let ticks = numerator / 10_000_000;
        u64::try_from(ticks)
            .map(DurationTicks::from_raw)
            .map_err(|_| TimeConversionError::Overflow)
    }

    pub fn timeline_from_100ns(
        self,
        intervals_100ns: u64,
    ) -> Result<TimelineTicks, TimeConversionError> {
        self.duration_from_100ns(intervals_100ns)
            .map(|ticks| TimelineTicks::from_raw(ticks.as_u64()))
    }

    /// Convert QPC duration ticks to 100-nanosecond intervals with floor rounding.
    ///
    /// Floor rounding ensures that kernel wait durations armed via waitable timers
    /// never sleep past the scheduled deadline, waking slightly early into the
    /// final precision spin window.
    pub fn duration_to_100ns(self, ticks: DurationTicks) -> Result<u64, TimeConversionError> {
        let intervals = (ticks.as_u64() as u128)
            .checked_mul(10_000_000)
            .ok_or(TimeConversionError::Overflow)?
            / self.frequency_hz.get() as u128;
        u64::try_from(intervals).map_err(|_| TimeConversionError::Overflow)
    }

    /// Convert QPC duration ticks to 100-nanosecond intervals with ceiling rounding.
    ///
    /// Ceiling rounding guarantees that physical floor invariants expressed in 100ns
    /// never underestimate the elapsed duration.
    pub fn duration_to_100ns_ceil(self, ticks: DurationTicks) -> Result<u64, TimeConversionError> {
        let freq = self.frequency_hz.get() as u128;
        let numerator = (ticks.as_u64() as u128)
            .checked_mul(10_000_000)
            .and_then(|value| value.checked_add(freq - 1))
            .ok_or(TimeConversionError::Overflow)?;
        let intervals = numerator / freq;
        u64::try_from(intervals).map_err(|_| TimeConversionError::Overflow)
    }

    pub fn timeline_to_100ns(self, ticks: TimelineTicks) -> Result<u64, TimeConversionError> {
        self.duration_to_100ns(DurationTicks::from_raw(ticks.as_u64()))
    }
}

pub fn qpc_frequency_checked() -> Result<u64, QpcError> {
    let frequency = qpc_frequency();
    (frequency > 0)
        .then_some(frequency)
        .ok_or(QpcError::FrequencyUnavailable)
}

pub fn qpc_now_ticks_checked() -> Result<QpcTicks, QpcError> {
    #[cfg(any(test, feature = "qpc-read-counting"))]
    record_qpc_read();
    #[cfg(any(test, feature = "test-support"))]
    if QPC_NOW_FAULT_INJECTED.with(std::cell::Cell::get) {
        return Err(QpcError::CounterUnavailable);
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(ticks) = QPC_NOW_OVERRIDE_TICKS.with(std::cell::Cell::get) {
        return Ok(QpcTicks::from_raw(ticks));
    }
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
            let nanos = START
                .elapsed()
                .as_nanos()
                .try_into()
                .map_err(|_| QpcError::CounterUnavailable)?;
            QpcTicks::from_raw(nanos)
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

pub fn qpc_ticks_to_us(ticks: QpcTicks) -> Result<u64, QpcError> {
    let frequency = NonZeroU64::new(qpc_frequency()).ok_or(QpcError::FrequencyUnavailable)?;
    QpcClock::from_frequency_hz(frequency)
        .timeline_to_us(TimelineTicks::from_raw(ticks.as_u64()))
        .map_err(|_| QpcError::ConversionOverflow)
}

pub fn qpc_us_to_ticks(microseconds: u64) -> Result<u64, QpcError> {
    let frequency = NonZeroU64::new(qpc_frequency()).ok_or(QpcError::FrequencyUnavailable)?;
    QpcClock::from_frequency_hz(frequency)
        .duration_from_us(microseconds)
        .map(|ticks| ticks.as_u64())
        .map_err(|_| QpcError::ConversionOverflow)
}

pub fn qpc_now_us() -> Result<u64, QpcError> {
    qpc_now_us_checked()
}

pub fn qpc_now_us_checked() -> Result<u64, QpcError> {
    qpc_now_ticks_checked().and_then(qpc_ticks_to_us)
}

#[cfg(test)]
mod tests {
    use super::{
        QpcClock, QpcTicks, qpc_frequency, qpc_now_ticks, qpc_ticks_to_us, qpc_us_to_ticks,
    };
    use std::num::NonZeroU64;

    #[test]
    fn qpc_conversion_round_trip_is_monotonic() {
        let one_second = qpc_us_to_ticks(1_000_000).unwrap();
        assert!(one_second > 0);
        assert!(qpc_ticks_to_us(QpcTicks::from_raw(one_second)).unwrap() >= 1_000_000);
        assert_eq!(
            qpc_ticks_to_us(QpcTicks::from_raw(qpc_frequency())).unwrap(),
            1_000_000
        );
    }

    #[test]
    fn qpc_ticks_advance() {
        let first = qpc_now_ticks().unwrap();
        let second = qpc_now_ticks().unwrap();
        assert!(second >= first);
    }

    #[test]
    fn qpc_fault_injection_is_local_to_the_test_thread() {
        struct ResetFault;
        impl Drop for ResetFault {
            fn drop(&mut self) {
                super::set_qpc_now_fault_for_test(false);
            }
        }

        let _reset_fault = ResetFault;
        super::set_qpc_now_fault_for_test(true);
        assert_eq!(qpc_now_ticks(), Err(super::QpcError::CounterUnavailable));

        std::thread::spawn(qpc_now_ticks)
            .join()
            .expect("QPC reader thread")
            .expect("fault must not leak to another test thread");
        assert_eq!(qpc_now_ticks(), Err(super::QpcError::CounterUnavailable));
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

    #[test]
    fn qpc_conversion_odd_frequency_pm_timer_3579545hz() {
        // Classic ACPI PM-timer / NTSC frequency: 3,579,545 Hz (~0.279 us per tick).
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(3_579_545).unwrap());
        let tolerances_us = [
            2_000, 2_500, 3_000, 3_500, 4_000, 4_500, 5_000, 5_500, 6_000, 6_500, 7_000, 7_500,
            8_000, 8_500, 9_000, 9_500, 10_000,
        ];

        let mut prev_ticks = 0u64;
        for &us in &tolerances_us {
            let ticks = clock.duration_from_us(us).expect("convert to ticks");
            assert!(
                ticks.as_u64() > prev_ticks,
                "ticks must be strictly monotonic: {ticks:?} > {prev_ticks}"
            );
            prev_ticks = ticks.as_u64();

            // Duration in ticks must never underestimate authored duration (ceiling guarantee)
            let tick_nanos_approx = (ticks.as_u64() as u128) * 1_000_000;
            let target_nanos_approx = (us as u128) * 3_579_545;
            assert!(
                tick_nanos_approx >= target_nanos_approx,
                "ticks must not underestimate duration: {tick_nanos_approx} >= {target_nanos_approx}"
            );

            // Ceiling division adds at most 1 tick of overshoot
            let overshoot_nanos = tick_nanos_approx - target_nanos_approx;
            assert!(
                overshoot_nanos < 3_579_545,
                "overshoot must be strictly less than 1 tick: {overshoot_nanos} < 3579545"
            );

            // Round-trip floor conversion returns the exact authored microsecond value
            let round_trip_us = clock.duration_to_us(ticks).expect("convert to us");
            assert_eq!(
                round_trip_us, us,
                "round-trip must preserve exact microsecond for {us} us"
            );
        }
    }

    #[test]
    fn qpc_100ns_conversion_exact_at_10mhz() {
        // Modern Windows standard frequency: 10 MHz (10_000_000 Hz).
        // 1 tick == exactly 1 100-ns interval.
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        for intervals in [
            0,
            1,
            10,
            100,
            1_000,
            10_000,
            1_000_000,
            10_000_000,
            864_000_000_000,
        ] {
            let ticks = clock.duration_from_100ns(intervals).unwrap();
            assert_eq!(
                ticks.as_u64(),
                intervals,
                "10MHz duration_from_100ns identity"
            );

            let back_intervals = clock.duration_to_100ns(ticks).unwrap();
            assert_eq!(
                back_intervals, intervals,
                "10MHz duration_to_100ns identity"
            );

            let back_ceil = clock.duration_to_100ns_ceil(ticks).unwrap();
            assert_eq!(
                back_ceil, intervals,
                "10MHz duration_to_100ns_ceil identity"
            );
        }
    }

    #[test]
    fn qpc_100ns_conversion_realistic_frequencies_property_matrix() {
        let realistic_frequencies = [
            1_000_000,     // 1 MHz (virtualized)
            3_579_545,     // 3.58 MHz (ACPI PM timer)
            10_000_000,    // 10 MHz (Windows invariant TSC)
            14_318_180,    // 14.3 MHz (HPET)
            19_200_000,    // 19.2 MHz (ARM64 Snapdragon)
            24_000_000,    // 24 MHz (Intel Crystal Clock)
            1_000_000_000, // 1 GHz (Direct TSC)
        ];

        let test_intervals = [
            1, 2, 5, 10, 15, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 16_667, 100_000,
            1_000_000, 10_000_000,
        ];

        for &freq in &realistic_frequencies {
            let clock = QpcClock::from_frequency_hz(NonZeroU64::new(freq).unwrap());

            for &intervals in &test_intervals {
                let ticks_ceil = clock.duration_from_100ns(intervals).unwrap();
                let ticks_floor = clock.duration_from_100ns_floor(intervals).unwrap();

                assert!(
                    ticks_ceil.as_u64() >= ticks_floor.as_u64(),
                    "ceil must be >= floor for freq={freq}, intervals={intervals}"
                );

                // Invariant 1: Physical floor guarantee.
                // Converting ticks_ceil back to 100ns with ceil must never undercut intervals.
                let back_ceil = clock.duration_to_100ns_ceil(ticks_ceil).unwrap();
                assert!(
                    back_ceil >= intervals,
                    "physical floor guarantee violated: back_ceil={back_ceil} < intervals={intervals} at freq={freq}"
                );

                // Invariant 2: Deadline guarantee.
                // Converting ticks_ceil to 100ns with floor must never exceed the deadline.
                let back_floor = clock.duration_to_100ns(ticks_ceil).unwrap();
                let converted_back_floor = clock.duration_from_100ns_floor(back_floor).unwrap();
                assert!(
                    converted_back_floor.as_u64() <= ticks_ceil.as_u64(),
                    "deadline overshoot violated: floor ticks={converted_back_floor:?} > target={ticks_ceil:?} at freq={freq}"
                );

                // Invariant 3: Precision bounds.
                // 100ns conversion error must be strictly within 1 interval (0.1 us).
                let diff_intervals = back_ceil.saturating_sub(back_floor);
                assert!(
                    diff_intervals <= 1,
                    "quantization error exceeded 1 interval: diff={diff_intervals} for freq={freq}"
                );
            }
        }
    }

    #[test]
    fn qpc_100ns_conversion_overflow_safety() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        // Day-long durations do not overflow
        let day_100ns = 86_400 * 10_000_000;
        assert!(clock.duration_from_100ns(day_100ns).is_ok());

        // Extreme values near u64::MAX reject with Overflow error cleanly
        let high_clock = QpcClock::from_frequency_hz(NonZeroU64::new(u64::MAX).unwrap());
        assert_eq!(
            high_clock.duration_from_100ns(u64::MAX),
            Err(super::TimeConversionError::Overflow)
        );

        let low_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1).unwrap());
        assert_eq!(
            low_clock.duration_to_100ns(super::DurationTicks::from_raw(u64::MAX)),
            Err(super::TimeConversionError::Overflow)
        );
        assert_eq!(
            low_clock.duration_to_100ns_ceil(super::DurationTicks::from_raw(u64::MAX)),
            Err(super::TimeConversionError::Overflow)
        );
    }
}
