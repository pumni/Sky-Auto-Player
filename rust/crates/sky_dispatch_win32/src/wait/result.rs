use super::{WaitFailure, WaitOutcome};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitResult {
    pub outcome: WaitOutcome,
    pub wake_qpc: Option<crate::clock::QpcTicks>,
    pub spin_ticks: crate::clock::DurationTicks,
    /// Reuses the waiter's first QPC sample after a timer return. This
    /// qualification timestamp adds no QPC read to the wait loop.
    #[cfg(any(test, feature = "test-support"))]
    pub timer_return_qpc: Option<crate::clock::QpcTicks>,
    /// Requested target of the most recent successful relative timer arm.
    #[cfg(any(test, feature = "test-support"))]
    pub timer_arm_qpc: Option<crate::clock::QpcTicks>,
    #[cfg(any(test, feature = "test-support"))]
    pub timer_arm_target_qpc: Option<crate::clock::QpcTicks>,
    /// Local wait-call identity used to pair a timer return with its arm.
    #[cfg(any(test, feature = "test-support"))]
    pub timer_arm_generation: Option<u64>,
    #[cfg(any(test, feature = "test-support"))]
    pub timer_return_generation: Option<u64>,
}

impl WaitResult {
    pub const fn failed(failure: WaitFailure) -> Self {
        Self {
            outcome: WaitOutcome::Failed(failure),
            wake_qpc: None,
            spin_ticks: crate::clock::DurationTicks::ZERO,
            #[cfg(any(test, feature = "test-support"))]
            timer_return_qpc: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_qpc: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_target_qpc: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_generation: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_return_generation: None,
        }
    }

    pub const fn interrupted() -> Self {
        Self {
            outcome: WaitOutcome::Interrupted,
            wake_qpc: None,
            spin_ticks: crate::clock::DurationTicks::ZERO,
            #[cfg(any(test, feature = "test-support"))]
            timer_return_qpc: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_qpc: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_target_qpc: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_generation: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_return_generation: None,
        }
    }
}
