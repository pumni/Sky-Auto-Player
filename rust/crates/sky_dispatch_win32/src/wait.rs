//! Interruptible wait strategy with command-event priority.

use crate::clock::qpc_now_us;
use crate::event::OwnedEvent;
use crate::timer::{TimerResolutionGuard, WaitableTimer};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitOutcome {
    Deadline,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitResult {
    pub outcome: WaitOutcome,
    pub spin_us: u64,
}

pub struct HybridWaiter {
    timer: Option<WaitableTimer>,
    _timer_resolution: Option<TimerResolutionGuard>,
    event_wait_enabled: bool,
}

impl HybridWaiter {
    pub fn new() -> Self {
        Self::with_options(true, true)
    }

    pub fn with_options(waitable_timer_enabled: bool, event_wait_enabled: bool) -> Self {
        let timer = waitable_timer_enabled.then(WaitableTimer::new).flatten();
        let timer_resolution = timer
            .is_none()
            .then(TimerResolutionGuard::acquire_1ms)
            .flatten();
        Self {
            timer,
            _timer_resolution: timer_resolution,
            event_wait_enabled,
        }
    }

    pub fn mode(&self) -> &'static str {
        match (self.timer.is_some(), self.event_wait_enabled) {
            (true, true) => "event+high_resolution_timer",
            (true, false) => "high_resolution_timer",
            (false, true) => "event+timer_resolution_fallback",
            (false, false) => "timer_resolution_fallback",
        }
    }

    pub fn wait_until_us(
        &self,
        target_us: u64,
        spin_threshold_us: u64,
        interrupt: &OwnedEvent,
    ) -> WaitOutcome {
        self.wait_until_us_with_metrics(target_us, spin_threshold_us, interrupt)
            .outcome
    }

    pub fn wait_until_us_with_metrics(
        &self,
        target_us: u64,
        spin_threshold_us: u64,
        interrupt: &OwnedEvent,
    ) -> WaitResult {
        let mut spin_started_us = None;
        loop {
            if self.event_wait_enabled && interrupt.try_take() {
                return WaitResult {
                    outcome: WaitOutcome::Interrupted,
                    spin_us: 0,
                };
            }

            let now_us = qpc_now_us();
            if now_us >= target_us {
                return WaitResult {
                    outcome: WaitOutcome::Deadline,
                    spin_us: spin_started_us.map_or(0, |started| now_us.saturating_sub(started)),
                };
            }
            let remaining_us = target_us - now_us;
            if remaining_us <= spin_threshold_us {
                spin_started_us.get_or_insert(now_us);
                std::hint::spin_loop();
                continue;
            }

            let kernel_wait_us = remaining_us - spin_threshold_us;
            #[cfg(windows)]
            if let Some(timer) = &self.timer
                && timer.arm_relative_us(kernel_wait_us)
            {
                use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
                use windows_sys::Win32::System::Threading::WaitForMultipleObjects;

                if self.event_wait_enabled {
                    // Event is deliberately index 0 so simultaneous readiness
                    // gives command processing priority over dispatch.
                    let handles = [interrupt.raw_handle(), timer.raw_handle()];
                    // SAFETY: both handles remain live throughout the wait and
                    // the slice provides exactly two valid HANDLE values.
                    let result =
                        unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, u32::MAX) };
                    if result == WAIT_OBJECT_0 {
                        return WaitResult {
                            outcome: WaitOutcome::Interrupted,
                            spin_us: 0,
                        };
                    }
                    if result == WAIT_OBJECT_0 + 1 {
                        continue;
                    }
                } else if timer.sleep_us(kernel_wait_us.min(1_000)) {
                    continue;
                }
            }

            // Portable/degraded fallback remains bounded so a command cannot
            // be hidden behind a long song gap.
            std::thread::sleep(std::time::Duration::from_micros(kernel_wait_us.min(2_000)));
        }
    }

    pub fn probe_wake_error_us(&self, interrupt: &OwnedEvent, samples: usize) -> Option<u64> {
        let mut max_error_us = 0;
        for _ in 0..samples {
            let target_us = qpc_now_us().saturating_add(2_000);
            if self.wait_until_us(target_us, 0, interrupt) == WaitOutcome::Interrupted {
                return None;
            }
            max_error_us = max_error_us.max(qpc_now_us().saturating_sub(target_us));
        }
        Some(max_error_us)
    }
}

impl Default for HybridWaiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{HybridWaiter, WaitOutcome};
    use crate::clock::qpc_now_us;
    use crate::event::OwnedEvent;

    #[test]
    fn pre_signalled_event_interrupts_a_long_wait() {
        let event = OwnedEvent::new_auto_reset().expect("event");
        assert!(event.signal());
        let waiter = HybridWaiter::new();
        assert_eq!(
            waiter.wait_until_us(qpc_now_us() + 1_000_000, 200, &event),
            WaitOutcome::Interrupted
        );
    }

    #[test]
    fn disabled_waitable_timer_reports_explicit_fallback() {
        let waiter = HybridWaiter::with_options(false, true);
        assert_eq!(waiter.mode(), "event+timer_resolution_fallback");
    }
}
