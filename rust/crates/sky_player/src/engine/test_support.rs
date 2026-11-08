#[cfg(any(test, feature = "test-support"))]
pub(crate) mod command_timing;
#[cfg(all(windows, feature = "test-support"))]
mod compact_arena_study;
#[cfg(all(windows, feature = "test-support"))]
pub use compact_arena_study::{CompactDirectPreparedEvidence, HeldPreparedStudyEvidence};
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod dispatch_harness;
#[cfg(any(test, feature = "test-support"))]
mod fault_injection;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod mock_sender;

#[cfg(any(test, feature = "test-support"))]
pub use fault_injection::MockSendInputCapture;
#[cfg(any(test, feature = "test-support"))]
pub use fault_injection::{FaultInjectionScript, InjectedSendOutcome};

#[cfg(any(test, feature = "test-support"))]
pub use command_timing::CommandTimingResult;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use command_timing::{
    CommandTimingCleanup, CommandTimingLookup as PauseTimingLookup, CommandTimingState,
};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use dispatch_harness::{
    CleanupCapture, DispatchTimingConfigEvidence, FinalizeEvidence, PhysicalFloorEvidence,
    PreparedBoundaryEvidence, PreparedStartupBuilder, PreparedStartupEvidence,
    ProductionDispatchTestHarness,
};
