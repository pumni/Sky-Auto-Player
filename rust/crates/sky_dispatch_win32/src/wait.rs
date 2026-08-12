//! Interruptible wait strategy with physical-deadline priority.

mod calibration;
mod spin;

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
    pub wake_qpc: Option<crate::clock::QpcTicks>,
    pub spin_ticks: crate::clock::DurationTicks,
}

impl WaitResult {
    pub const fn failed(failure: WaitFailure) -> Self {
        Self {
            outcome: WaitOutcome::Failed(failure),
            wake_qpc: None,
            spin_ticks: crate::clock::DurationTicks::ZERO,
        }
    }

    pub const fn interrupted() -> Self {
        Self {
            outcome: WaitOutcome::Interrupted,
            wake_qpc: None,
            spin_ticks: crate::clock::DurationTicks::ZERO,
        }
    }
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

mod hybrid;
pub use hybrid::HybridWaiter;

#[cfg(test)]
mod tests {
    use super::calibration::robust_wake_error_us;
    use super::spin::{spin_duration_ticks, wait_result_with_spin};
    use super::{HybridWaiter, WaitOutcome};
    use crate::clock::{DurationTicks, QpcTicks, qpc_now_us};
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
    fn already_due_target_beats_a_pending_interrupt() {
        let event = OwnedEvent::new_auto_reset().expect("event");
        assert!(event.signal());
        let waiter = HybridWaiter::new();
        let result = waiter.wait_until_ticks_with_metrics(QpcTicks::ZERO, 200, &event);

        assert_eq!(result.outcome, WaitOutcome::Deadline);
        assert_eq!(event.take_count(), 1);
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

    #[test]
    fn spin_duration_converts_one_completed_tick_interval() {
        assert_eq!(
            spin_duration_ticks(Some(QpcTicks::from_raw(1_000)), QpcTicks::from_raw(1_500),),
            Ok(crate::clock::DurationTicks::from_raw(500))
        );
        assert_eq!(
            spin_duration_ticks(None, QpcTicks::from_raw(1_500)),
            Ok(crate::clock::DurationTicks::ZERO)
        );
    }

    #[test]
    fn wait_result_keeps_raw_wake_and_spin_evidence() {
        let result = wait_result_with_spin(
            WaitOutcome::Deadline,
            Some(QpcTicks::from_raw(1_000)),
            QpcTicks::from_raw(1_500),
        );
        assert_eq!(result.outcome, WaitOutcome::Deadline);
        assert_eq!(result.wake_qpc, Some(QpcTicks::from_raw(1_500)));
        assert_eq!(result.spin_ticks, DurationTicks::from_raw(500));
    }

    #[test]
    fn final_spin_does_not_poll_the_event_object_per_iteration() {
        let event = OwnedEvent::new_auto_reset().expect("event");
        let waiter = HybridWaiter::new();
        let target = qpc_now_us().expect("test QPC clock") + 500;
        assert_eq!(
            waiter.wait_until_us(target, 500, &event),
            WaitOutcome::Deadline
        );
        // One initial consume and one final deadline handoff probe are
        // allowed; the final spin itself must not poll the event per iteration.
        assert!(event.take_count() <= 2);
    }

    #[test]
    fn signal_during_final_spin_interrupts_without_win32_polling_loop() {
        let event = std::sync::Arc::new(OwnedEvent::new_auto_reset().expect("event"));
        let signal_event = std::sync::Arc::clone(&event);
        let signaler = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1));
            assert!(signal_event.signal());
        });
        let waiter = HybridWaiter::new();
        let result = waiter.wait_until_us(
            qpc_now_us().expect("test QPC clock") + 50_000,
            50_000,
            &event,
        );
        signaler.join().expect("signaler");
        assert_eq!(result, WaitOutcome::Interrupted);
        assert!(event.take_count() <= 2);
    }
}
