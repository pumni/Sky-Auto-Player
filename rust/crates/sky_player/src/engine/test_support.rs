#[cfg(any(test, feature = "test-support"))]
pub(crate) mod command_timing;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod dispatch_harness;
#[cfg(any(test, feature = "test-support"))]
mod dispatch_timing_evidence;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_timing_evidence::PacketSenderAttemptEvidence;
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
pub use dispatch_harness::{
    CleanupCapture, DispatchTimingConfigEvidence, FinalizeEvidence, PhysicalFloorEvidence,
    PreparedBoundaryEvidence, PreparedStartupBuilder, PreparedStartupEvidence,
    ProductionDispatchTestHarness,
};
