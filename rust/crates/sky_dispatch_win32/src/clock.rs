//! Monotonic QueryPerformanceCounter clock query helper.

pub fn qpc_frequency() -> u64 {
    #[cfg(windows)]
    {
        let mut freq: i64 = 0;
        unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut freq);
        }
        freq as u64
    }
    #[cfg(not(windows))]
    {
        1_000_000_000
    }
}

pub fn qpc_now_us() -> u64 {
    #[cfg(windows)]
    {
        let mut ticks: i64 = 0;
        unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceCounter(&mut ticks);
        }
        let freq = qpc_frequency();
        if freq == 0 {
            return 0;
        }
        ((ticks as i128 * 1_000_000) / freq as i128) as u64
    }
    #[cfg(not(windows))]
    {
        use std::time::Instant;
        static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
        START.elapsed().as_micros() as u64
    }
}
