//! Microsecond-accurate hybrid sleeper and adaptive spin reprobe measurement.

use crate::clock::{
    DurationTicks, QpcError, QpcTicks, qpc_now_ticks, qpc_ticks_to_us, qpc_us_to_ticks,
};
use crate::timer::WaitableTimer;

thread_local! {
    static THREAD_TIMER: Option<WaitableTimer> = WaitableTimer::new();
}

/// Hybrid sleeper: sleeps via high-resolution Waitable Timer up to (target_us - spin_margin_us),
/// then spin-waits until target_us is reached.
/// Returns overshoot in microseconds (actual_us - target_us).
pub fn sleep_until_us(target_us: u64, spin_margin_us: u64) -> Result<u64, QpcError> {
    let now_ticks = qpc_now_ticks()?;
    let now_us = qpc_ticks_to_us(now_ticks)?;
    if target_us <= now_us {
        return Ok(now_us.saturating_sub(target_us));
    }
    let target_ticks = now_ticks
        .checked_add_duration(DurationTicks::from_raw(qpc_us_to_ticks(
            target_us - now_us,
        )?))
        .map_err(|_| QpcError::DeadlineOverflow)?;
    sleep_until_ticks(target_ticks, spin_margin_us)
}

pub fn sleep_until_ticks(target_ticks: QpcTicks, spin_margin_us: u64) -> Result<u64, QpcError> {
    let now_ticks = qpc_now_ticks()?;
    if now_ticks >= target_ticks {
        return qpc_ticks_to_us(QpcTicks::from_raw(
            now_ticks.as_u64().saturating_sub(target_ticks.as_u64()),
        ));
    }

    let remaining_us = qpc_ticks_to_us(QpcTicks::from_raw(
        target_ticks.as_u64().saturating_sub(now_ticks.as_u64()),
    ))?;
    if remaining_us > spin_margin_us {
        let sleep_duration_us = remaining_us - spin_margin_us;
        THREAD_TIMER.with(|timer_opt| {
            if let Some(ref timer) = *timer_opt {
                let _ = timer.sleep_us(sleep_duration_us);
            } else {
                std::thread::sleep(std::time::Duration::from_micros(sleep_duration_us));
            }
        });
    }

    // Spin-wait loop in raw QPC ticks.  Conversion/arithmetic stays out of
    // the hottest loop.
    loop {
        let current_ticks = qpc_now_ticks()?;
        if current_ticks >= target_ticks {
            return qpc_ticks_to_us(QpcTicks::from_raw(
                current_ticks.as_u64().saturating_sub(target_ticks.as_u64()),
            ));
        }
        std::hint::spin_loop();
    }
}

/// Measures spin-loop reprobe / yield overhead over 100 iterations.
pub fn measure_spin_overhead_us() -> Result<u64, QpcError> {
    let mut total_overhead_us = 0;
    const ITERATIONS: u32 = 100;

    for _ in 0..ITERATIONS {
        let t0 = qpc_now_ticks()?;
        std::hint::spin_loop();
        let t1 = qpc_now_ticks()?;
        total_overhead_us +=
            qpc_ticks_to_us(QpcTicks::from_raw(t1.as_u64().saturating_sub(t0.as_u64())))?;
    }

    Ok((total_overhead_us / ITERATIONS as u64).max(1))
}

#[cfg(all(test, feature = "test-support", windows))]
mod tests {
    use super::THREAD_TIMER;
    use crate::timer::test_support;

    #[test]
    fn thread_local_timer_is_dropped_after_repeated_worker_thread_exit() {
        const ITERATIONS: usize = 16;
        let counters = test_support::new_counters();

        for _ in 0..ITERATIONS {
            let counters_for_thread = std::sync::Arc::clone(&counters);
            std::thread::spawn(move || {
                test_support::with_context(&counters_for_thread, || {
                    THREAD_TIMER.with(|timer| {
                        assert!(
                            timer.is_some(),
                            "high-resolution waitable timer creation failed"
                        );
                    });
                });
            })
            .join()
            .expect("timer worker thread must exit without panic");
        }

        let counts = test_support::snapshot(&counters);
        assert_eq!(counts.created, ITERATIONS);
        assert_eq!(counts.dropped, ITERATIONS);
        assert_eq!(counts.live, 0);
    }
}
