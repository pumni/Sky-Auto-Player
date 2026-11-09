use super::{
    WaitFailure, WaitOutcome, WaitResult, WakeErrorStats,
    calibration::robust_wake_error_us,
    spin::{deadline_wait_result, wait_result_with_spin},
};
use crate::clock::{DurationTicks, QpcClock, QpcTicks};
use crate::event::OwnedEvent;
use crate::timer::{TimerResolutionGuard, WaitableTimer};
use std::num::NonZeroU64;

pub struct HybridWaiter {
    timer: Option<WaitableTimer>,
    _timer_resolution: Option<TimerResolutionGuard>,
    event_wait_enabled: bool,
    initial_failure: Option<WaitFailure>,
    #[cfg(any(test, feature = "test-support"))]
    capture_timer_provenance: bool,
}

impl HybridWaiter {
    /// Construct the production waiter.  Production has no timer-resolution
    /// fallback: failure to create the high-resolution waitable timer is a
    /// startup error, never a reason to enter coarse sleep polling.
    pub fn production() -> Self {
        Self::production_with_timer_provenance(false)
    }

    pub fn production_with_timer_provenance(capture_timer_provenance: bool) -> Self {
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = capture_timer_provenance;
        let (timer, initial_failure) = match WaitableTimer::new_with_error() {
            Ok(timer) => (Some(timer), None),
            Err(win32_error) => (None, Some(WaitFailure::TimerCreate { win32_error })),
        };
        Self {
            timer,
            _timer_resolution: None,
            event_wait_enabled: true,
            initial_failure,
            #[cfg(any(test, feature = "test-support"))]
            capture_timer_provenance,
        }
    }

    pub fn new() -> Self {
        Self::with_options(true, true)
    }

    pub fn with_options(waitable_timer_enabled: bool, event_wait_enabled: bool) -> Self {
        Self::with_options_and_timer_provenance(waitable_timer_enabled, event_wait_enabled, false)
    }

    pub fn with_options_and_timer_provenance(
        waitable_timer_enabled: bool,
        event_wait_enabled: bool,
        capture_timer_provenance: bool,
    ) -> Self {
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = capture_timer_provenance;
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
            #[cfg(any(test, feature = "test-support"))]
            capture_timer_provenance,
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
        #[cfg(any(test, feature = "test-support"))]
        let mut timer_return_pending_generation = None;
        #[cfg(any(test, feature = "test-support"))]
        let mut timer_generation = 0_u64;
        #[cfg(any(test, feature = "test-support"))]
        let mut timer_evidence = TimerEvidence::default();
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
            #[cfg(any(test, feature = "test-support"))]
            if let Some(returned_generation) = timer_return_pending_generation.take() {
                timer_evidence.timer_return_qpc = Some(now_ticks);
                timer_evidence.timer_return_generation = Some(returned_generation);
            }
            if now_ticks >= target_ticks {
                return deadline_wait_result_with_timer_evidence(
                    spin_started_ticks,
                    now_ticks,
                    #[cfg(any(test, feature = "test-support"))]
                    timer_evidence,
                );
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
                        return deadline_wait_result_with_timer_evidence(
                            spin_started_ticks,
                            now_ticks,
                            #[cfg(any(test, feature = "test-support"))]
                            timer_evidence,
                        );
                    }
                    if self.event_wait_enabled {
                        if spin_iterations & 31 == 0
                            && interrupt.signal_generation_relaxed() != observed_generation
                        {
                            return attach_timer_evidence(
                                classify_interrupt_with_fresh_qpc(
                                    qpc_clock,
                                    spin_started_ticks,
                                    target_ticks,
                                    true,
                                ),
                                #[cfg(any(test, feature = "test-support"))]
                                timer_evidence,
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

            let Some(kernel_wait_us) =
                (match classify_kernel_wait_us(qpc_clock, remaining_ticks, spin_threshold_ticks) {
                    Ok(value) => value,
                    Err(failure) => return WaitResult::failed(failure),
                })
            else {
                // A positive QPC remainder can floor to zero microseconds. A
                // zero delay cannot arm a waitable timer, so recheck QPC
                // directly instead of entering an infinite multi-wait.
                continue;
            };
            #[cfg(windows)]
            if self.event_wait_enabled {
                if let Some(timer) = &self.timer {
                    #[cfg(any(test, feature = "test-support"))]
                    let timer_arm_delta = self
                        .capture_timer_provenance
                        .then(|| qpc_clock.duration_from_us(kernel_wait_us.get()).ok())
                        .flatten();
                    #[cfg(any(test, feature = "test-support"))]
                    let arm_qpc = self
                        .capture_timer_provenance
                        .then(|| qpc_clock.now().ok())
                        .flatten();
                    #[cfg(any(test, feature = "test-support"))]
                    let arm_target_qpc = arm_qpc.and_then(|arm| {
                        timer_arm_delta.and_then(|delta| arm.checked_add_duration(delta).ok())
                    });
                    let arm_result = timer.arm_relative_us(kernel_wait_us);
                    if let Err(error) = arm_result {
                        return WaitResult::failed(WaitFailure::TimerArm { win32_error: error });
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    let armed_generation = if self.capture_timer_provenance {
                        timer_generation.checked_add(1).inspect(|&generation| {
                            timer_generation = generation;
                            timer_evidence = TimerEvidence {
                                timer_arm_qpc: arm_qpc,
                                timer_arm_target_qpc: arm_target_qpc,
                                timer_arm_generation: Some(generation),
                                timer_return_qpc: None,
                                timer_return_generation: None,
                            };
                        })
                    } else {
                        None
                    };

                    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
                    use windows_sys::Win32::System::Threading::WaitForMultipleObjects;

                    // The timer wakes the worker near the authored target;
                    // final QPC/control admission still decides whether to
                    // dispatch when both handles are ready.
                    let handles = [timer.raw_handle(), interrupt.raw_handle()];
                    // SAFETY: both handles remain live throughout the wait and
                    // the slice provides exactly two valid HANDLE values.
                    let result =
                        unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, u32::MAX) };
                    if result == WAIT_OBJECT_0 {
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            timer_return_pending_generation = armed_generation;
                        }
                        continue;
                    }
                    if result == WAIT_OBJECT_0 + 1 {
                        return attach_timer_evidence(
                            classify_interrupt_with_fresh_qpc(
                                qpc_clock,
                                spin_started_ticks,
                                target_ticks,
                                false,
                            ),
                            #[cfg(any(test, feature = "test-support"))]
                            timer_evidence,
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
                match timer.sleep_us(kernel_wait_us.get()) {
                    Ok(()) => {
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            timer_return_pending_generation = None;
                        }
                        continue;
                    }
                    Err(error) => {
                        return WaitResult::failed(WaitFailure::TimerWait { win32_error: error });
                    }
                }
            }

            // Portable/degraded fallback remains bounded so a command cannot
            // be hidden behind a long song gap.
            std::thread::sleep(std::time::Duration::from_micros(
                kernel_wait_us.get().min(2_000),
            ));
            let wake_ticks = match qpc_clock.now() {
                Ok(ticks) => ticks,
                Err(_) => {
                    return WaitResult::failed(WaitFailure::Clock);
                }
            };
            if wake_ticks >= target_ticks {
                return deadline_wait_result_with_timer_evidence(
                    spin_started_ticks,
                    wake_ticks,
                    #[cfg(any(test, feature = "test-support"))]
                    timer_evidence,
                );
            }
            if self.event_wait_enabled && interrupt.signal_generation() != observed_generation {
                if interrupt.try_take() {
                    return attach_timer_evidence(
                        classify_interrupt_with_fresh_qpc(
                            qpc_clock,
                            spin_started_ticks,
                            target_ticks,
                            false,
                        ),
                        #[cfg(any(test, feature = "test-support"))]
                        timer_evidence,
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
        self.probe_wake_error_stats_until(qpc_clock, interrupt, samples, None)
    }

    /// Probe wake error while refusing to cross an optional startup deadline.
    ///
    /// The deadline is checked before arming every sample and after every
    /// wake. This lets optional startup calibration yield to readiness work
    /// instead of consuming the entire startup correctness budget.
    pub fn probe_wake_error_stats_until(
        &self,
        qpc_clock: QpcClock,
        interrupt: &OwnedEvent,
        samples: usize,
        deadline_ticks: Option<crate::clock::QpcTicks>,
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
            if deadline_ticks.is_some_and(|deadline| target_ticks > deadline) {
                return None;
            }
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
            if deadline_ticks.is_some_and(|deadline| now_ticks > deadline) {
                return None;
            }
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

fn classify_kernel_wait_us(
    qpc_clock: QpcClock,
    remaining_ticks: u64,
    spin_threshold_ticks: DurationTicks,
) -> Result<Option<NonZeroU64>, WaitFailure> {
    let kernel_wait_ticks = remaining_ticks
        .checked_sub(spin_threshold_ticks.as_u64())
        .ok_or(WaitFailure::Clock)?;
    let kernel_wait_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(kernel_wait_ticks))
        .map_err(|_| WaitFailure::Clock)?;
    Ok(NonZeroU64::new(kernel_wait_us))
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Default)]
struct TimerEvidence {
    timer_arm_qpc: Option<QpcTicks>,
    timer_arm_target_qpc: Option<QpcTicks>,
    timer_arm_generation: Option<u64>,
    timer_return_qpc: Option<QpcTicks>,
    timer_return_generation: Option<u64>,
}

#[cfg(any(test, feature = "test-support"))]
fn attach_timer_evidence(mut result: WaitResult, evidence: TimerEvidence) -> WaitResult {
    result.timer_arm_qpc = evidence.timer_arm_qpc;
    result.timer_arm_target_qpc = evidence.timer_arm_target_qpc;
    result.timer_arm_generation = evidence.timer_arm_generation;
    result.timer_return_qpc = evidence.timer_return_qpc;
    result.timer_return_generation = evidence.timer_return_generation;
    result
}

#[cfg(not(any(test, feature = "test-support")))]
fn attach_timer_evidence(result: WaitResult) -> WaitResult {
    result
}

fn deadline_wait_result_with_timer_evidence(
    started_ticks: Option<QpcTicks>,
    completed_ticks: QpcTicks,
    #[cfg(any(test, feature = "test-support"))] evidence: TimerEvidence,
) -> WaitResult {
    attach_timer_evidence(
        deadline_wait_result(started_ticks, completed_ticks),
        #[cfg(any(test, feature = "test-support"))]
        evidence,
    )
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
    use super::{
        classify_interrupt_after_refresh, classify_kernel_wait_us, classify_wake,
        percentile_from_sorted,
    };
    use crate::clock::{DurationTicks, QpcClock, QpcTicks};
    use crate::wait::WaitOutcome;
    use std::num::NonZeroU64;

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

    #[test]
    fn kernel_wait_floors_are_safe_at_10mhz() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        let spin_threshold = DurationTicks::from_raw(100);

        assert_eq!(
            classify_kernel_wait_us(clock, 100, spin_threshold),
            Ok(None),
            "exactly the spin threshold must not arm a timer"
        );
        for ticks in [1, 2, 9] {
            assert_eq!(
                classify_kernel_wait_us(clock, 100 + ticks, spin_threshold),
                Ok(None),
                "positive sub-microsecond remainder of {ticks} QPC ticks must spin"
            );
        }
        assert_eq!(
            classify_kernel_wait_us(clock, 110, spin_threshold),
            Ok(NonZeroU64::new(1)),
            "exactly 1 microsecond must remain an ordinary timer wait"
        );
        assert_eq!(
            classify_kernel_wait_us(clock, 111, spin_threshold),
            Ok(NonZeroU64::new(1)),
            "1 microsecond plus one QPC tick preserves floor conversion"
        );
    }

    #[test]
    fn kernel_wait_floors_are_safe_at_3579545hz() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(3_579_545).unwrap());
        let spin_threshold = DurationTicks::from_raw(100);

        assert_eq!(
            classify_kernel_wait_us(clock, 101, spin_threshold),
            Ok(None),
            "a positive sub-microsecond remainder must not arm a timer"
        );
        assert_eq!(
            classify_kernel_wait_us(clock, 103, spin_threshold),
            Ok(None),
            "the last sub-microsecond remainder must stay on QPC recheck"
        );
        assert_eq!(
            classify_kernel_wait_us(clock, 104, spin_threshold),
            Ok(NonZeroU64::new(1)),
            "the first nonzero floored microsecond must arm normally"
        );
        assert_eq!(
            classify_kernel_wait_us(clock, 105, spin_threshold),
            Ok(NonZeroU64::new(1)),
            "the boundary after the first nonzero microsecond remains stable"
        );
    }
}
