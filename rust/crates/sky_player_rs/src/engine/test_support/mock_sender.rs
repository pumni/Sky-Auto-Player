use sky_dispatch_win32::clock::{QpcClock, QpcTicks, QpcError};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::input::PlatformSendResult;

pub(crate) fn mock_platform_send_result_from_started_ticks(
    qpc_clock: QpcClock,
    started_ticks: Result<QpcTicks, QpcError>,
    requested: u32,
    inserted: u32,
    win32_error: u32,
    latency_ticks: u64,
) -> PlatformSendResult {
    let started_ticks = match started_ticks {
        Ok(ticks) => ticks,
        Err(error) => {
            return PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: None,
                completed_us: 0,
                win32_error,
                timing_error: Some(error),
            };
        }
    };
    let deadline = match started_ticks.checked_add_duration(DurationTicks::from_raw(latency_ticks))
    {
        Ok(deadline) => deadline,
        Err(_) => {
            return PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks,
                completed_ticks: None,
                completed_us: 0,
                win32_error,
                timing_error: Some(QpcError::DeadlineOverflow),
            };
        }
    };
    loop {
        match qpc_clock.now() {
            Ok(now) if now >= deadline => {
                let (completed_us, timing_error) =
                    match qpc_clock.duration_to_us(DurationTicks::from_raw(now.as_u64())) {
                        Ok(micros) => (micros, None),
                        Err(_) => (0, Some(QpcError::ConversionOverflow)),
                    };
                return PlatformSendResult {
                    requested,
                    inserted,
                    started_ticks,
                    completed_ticks: Some(now),
                    completed_us,
                    win32_error,
                    timing_error,
                };
            }
            Ok(_) => std::hint::spin_loop(),
            Err(error) => {
                return PlatformSendResult {
                    requested,
                    inserted: 0,
                    started_ticks,
                    completed_ticks: None,
                    completed_us: 0,
                    win32_error,
                    timing_error: Some(error),
                };
            }
        }
    }
}
