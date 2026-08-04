use crate::engine::telemetry::SharedMetrics;
use parking_lot::Mutex;
use sky_dispatch_win32::mmcss::{MmcssGuard, PriorityMode};
use sky_dispatch_win32::power::PowerThrottlingGuard;
use sky_dispatch_win32::wait::HybridWaiter;

pub(super) struct StartupResources {
    pub(super) power_guard: PowerThrottlingGuard,
    pub(super) priority_guard: MmcssGuard,
    pub(super) waiter: HybridWaiter,
    pub(super) power_throttling_disabled: bool,
}

pub(super) fn initialize_startup(
    priority_mode: PriorityMode,
    enable_waitable_timer: bool,
    enable_event_wait: bool,
    priority_acquired: &Mutex<String>,
    metrics: &SharedMetrics,
) -> StartupResources {
    let power_guard = PowerThrottlingGuard::disable_current_thread();
    let power_throttling_disabled = power_guard.is_active();
    let priority_guard = MmcssGuard::acquire(priority_mode);
    *priority_acquired.lock() = priority_guard.acquired().to_string();
    let waiter = HybridWaiter::with_options(enable_waitable_timer, enable_event_wait);
    *metrics.wait_strategy_acquired.lock() = waiter.mode().to_string();
    StartupResources {
        power_guard,
        priority_guard,
        waiter,
        power_throttling_disabled,
    }
}
