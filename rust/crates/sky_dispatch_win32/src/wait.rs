//! Interruptible wait strategy with command-event priority.

use crate::clock::{DurationTicks, QpcClock, QpcTicks};
use crate::event::OwnedEvent;
use crate::timer::{TimerResolutionGuard, WaitableTimer};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitFailure {
    Clock,
    TimerCreate { win32_error: u32 },
    TimerArm { win32_error: u32 },
    TimerWait { win32_error: u32 },
    MultiWait { win32_error: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitOutcome {
    Deadline,
    Interrupted,
    Failed(WaitFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitResult {
    pub outcome: WaitOutcome,
    pub spin_us: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WakeErrorStats {
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    /// Robust periodic-reprobe estimate: median + 6 * MAD.
    ///
    /// This is separate from p95/p99 telemetry so a small periodic sample
    /// cannot let one scheduler outlier expand the spin window to its cap.
    pub robust_us: u64,
}

pub struct HybridWaiter {
    timer: Option<WaitableTimer>,
    _timer_resolution: Option<TimerResolutionGuard>,
    event_wait_enabled: bool,
    initial_failure: Option<WaitFailure>,
}

impl HybridWaiter {
    pub fn new() -> Self {
        Self::with_options(true, true)
    }

    pub fn with_options(waitable_timer_enabled: bool, event_wait_enabled: bool) -> Self {
        let (timer, initial_failure) = if waitable_timer_enabled {
            match WaitableTimer::new_with_error() {
                Ok(timer) => (Some(timer), None),
                Err(win32_error) => (None, Some(WaitFailure::TimerCreate { win32_error })),
            }
        } else {
            (None, None)
        };
        let timer_resolution = timer
            .is_none()
            .then(TimerResolutionGuard::acquire_1ms)
            .flatten();
        Self {
            timer,
            _timer_resolution: timer_resolution,
            event_wait_enabled,
            initial_failure,
        }
    }

    pub fn initial_failure(&self) -> Option<WaitFailure> {
        self.initial_failure
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
        let qpc_clock = match QpcClock::initialize() {
            Ok(clock) => clock,
            Err(_) => {
                return WaitResult {
                    outcome: WaitOutcome::Failed(WaitFailure::Clock),
                    spin_us: 0,
                };
            }
        };
        let now_ticks = match qpc_clock.now() {
            Ok(ticks) => ticks,
            Err(_) => {
                return WaitResult {
                    outcome: WaitOutcome::Failed(WaitFailure::Clock),
                    spin_us: 0,
                };
            }
        };
        let now_us = match qpc_clock.duration_to_us(DurationTicks::from_raw(now_ticks.as_u64())) {
            Ok(value) => value,
            Err(_) => {
                return WaitResult {
                    outcome: WaitOutcome::Failed(WaitFailure::Clock),
                    spin_us: 0,
                };
            }
        };
        let delta_ticks = match target_us.checked_sub(now_us) {
            Some(delta_us) => match qpc_clock.duration_from_us(delta_us) {
                Ok(value) => value,
                Err(_) => {
                    return WaitResult {
                        outcome: WaitOutcome::Failed(WaitFailure::Clock),
                        spin_us: 0,
                    };
                }
            },
            None => DurationTicks::ZERO,
        };
        let target_ticks = match now_ticks.checked_add_duration(delta_ticks) {
            Ok(ticks) => ticks,
            Err(_) => {
                return WaitResult {
                    outcome: WaitOutcome::Failed(WaitFailure::Clock),
                    spin_us: 0,
                };
            }
        };
        self.wait_until_ticks_with_metrics_typed(
            qpc_clock,
            target_ticks,
            match qpc_clock.duration_from_us(spin_threshold_us) {
                Ok(value) => value,
                Err(_) => {
                    return WaitResult {
                        outcome: WaitOutcome::Failed(WaitFailure::Clock),
                        spin_us: 0,
                    };
                }
            },
            interrupt,
        )
    }

    pub fn wait_until_ticks_with_metrics(
        &self,
        target_ticks: QpcTicks,
        spin_threshold_us: u64,
        interrupt: &OwnedEvent,
    ) -> WaitResult {
        let qpc_clock = match QpcClock::initialize() {
            Ok(clock) => clock,
            Err(_) => {
                return WaitResult {
                    outcome: WaitOutcome::Failed(WaitFailure::Clock),
                    spin_us: 0,
                };
            }
        };
        let spin_threshold_ticks = match qpc_clock.duration_from_us(spin_threshold_us) {
            Ok(value) => value,
            Err(_) => {
                return WaitResult {
                    outcome: WaitOutcome::Failed(WaitFailure::Clock),
                    spin_us: 0,
                };
            }
        };
        self.wait_until_ticks_with_metrics_typed(
            qpc_clock,
            target_ticks,
            spin_threshold_ticks,
            interrupt,
        )
    }

    /// Production tick-domain wait. Configuration is converted once by the
    /// caller; this method never rebuilds a deadline through microseconds.
    pub fn wait_until_ticks_with_metrics_typed(
        &self,
        qpc_clock: QpcClock,
        target_ticks: QpcTicks,
        spin_threshold_ticks: DurationTicks,
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

            let now_ticks = match qpc_clock.now() {
                Ok(ticks) => ticks,
                Err(_) => {
                    return WaitResult {
                        outcome: WaitOutcome::Failed(WaitFailure::Clock),
                        spin_us: 0,
                    };
                }
            };
            if now_ticks >= target_ticks {
                let now_us =
                    match qpc_clock.duration_to_us(DurationTicks::from_raw(now_ticks.as_u64())) {
                        Ok(value) => value,
                        Err(_) => {
                            return WaitResult {
                                outcome: WaitOutcome::Failed(WaitFailure::Clock),
                                spin_us: 0,
                            };
                        }
                    };
                return WaitResult {
                    outcome: WaitOutcome::Deadline,
                    spin_us: spin_started_us.map_or(0, |started| now_us.saturating_sub(started)),
                };
            }
            let remaining_ticks = match target_ticks.as_u64().checked_sub(now_ticks.as_u64()) {
                Some(remaining) => remaining,
                None => {
                    return WaitResult {
                        outcome: WaitOutcome::Failed(WaitFailure::Clock),
                        spin_us: 0,
                    };
                }
            };
            if remaining_ticks <= spin_threshold_ticks.as_u64() {
                let now_us =
                    match qpc_clock.duration_to_us(DurationTicks::from_raw(now_ticks.as_u64())) {
                        Ok(value) => value,
                        Err(_) => {
                            return WaitResult {
                                outcome: WaitOutcome::Failed(WaitFailure::Clock),
                                spin_us: 0,
                            };
                        }
                    };
                spin_started_us.get_or_insert(now_us);
                std::hint::spin_loop();
                continue;
            }

            let kernel_wait_ticks = match remaining_ticks.checked_sub(spin_threshold_ticks.as_u64())
            {
                Some(value) => value,
                None => {
                    return WaitResult {
                        outcome: WaitOutcome::Failed(WaitFailure::Clock),
                        spin_us: 0,
                    };
                }
            };
            let kernel_wait_us =
                match qpc_clock.duration_to_us(DurationTicks::from_raw(kernel_wait_ticks)) {
                    Ok(value) => value,
                    Err(_) => {
                        return WaitResult {
                            outcome: WaitOutcome::Failed(WaitFailure::Clock),
                            spin_us: 0,
                        };
                    }
                };
            #[cfg(windows)]
            if self.event_wait_enabled {
                if let Some(timer) = &self.timer {
                    let arm_result = timer.arm_relative_us(kernel_wait_us);
                    if let Err(error) = arm_result {
                        return WaitResult {
                            outcome: WaitOutcome::Failed(WaitFailure::TimerArm {
                                win32_error: error,
                            }),
                            spin_us: 0,
                        };
                    }

                    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
                    use windows_sys::Win32::System::Threading::WaitForMultipleObjects;

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
                    return WaitResult {
                        outcome: WaitOutcome::Failed(WaitFailure::MultiWait {
                            win32_error: unsafe { windows_sys::Win32::Foundation::GetLastError() },
                        }),
                        spin_us: 0,
                    };
                }
            } else if let Some(timer) = &self.timer
            // `sleep_us` arms and waits once. Do not arm the same timer
            // above and then re-arm it to a 1 ms cap: that turns every
            // long gap into a polling loop and distorts wake metrics.
            {
                match timer.sleep_us(kernel_wait_us) {
                    Ok(()) => continue,
                    Err(error) => {
                        return WaitResult {
                            outcome: WaitOutcome::Failed(WaitFailure::TimerWait {
                                win32_error: error,
                            }),
                            spin_us: 0,
                        };
                    }
                }
            }

            // Portable/degraded fallback remains bounded so a command cannot
            // be hidden behind a long song gap.
            std::thread::sleep(std::time::Duration::from_micros(kernel_wait_us.min(2_000)));
        }
    }

    pub fn probe_wake_error_us(&self, interrupt: &OwnedEvent, samples: usize) -> Option<u64> {
        let qpc_clock = QpcClock::initialize().ok()?;
        self.probe_wake_error_stats(qpc_clock, interrupt, samples)
            .map(|stats| stats.p95_us)
    }

    pub fn probe_wake_error_stats(
        &self,
        qpc_clock: QpcClock,
        interrupt: &OwnedEvent,
        samples: usize,
    ) -> Option<WakeErrorStats> {
        if samples == 0 {
            return None;
        }
        let mut errors = Vec::with_capacity(samples);
        for _ in 0..samples {
            let target_ticks = match qpc_clock.now() {
                Ok(now) => now
                    .checked_add_duration(qpc_clock.duration_from_us(2_000).ok()?)
                    .ok()?,
                Err(_) => return None,
            };
            match self
                .wait_until_ticks_with_metrics_typed(
                    qpc_clock,
                    target_ticks,
                    DurationTicks::ZERO,
                    interrupt,
                )
                .outcome
            {
                WaitOutcome::Interrupted | WaitOutcome::Failed(_) => return None,
                WaitOutcome::Deadline => {}
            }
            let now_ticks = match qpc_clock.now() {
                Ok(ticks) => ticks,
                Err(_) => return None,
            };
            let elapsed = now_ticks.checked_duration_since(target_ticks).ok()?;
            errors.push(qpc_clock.duration_to_us(elapsed).ok()?);
        }
        errors.sort_unstable();
        let percentile = |numerator: usize| {
            let index = ((errors.len() * numerator).saturating_add(99) / 100)
                .saturating_sub(1)
                .min(errors.len() - 1);
            errors[index]
        };
        Some(WakeErrorStats {
            p50_us: percentile(50),
            p95_us: percentile(95),
            p99_us: percentile(99),
            max_us: *errors.last().unwrap_or(&0),
            robust_us: robust_wake_error_us(&errors),
        })
    }
}

fn median_sorted(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        values[middle - 1].saturating_add(values[middle]) / 2
    } else {
        values[middle]
    }
}

fn robust_wake_error_us(sorted_errors: &[u64]) -> u64 {
    let median = median_sorted(sorted_errors);
    let mut deviations: Vec<u64> = sorted_errors
        .iter()
        .map(|value| value.abs_diff(median))
        .collect();
    deviations.sort_unstable();
    median.saturating_add(median_sorted(&deviations).saturating_mul(6))
}

impl Default for HybridWaiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{HybridWaiter, WaitOutcome, robust_wake_error_us};
    use crate::clock::qpc_now_us;
    use crate::event::OwnedEvent;

    #[test]
    fn pre_signalled_event_interrupts_a_long_wait() {
        let event = OwnedEvent::new_auto_reset().expect("event");
        assert!(event.signal());
        let waiter = HybridWaiter::new();
        assert_eq!(
            waiter.wait_until_us(
                qpc_now_us().expect("test QPC clock") + 1_000_000,
                200,
                &event
            ),
            WaitOutcome::Interrupted
        );
    }

    #[test]
    fn disabled_waitable_timer_reports_explicit_fallback() {
        let waiter = HybridWaiter::with_options(false, true);
        assert_eq!(waiter.mode(), "event+timer_resolution_fallback");
    }

    #[test]
    fn robust_wake_error_ignores_one_periodic_outlier() {
        let mut errors = vec![300; 7];
        errors.push(1_500);
        errors.sort_unstable();
        assert_eq!(robust_wake_error_us(&errors), 300);
    }
}
