use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes,
};

/// Returns the CPU time in microseconds for the current thread.
/// Includes both user and kernel time.
/// Returns 0 if the query fails.
pub fn current_thread_cpu_time_us() -> u64 {
    let mut creation_time = unsafe { std::mem::zeroed() };
    let mut exit_time = unsafe { std::mem::zeroed() };
    let mut kernel_time = unsafe { std::mem::zeroed() };
    let mut user_time = unsafe { std::mem::zeroed() };

    let success = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    };

    if success == 0 {
        return 0;
    }

    let kernel = ((kernel_time.dwHighDateTime as u64) << 32) | (kernel_time.dwLowDateTime as u64);
    let user = ((user_time.dwHighDateTime as u64) << 32) | (user_time.dwLowDateTime as u64);

    // time is in 100-nanosecond intervals. Divide by 10 to get microseconds.
    (kernel + user) / 10
}

/// Returns the CPU time in microseconds for the current process.
/// Includes both user and kernel time across all threads.
/// Returns 0 if the query fails.
pub fn current_process_cpu_time_us() -> u64 {
    let mut creation_time = unsafe { std::mem::zeroed() };
    let mut exit_time = unsafe { std::mem::zeroed() };
    let mut kernel_time = unsafe { std::mem::zeroed() };
    let mut user_time = unsafe { std::mem::zeroed() };

    let success = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    };

    if success == 0 {
        return 0;
    }

    let kernel = ((kernel_time.dwHighDateTime as u64) << 32) | (kernel_time.dwLowDateTime as u64);
    let user = ((user_time.dwHighDateTime as u64) << 32) | (user_time.dwLowDateTime as u64);

    // time is in 100-nanosecond intervals. Divide by 10 to get microseconds.
    (kernel + user) / 10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_cpu_time_increases() {
        let start = current_thread_cpu_time_us();

        // spin a bit to burn CPU
        let mut sum: u64 = 0;
        for i in 0..1_000_000 {
            sum = sum.wrapping_add(i);
        }
        std::hint::black_box(sum);

        let end = current_thread_cpu_time_us();
        assert!(end >= start, "CPU time should be monotonically increasing");

        let process_start = current_process_cpu_time_us();
        assert!(
            process_start >= end,
            "Process time should be >= thread time"
        );
    }
}
