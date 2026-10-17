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
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) startup_ordering_hook: Option<Arc<super::super::config::StartupOrderingHook>>,
}

impl WorkerSchedulingGuards {
    pub(crate) fn acquire(priority_mode: PriorityMode) -> Self {
        Self {
            priority: MmcssGuard::acquire(priority_mode),
            power: PowerThrottlingGuard::disable_current_thread(),
            #[cfg(any(test, feature = "test-support"))]
            drop_probe: None,
            #[cfg(any(test, feature = "test-support"))]
            startup_ordering_hook: None,
        }
    }

    #[allow(dead_code)]
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
impl WorkerSchedulingGuards {
    pub(crate) fn create_test_guards() -> Self {
        Self {
            priority: MmcssGuard::acquire(PriorityMode::Off),
            power: PowerThrottlingGuard::disable_current_thread(),
            drop_probe: None,
            startup_ordering_hook: None,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for WorkerSchedulingGuards {
    fn drop(&mut self) {
        if let Some(probe) = &self.drop_probe {
            probe.fetch_add(1, Ordering::SeqCst);
        }
        if let Some(hook) = &self.startup_ordering_hook {
            hook.record_scheduling_guard_dropped();
        }
    }
}

pub(super) struct StartupResources {
    pub(super) waiter: HybridWaiter,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn initialize_startup(
    priority_mode: PriorityMode,
    production_wait: bool,
    enable_waitable_timer: bool,
    enable_event_wait: bool,
    requested_wait_policy: &'static str,
    effective_wait_policy: &'static str,
    priority_acquired: &Mutex<String>,
    metrics: &SharedMetrics,
) -> StartupResources {
    *priority_acquired.lock() = "pending".to_string();
    *metrics.requested_priority_mode.lock() = priority_mode.requested_label().to_string();
    *metrics.requested_wait_policy.lock() = requested_wait_policy.to_string();
    *metrics.effective_wait_policy.lock() = effective_wait_policy.to_string();
    let waiter = if production_wait {
        HybridWaiter::production()
    } else {
        HybridWaiter::with_options(enable_waitable_timer, enable_event_wait)
    };
    *metrics.wait_strategy_acquired.lock() = waiter.mode().to_string();
    StartupResources { waiter }
}
