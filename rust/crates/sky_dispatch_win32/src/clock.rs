//! Monotonic QueryPerformanceCounter clock query helper.

use std::sync::OnceLock;

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

pub fn qpc_now_us() -> u64 {
    #[cfg(windows)]
    {
        let mut ticks: i64 = 0;
        // SAFETY: `ticks` is a valid writable out-parameter and the API does
        // not retain its address.
        let success =
            unsafe { windows_sys::Win32::System::Performance::QueryPerformanceCounter(&mut ticks) };
        if success == 0 || ticks < 0 {
            return 0;
        }
        let freq = qpc_frequency();
        if freq == 0 {
            return 0;
        }
        u64::try_from((ticks as i128 * 1_000_000) / freq as i128).unwrap_or(u64::MAX)
    }
    #[cfg(not(windows))]
    {
        use std::time::Instant;
        static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
        START.elapsed().as_micros() as u64
    }
}
