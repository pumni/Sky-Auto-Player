//! Microsecond-accurate hybrid sleeper and adaptive spin reprobe measurement.

use crate::clock::{QpcTicks, qpc_now_ticks, qpc_ticks_to_us, qpc_us_to_ticks};
use crate::timer::WaitableTimer;

thread_local! {
    static THREAD_TIMER: Option<WaitableTimer> = WaitableTimer::new();
}

/// Hybrid sleeper: sleeps via high-resolution Waitable Timer up to (target_us - spin_margin_us),
/// then spin-waits until target_us is reached.
/// Returns overshoot in microseconds (actual_us - target_us).
pub fn sleep_until_us(target_us: u64, spin_margin_us: u64) -> u64 {
    let now_ticks = qpc_now_ticks();
    let now_us = qpc_ticks_to_us(now_ticks);
    if target_us <= now_us {
        return now_us.saturating_sub(target_us);
    }
    let target_ticks = QpcTicks(
        now_ticks
            .0
            .saturating_add(qpc_us_to_ticks(target_us - now_us)),
    );
    sleep_until_ticks(target_ticks, spin_margin_us)
}

pub fn sleep_until_ticks(target_ticks: QpcTicks, spin_margin_us: u64) -> u64 {
    let now_ticks = qpc_now_ticks();
    if now_ticks >= target_ticks {
        return qpc_ticks_to_us(QpcTicks(now_ticks.0.saturating_sub(target_ticks.0)));
    }

    let remaining_us = qpc_ticks_to_us(QpcTicks(target_ticks.0.saturating_sub(now_ticks.0)));
    if remaining_us > spin_margin_us {
        let sleep_duration_us = remaining_us - spin_margin_us;
        THREAD_TIMER.with(|timer_opt| {
            if let Some(ref timer) = *timer_opt {
                timer.sleep_us(sleep_duration_us);
            } else {
                std::thread::sleep(std::time::Duration::from_micros(sleep_duration_us));
            }
        });
    }

    // Spin-wait loop in raw QPC ticks.  Conversion/arithmetic stays out of
    // the hottest loop.
    loop {
        let current_ticks = qpc_now_ticks();
        if current_ticks >= target_ticks {
            return qpc_ticks_to_us(QpcTicks(current_ticks.0.saturating_sub(target_ticks.0)));
        }
        std::hint::spin_loop();
    }
}

/// Measures spin-loop reprobe / yield overhead over 100 iterations.
pub fn measure_spin_overhead_us() -> u64 {
    let mut total_overhead_us = 0;
    const ITERATIONS: u32 = 100;

    for _ in 0..ITERATIONS {
        let t0 = qpc_now_ticks();
        std::hint::spin_loop();
        let t1 = qpc_now_ticks();
        total_overhead_us += qpc_ticks_to_us(QpcTicks(t1.0.saturating_sub(t0.0)));
    }

    (total_overhead_us / ITERATIONS as u64).max(1)
}
