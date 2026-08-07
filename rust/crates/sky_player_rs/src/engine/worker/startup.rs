use crate::engine::telemetry::SharedMetrics;
use parking_lot::Mutex;
use sky_dispatch_win32::mmcss::{MmcssGuard, PriorityMode};
use sky_dispatch_win32::power::PowerThrottlingGuard;
use sky_dispatch_win32::wait::HybridWaiter;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
pub(crate) struct WorkerSchedulingGuards {
    pub(crate) priority: MmcssGuard,
    pub(crate) power: PowerThrottlingGuard,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) drop_probe: Option<Arc<AtomicUsize>>,
}

impl WorkerSchedulingGuards {
    pub(crate) fn is_priority_active(&self) -> bool {
        self.priority.is_active()
    }

    pub(crate) fn is_power_active(&self) -> bool {
        self.power.is_active()
    }

    pub(crate) fn priority_label(&self) -> &'static str {
        self.priority.acquired()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for WorkerSchedulingGuards {
    fn drop(&mut self) {
        if let Some(probe) = &self.drop_probe {
            probe.fetch_add(1, Ordering::SeqCst);
        }
    }
}

pub(super) struct StartupResources {
    pub(super) scheduling: WorkerSchedulingGuards,
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
    let power = PowerThrottlingGuard::disable_current_thread();
    let priority = MmcssGuard::acquire(priority_mode);
    let scheduling = WorkerSchedulingGuards {
        priority,
        power,
        #[cfg(any(test, feature = "test-support"))]
        drop_probe: None,
    };
    let power_throttling_disabled = scheduling.is_power_active();
    *priority_acquired.lock() = scheduling.priority_label().to_string();
    let waiter = HybridWaiter::with_options(enable_waitable_timer, enable_event_wait);
    *metrics.wait_strategy_acquired.lock() = waiter.mode().to_string();
    StartupResources {
        scheduling,
        waiter,
        power_throttling_disabled,
    }
}
