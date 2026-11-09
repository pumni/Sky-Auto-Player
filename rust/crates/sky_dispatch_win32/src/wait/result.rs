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
}

impl WaitResult {
    pub const fn failed(failure: WaitFailure) -> Self {
        Self {
            outcome: WaitOutcome::Failed(failure),
            wake_qpc: None,
            spin_ticks: crate::clock::DurationTicks::ZERO,
            #[cfg(any(test, feature = "test-support"))]
            timer_return_qpc: None,
        }
    }

    pub const fn interrupted() -> Self {
        Self {
            outcome: WaitOutcome::Interrupted,
            wake_qpc: None,
            spin_ticks: crate::clock::DurationTicks::ZERO,
            #[cfg(any(test, feature = "test-support"))]
            timer_return_qpc: None,
        }
    }
}
