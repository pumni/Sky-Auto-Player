#[cfg(any(test, feature = "test-support"))]
pub(crate) mod command_timing;
mod fault_injection;

pub use fault_injection::{FaultInjectionScript, InjectedSendOutcome};

#[cfg(any(test, feature = "test-support"))]
pub use command_timing::CommandTimingResult;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use command_timing::{
    CommandTimingCleanup, CommandTimingLookup as PauseTimingLookup, CommandTimingState,
};
