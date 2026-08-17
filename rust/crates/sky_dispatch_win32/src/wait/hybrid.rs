use super::{
    WaitFailure, WaitOutcome, WaitResult, WakeErrorStats,
    calibration::robust_wake_error_us,
    spin::{deadline_wait_result, wait_result_with_spin},
};
use crate::clock::{DurationTicks, QpcClock, QpcTicks};
use crate::event::OwnedEvent;
use crate::timer::{TimerResolutionGuard, WaitableTimer};

pub struct HybridWaiter {
    timer: Option<WaitableTimer>,
    _timer_resolution: Option<TimerResolutionGuard>,
    event_wait_enabled: bool,
    initial_failure: Option<WaitFailure>,
}

impl HybridWaiter {
    /// Construct the production waiter.  Production has no timer-resolution
    /// fallback: failure to create the high-resolution waitable timer is a
    /// startup error, never a reason to enter coarse sleep polling.
    pub fn production() -> Self {
        let (timer, initial_failure) = match WaitableTimer::new_with_error() {
            Ok(timer) => (Some(timer), None),
            Err(win32_error) => (None, Some(WaitFailure::TimerCreate { win32_error })),
        };
        Self {
            timer,
            _timer_resolution: None,
            event_wait_enabled: true,
            initial_failure,
        }
    }

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
                return WaitResult::failed(WaitFailure::Clock);
            }
        };
        let now_ticks = match qpc_clock.now() {
            Ok(ticks) => ticks,
            Err(_) => {
                return WaitResult::failed(WaitFailure::Clock);
            }
        };
        let now_us = match qpc_clock.duration_to_us(DurationTicks::from_raw(now_ticks.as_u64())) {
            Ok(value) => value,
            Err(_) => {
                return WaitResult::failed(WaitFailure::Clock);
            }
        };
        let delta_ticks = match target_us.checked_sub(now_us) {
            Some(delta_us) => match qpc_clock.duration_from_us(delta_us) {
                Ok(value) => value,
                Err(_) => {
                    return WaitResult::failed(WaitFailure::Clock);
                }
            },
            None => DurationTicks::ZERO,
        };
        let target_ticks = match now_ticks.checked_add_duration(delta_ticks) {
            Ok(ticks) => ticks,
            Err(_) => {
                return WaitResult::failed(WaitFailure::Clock);
            }
        };
        self.wait_until_ticks_with_metrics_typed(
            qpc_clock,
            target_ticks,
            match qpc_clock.duration_from_us(spin_threshold_us) {
                Ok(value) => value,
                Err(_) => {
                    return WaitResult::failed(WaitFailure::Clock);
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
                return WaitResult::failed(WaitFailure::Clock);
            }
        };
        let spin_threshold_ticks = match qpc_clock.duration_from_us(spin_threshold_us) {
            Ok(value) => value,
            Err(_) => {
                return WaitResult::failed(WaitFailure::Clock);
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
        // Interrupts wake/replan only while the physical target is still in
        // the future. Once QPC reaches the target, this waiter returns
        // Deadline; final control/target/focus/lease admission decides
        // whether transport is still allowed.
        let mut spin_started_ticks = None;
        let mut observed_generation = interrupt.signal_generation();
        if self.event_wait_enabled {
            if interrupt.try_take() {
                return classify_interrupt_with_fresh_qpc(qpc_clock, None, target_ticks, false);
            }
            // Close the handoff race between the first event consume and the
            // generation sample. A signal after this second consume is seen
            // by the generation check in the final spin.
            let after_take = interrupt.signal_generation();
            if after_take != observed_generation {
                if interrupt.try_take() {
                    return classify_interrupt_with_fresh_qpc(qpc_clock, None, target_ticks, false);
                }
                observed_generation = interrupt.signal_generation();
            }
        }
        loop {
            let now_ticks = match qpc_clock.now() {
                Ok(ticks) => ticks,
                Err(_) => {
                    return WaitResult::failed(WaitFailure::Clock);
                }
            };
            if now_ticks >= target_ticks {
                return deadline_wait_result(spin_started_ticks, now_ticks);
            }
            let remaining_ticks = match target_ticks.as_u64().checked_sub(now_ticks.as_u64()) {
                Some(remaining) => remaining,
                None => {
                    return WaitResult::failed(WaitFailure::Clock);
                }
            };
            if remaining_ticks <= spin_threshold_ticks.as_u64() {
                let mut spin_iterations = 0_u32;
                loop {
                    let now_ticks = match qpc_clock.now() {
                        Ok(ticks) => ticks,
                        Err(_) => {
                            return WaitResult::failed(WaitFailure::Clock);
                        }
                    };
                    if now_ticks >= target_ticks {
                        return deadline_wait_result(spin_started_ticks, now_ticks);
                    }
                    if self.event_wait_enabled {
                        if spin_iterations & 31 == 0
                            && interrupt.signal_generation_relaxed() != observed_generation
                        {
                            return classify_interrupt_with_fresh_qpc(
                                qpc_clock,
                                spin_started_ticks,
                                target_ticks,
                                true,
                            );
                        }
                        spin_iterations = spin_iterations.wrapping_add(1);
                    }
                    let remaining_ticks =
                        match target_ticks.as_u64().checked_sub(now_ticks.as_u64()) {
                            Some(remaining) => remaining,
                            None => {
                                return WaitResult::failed(WaitFailure::Clock);
                            }
                        };
                    if remaining_ticks > spin_threshold_ticks.as_u64() {
                        break;
                    }
                    spin_started_ticks.get_or_insert(now_ticks);
                    std::hint::spin_loop();
                }
                continue;
            }

            let kernel_wait_ticks = match remaining_ticks.checked_sub(spin_threshold_ticks.as_u64())
            {
                Some(value) => value,
                None => {
                    return WaitResult::failed(WaitFailure::Clock);
                }
            };
            let kernel_wait_us =
                match qpc_clock.duration_to_us(DurationTicks::from_raw(kernel_wait_ticks)) {
                    Ok(value) => value,
                    Err(_) => {
                        return WaitResult::failed(WaitFailure::Clock);
                    }
                };
            #[cfg(windows)]
            if self.event_wait_enabled {
                if let Some(timer) = &self.timer {
                    let arm_result = timer.arm_relative_us(kernel_wait_us);
                    if let Err(error) = arm_result {
                        return WaitResult::failed(WaitFailure::TimerArm { win32_error: error });
                    }

                    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
                    use windows_sys::Win32::System::Threading::WaitForMultipleObjects;

                    // The timer wakes the worker at the precision handoff;
                    // final QPC/control admission still decides whether to
                    // dispatch when both handles are ready.
                    let handles = [timer.raw_handle(), interrupt.raw_handle()];
                    // SAFETY: both handles remain live throughout the wait and
                    // the slice provides exactly two valid HANDLE values.
                    let result =
                        unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, u32::MAX) };
                    if result == WAIT_OBJECT_0 {
                        continue;
                    }
                    if result == WAIT_OBJECT_0 + 1 {
                        return classify_interrupt_with_fresh_qpc(
                            qpc_clock,
                            spin_started_ticks,
                            target_ticks,
                            false,
                        );
                    }
                    return WaitResult::failed(WaitFailure::MultiWait {
                        win32_error: unsafe { windows_sys::Win32::Foundation::GetLastError() },
                    });
                }
            } else if let Some(timer) = &self.timer
            // `sleep_us` arms and waits once. Do not arm the same timer
            // above and then re-arm it to a 1 ms cap: that turns every
            // long gap into a polling loop and distorts wake metrics.
            {
                match timer.sleep_us(kernel_wait_us) {
                    Ok(()) => continue,
                    Err(error) => {
                        return WaitResult::failed(WaitFailure::TimerWait { win32_error: error });
                    }
                }
            }

            // Portable/degraded fallback remains bounded so a command cannot
            // be hidden behind a long song gap.
            std::thread::sleep(std::time::Duration::from_micros(kernel_wait_us.min(2_000)));
            let wake_ticks = match qpc_clock.now() {
                Ok(ticks) => ticks,
                Err(_) => {
                    return WaitResult::failed(WaitFailure::Clock);
                }
            };
            if wake_ticks >= target_ticks {
                return deadline_wait_result(spin_started_ticks, wake_ticks);
            }
            if self.event_wait_enabled && interrupt.signal_generation() != observed_generation {
                if interrupt.try_take() {
                    return classify_interrupt_with_fresh_qpc(
                        qpc_clock,
                        spin_started_ticks,
                        target_ticks,
                        false,
                    );
                }
                observed_generation = interrupt.signal_generation();
            }
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
        Some(WakeErrorStats {
            p50_us: percentile_from_sorted(&errors, 50),
            p95_us: percentile_from_sorted(&errors, 95),
            p99_us: percentile_from_sorted(&errors, 99),
            max_us: *errors.last().unwrap_or(&0),
            robust_us: robust_wake_error_us(&errors),
        })
    }
}

fn classify_interrupt_with_fresh_qpc(
    qpc_clock: QpcClock,
    spin_started_ticks: Option<QpcTicks>,
    target_ticks: QpcTicks,
    include_spin_evidence: bool,
) -> WaitResult {
    let completed_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(_) => {
            return WaitResult::failed(WaitFailure::Clock);
        }
    };
    classify_interrupt_after_refresh(
        spin_started_ticks,
        target_ticks,
        completed_ticks,
        include_spin_evidence,
    )
}

fn classify_interrupt_after_refresh(
    spin_started_ticks: Option<QpcTicks>,
    target_ticks: QpcTicks,
    completed_ticks: QpcTicks,
    include_spin_evidence: bool,
) -> WaitResult {
    match classify_wake(completed_ticks, target_ticks) {
        WaitOutcome::Deadline => deadline_wait_result(spin_started_ticks, completed_ticks),
        WaitOutcome::Interrupted if include_spin_evidence => wait_result_with_spin(
            WaitOutcome::Interrupted,
            spin_started_ticks,
            completed_ticks,
        ),
        WaitOutcome::Interrupted => WaitResult::interrupted(),
        WaitOutcome::Failed(failure) => WaitResult::failed(failure),
    }
}

fn classify_wake(now_ticks: QpcTicks, target_ticks: QpcTicks) -> WaitOutcome {
    if now_ticks >= target_ticks {
        WaitOutcome::Deadline
    } else {
        WaitOutcome::Interrupted
    }
}

fn percentile_from_sorted(sorted: &[u64], numerator: usize) -> u64 {
    let index = ((sorted.len() * numerator).saturating_add(99) / 100)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index]
}

impl Default for HybridWaiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_interrupt_after_refresh, classify_wake, percentile_from_sorted};
    use crate::clock::QpcTicks;
    use crate::wait::WaitOutcome;

    #[test]
    fn thirty_two_sample_p95_does_not_promote_one_extreme_maximum() {
        let mut samples = vec![300_u64; 31];
        samples.push(50_000);
        samples.sort_unstable();

        assert_eq!(samples.len(), 32);
        assert_eq!(percentile_from_sorted(&samples, 95), 300);
        assert_eq!(*samples.last().unwrap(), 50_000);
    }

    #[test]
    fn interrupt_before_target_is_classified_as_interrupted() {
        assert_eq!(
            classify_wake(QpcTicks::from_raw(10), QpcTicks::from_raw(11)),
            WaitOutcome::Interrupted
        );
    }

    #[test]
    fn interrupt_at_target_is_classified_as_deadline() {
        assert_eq!(
            classify_wake(QpcTicks::from_raw(11), QpcTicks::from_raw(11)),
            WaitOutcome::Deadline
        );
    }

    #[test]
    fn interrupt_classification_uses_fresh_qpc_after_stale_precheck() {
        let stale_ticks = QpcTicks::from_raw(10);
        let target_ticks = QpcTicks::from_raw(11);
        let refreshed_ticks = QpcTicks::from_raw(12);

        assert_eq!(
            classify_wake(stale_ticks, target_ticks),
            WaitOutcome::Interrupted
        );
        let result = classify_interrupt_after_refresh(None, target_ticks, refreshed_ticks, false);

        assert_eq!(result.outcome, WaitOutcome::Deadline);
        assert_eq!(result.wake_qpc, Some(refreshed_ticks));
    }

    #[test]
    fn final_spin_interrupt_classification_uses_fresh_qpc_for_spin_evidence() {
        let stale_ticks = QpcTicks::from_raw(10);
        let target_ticks = QpcTicks::from_raw(11);
        let refreshed_ticks = QpcTicks::from_raw(10);

        assert_eq!(
            classify_wake(stale_ticks, target_ticks),
            WaitOutcome::Interrupted
        );
        let result = classify_interrupt_after_refresh(
            Some(QpcTicks::from_raw(9)),
            target_ticks,
            refreshed_ticks,
            true,
        );

        assert_eq!(result.outcome, WaitOutcome::Interrupted);
        assert_eq!(result.wake_qpc, Some(refreshed_ticks));
        assert_eq!(result.spin_ticks, crate::clock::DurationTicks::from_raw(1));
    }
}
