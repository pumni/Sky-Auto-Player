//! End-to-end real-time native dispatch session engine.

mod config;
mod session;
mod shared;
mod snapshot;
mod target;
pub(crate) mod telemetry;
#[cfg(any(test, feature = "test-support"))]
mod test_support;
#[cfg(test)]
mod tests;
mod worker;

pub use config::DispatchProfile;
#[cfg(any(test, feature = "test-support"))]
pub use config::StartupCalibrationPlacement;
#[cfg(any(test, feature = "test-support"))]
pub use config::TestWaitPolicy;
use config::WorkerConfig;
pub use config::{
    BackendConfig, DEFAULT_MODIFIER_HOLD_TIMEOUT_US, DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
    FocusOptions, NativeSessionOptions, PriorityOptions, TelemetryOptions, TimingOptions,
    WaitOptions,
};
#[cfg(any(test, feature = "test-support"))]
pub use config::{RestoreRaceHook, StartupCheckpoint, StartupOrderingHook};
#[cfg(feature = "real-input-acceptance")]
pub use session::NativeAcceptanceLifecycleEvidence;
pub use session::{
    NativeDispatchSession, OwnerIdentityPollResult, SupervisorLifetimeGuard, SystemPowerSnapshot,
};
pub use shared::SystemPowerEndpoint;
#[cfg(any(test, feature = "test-support"))]
pub use shared::SystemPowerState;
pub use snapshot::{
    EnginePollSnapshot, EnginePollStatus, EngineProgressSnapshot, EngineSnapshot,
    WorkerProgressState,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use telemetry::TRACE_FLAG_DEFERRED;
#[cfg(test)]
pub(crate) use telemetry::try_publish_metrics;
pub use telemetry::{
    NATIVE_TELEMETRY_SCHEMA_VERSION, NATIVE_UP_SLOT_TRACE_CAPACITY, NativeTelemetryOutput,
    NativeTelemetrySummary, RtTraceRecord, TelemetryMode, TimestampProvenance, TimingSemantics,
    UpSlotTrace, WorkerMetricsLocal,
};
pub(crate) use telemetry::{
    SharedMetrics, TRACE_FLAG_ANOMALY, TRACE_FLAG_RECOVERY, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN,
    TRACE_KIND_MIXED, TRACE_KIND_UP, TRACE_SEND_STATUS_NOT_ATTEMPTED, TelemetryCollector,
    TraceContext, TraceDelivery, TraceTiming, UpSlotLossReason, cpu_metrics_sample_due,
    publish_lifecycle_metrics, trace_outcome_code, trace_send_status_code,
    try_publish_metrics_with_update, try_publish_observer_metrics,
};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::CommandTimingResult;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use test_support::mock_sender::create_mock_backend;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use test_support::{CommandTimingCleanup, CommandTimingState, PauseTimingLookup};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{FaultInjectionScript, InjectedSendOutcome, MockSendInputCapture};
#[cfg(test)]
pub(crate) use worker::*;
#[cfg(any(test, feature = "test-support"))]
pub use worker::{StagedCandidate, StagedDispatchOutcome};
#[cfg(any(test, feature = "test-support"))]
pub mod dispatch_primitives {
    //! Queue primitive types exported for the §8.11 no-alloc integration test only.
    //! Do not use in production code.
    pub use super::target::SessionTarget;
    pub use super::test_support::{
        CleanupCapture, DispatchTimingConfigEvidence, FinalizeEvidence, PacketCapture,
        ProductionDispatchTestHarness,
    };
    pub use super::worker::WaitObservation;
    pub use super::worker::dispatch::DispatchStep;
    pub use super::worker::dispatch::observation::{
        DispatchObservation, DownObservation, DownTraceObservation, OBSERVATION_QUEUE_CAPACITY,
        PrecisionHandoffEvidence, UpObservation, UpTraceObservation,
    };
    pub use super::worker::dispatch::observer::PendingObservationQueue;
    pub use super::worker::dispatch::timing::{
        DispatchObservationEvidence, is_clean_dispatch_observation,
    };
    pub use super::worker::health::DispatchPath;

    /// Production timing policy constants shared by diagnostic benchmarks.
    pub const PRODUCTION_MIN_SPIN_THRESHOLD_US: u64 = super::config::MIN_CALIBRATED_SPIN_US;
    pub const PRODUCTION_SPIN_THRESHOLD_US: u64 = super::config::DEFAULT_SPIN_THRESHOLD_US;
    pub const PRODUCTION_CALIBRATION_SAMPLES: usize = super::config::CALIBRATION_SAMPLES;
    pub const PRODUCTION_CALIBRATION_BUDGET_US: u64 =
        super::config::CALIBRATION_MAX_STARTUP_BUDGET_US;
    pub const PRODUCTION_STARTUP_READINESS_RESERVE_US: u64 =
        super::config::STARTUP_READINESS_RESERVE_US;
    pub const PRODUCTION_MIN_PREROLL_US: u64 = super::config::MIN_PRODUCTION_PREROLL_US;

    /// Apply the frozen production calibration policy to benchmark wake stats.
    pub fn calibrated_spin_threshold_us(stats: sky_dispatch_win32::wait::WakeErrorStats) -> u64 {
        super::worker::calibrated_spin_threshold_us(stats)
    }
}

/// Narrow native surface needed by delivery adapters without bringing a
/// delivery framework into the player engine.
pub mod binding_support {
    pub use crate::adapter_support::*;
}

/// Test-only hooks for §8.12 slow-observer regression scenarios.
///
/// Functions are hand-written wrappers (not `pub use` re-exports) so the
/// public path via `sky_player::observer_test_hooks::*` is stable
/// for both crate-internal unit tests and the external `tests/` integration
/// test binary, without leaking the crate-internal `worker::dispatch` module
/// into the public API (E0364).
#[cfg(any(test, feature = "test-support"))]
pub mod observer_test_hooks {
    pub use super::worker::dispatch::observer::ObserverTestHookGuard;

    /// Acquire exclusive access to observer test timing hooks.
    pub fn observer_test_hook_guard() -> ObserverTestHookGuard {
        super::worker::dispatch::observer::observer_test_hook_guard()
    }

    /// Force every observer drain to sleep this many microseconds.
    pub fn set_observer_artificial_cost_us(us: u64) {
        super::worker::dispatch::observer::set_observer_artificial_cost_us(us);
    }

    /// Clear the artificial observer cost after a scenario.
    pub fn reset_observer_test_hooks() {
        super::worker::dispatch::observer::reset_observer_test_hooks();
    }
}

use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
#[cfg(any(test, feature = "test-support"))]
pub use sky_dispatch_core::executor::{DispatchExecutor, DispatchTimingWindow};
#[cfg(any(test, feature = "test-support"))]
pub use sky_dispatch_core::model::ActionKind;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcError, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::cpu::{current_process_cpu_time_us, current_thread_cpu_time_us};
use sky_dispatch_win32::event::OwnedEvent;
pub(crate) use sky_dispatch_win32::input::TrackedKeyState;
use sky_dispatch_win32::wait::HybridWaiter;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

const LIFECYCLE_NEW: u8 = 0;
const LIFECYCLE_STARTING: u8 = 1;
const LIFECYCLE_RUNNING: u8 = 2;
const LIFECYCLE_FINISHED: u8 = 3;
const LIFECYCLE_POISONED: u8 = 4;
const OUTCOME_NONE: u8 = 0;
const OUTCOME_FINISHED: u8 = 1;
const OUTCOME_QUIT: u8 = 2;
const OUTCOME_SKIPPED: u8 = 3;
const OUTCOME_ERROR: u8 = 4;
const OUTCOME_SYSTEM_SUSPENDED: u8 = 5;
const CPU_METRICS_SAMPLE_INTERVAL_US: u64 = 100_000;

/// Classification of why the playback worker reached its terminal boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerTerminalCause {
    Finished,
    Quit,
    Skipped,
    SystemSuspended,
    SenderAnomaly,
    WorkerPanic,
    CleanupPanic,
    Failure,
}

/// Stable terminal outcome category, independent of the legacy numeric worker
/// status used by the session polling ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerTerminalOutcome {
    Finished,
    Quit,
    Skipped,
    SystemSuspended,
    Error,
}

impl WorkerTerminalOutcome {
    pub(crate) const fn legacy_code(self) -> u8 {
        match self {
            Self::Finished => OUTCOME_FINISHED,
            Self::Quit => OUTCOME_QUIT,
            Self::Skipped => OUTCOME_SKIPPED,
            Self::SystemSuspended => OUTCOME_SYSTEM_SUSPENDED,
            Self::Error => OUTCOME_ERROR,
        }
    }
}

/// Physical journal state and its immutable origin/profile, retained for a
/// later recovery layer to inspect without claiming that cleanup released it.
#[derive(Clone, Debug)]
pub struct TerminalReleaseResidual {
    pub outstanding_mask: u16,
    pub needs_up_mask: u16,
    pub needs_probe_mask: u16,
    pub owner: Option<sky_dispatch_win32::input::ReleaseOwnerProof>,
    pub profile: [sky_dispatch_win32::input::PhysicalKey; sky_dispatch_core::model::MAX_KEYS],
}

impl TerminalReleaseResidual {
    pub(crate) fn from_backend(backend: &TrackedKeyState) -> Self {
        Self {
            outstanding_mask: backend.release_obligation_mask(),
            needs_up_mask: backend.needs_up_mask(),
            needs_probe_mask: backend.needs_probe_mask,
            owner: backend.session_release_owner_proof().cloned(),
            profile: std::array::from_fn(|slot| {
                backend.instrument_key_profile().physical_key(slot)
            }),
        }
    }

    pub(crate) fn empty() -> Self {
        let profile =
            sky_dispatch_win32::input::profile::MaterializedInstrumentKeyProfile::canonical();
        Self {
            outstanding_mask: 0,
            needs_up_mask: 0,
            needs_probe_mask: 0,
            owner: None,
            profile: std::array::from_fn(|slot| profile.physical_key(slot)),
        }
    }
}

/// Typed terminal record published after the cleanup reducer has run.
#[derive(Clone, Debug)]
pub struct WorkerTerminalResult {
    pub outcome: WorkerTerminalOutcome,
    pub cause: WorkerTerminalCause,
    pub terminal_error: Option<String>,
    pub residual: TerminalReleaseResidual,
}
