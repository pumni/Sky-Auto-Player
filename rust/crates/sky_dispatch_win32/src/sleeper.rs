//! Microsecond-accurate hybrid sleeper and adaptive spin reprobe measurement.

use crate::clock::qpc_now_us;
use crate::timer::WaitableTimer;

thread_local! {
    static THREAD_TIMER: Option<WaitableTimer> = WaitableTimer::new();
}

/// Hybrid sleeper: sleeps via high-resolution Waitable Timer up to (target_us - spin_margin_us),
/// then spin-waits until target_us is reached.
/// Returns overshoot in microseconds (actual_us - target_us).
pub fn sleep_until_us(target_us: u64, spin_margin_us: u64) -> u64 {
    let now_us = qpc_now_us();
    if now_us >= target_us {
        return now_us - target_us;
    }

    let remaining_us = target_us - now_us;
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

    // Spin-wait loop until target_us
    loop {
        let current_us = qpc_now_us();
        if current_us >= target_us {
            return current_us - target_us;
        }
        std::hint::spin_loop();
    }
}

/// Measures spin-loop reprobe / yield overhead over 100 iterations.
pub fn measure_spin_overhead_us() -> u64 {
    let mut total_overhead_us = 0;
    const ITERATIONS: u32 = 100;

    for _ in 0..ITERATIONS {
        let t0 = qpc_now_us();
        std::hint::spin_loop();
        let t1 = qpc_now_us();
        total_overhead_us += t1.saturating_sub(t0);
    }

    (total_overhead_us / ITERATIONS as u64).max(1)
}
