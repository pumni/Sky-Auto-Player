//! End-to-end real-time native dispatch session engine.

mod config;
mod session;
mod shared;
mod snapshot;
pub(crate) mod telemetry;
#[cfg(any(test, feature = "test-support"))]
mod test_support;
mod worker;

pub use config::DispatchProfile;
#[cfg(any(test, feature = "test-support"))]
pub use config::StartupOrderingHook;
#[cfg(any(test, feature = "test-support"))]
pub use config::TestWaitPolicy;
use config::WorkerConfig;
pub use config::{
    BackendConfig, DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US, FocusOptions, NativeSessionOptions,
    PriorityOptions, TelemetryOptions, TimingOptions, WaitOptions,
};
pub use session::NativeDispatchSession;
pub use snapshot::{EnginePollSnapshot, EnginePollStatus, EngineProgressSnapshot, EngineSnapshot};
pub use telemetry::{
    NATIVE_TELEMETRY_SCHEMA_VERSION, NativeTelemetryOutput, NativeTelemetrySummary, RtTraceRecord,
    TelemetryMode, TimingSemantics, WorkerMetricsLocal,
};
pub(crate) use telemetry::{
    SharedMetrics, TRACE_FLAG_ANOMALY, TRACE_FLAG_DEFERRED, TRACE_FLAG_RECOVERY,
    TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TRACE_KIND_UP, TelemetryCollector, TraceContext,
    TraceDelivery, TraceTiming, cpu_metrics_sample_due, trace_outcome_code, try_publish_metrics,
};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::CommandTimingResult;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use test_support::mock_sender::create_mock_backend;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use test_support::{CommandTimingCleanup, CommandTimingState, PauseTimingLookup};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{FaultInjectionScript, InjectedSendOutcome};
#[cfg(test)]
pub(crate) use worker::*;
#[cfg(feature = "test-support")]
pub mod dispatch_primitives {
    //! Queue primitive types exported for the §8.11 no-alloc integration test only.
    //! Do not use in production code.
    pub use super::test_support::ProductionDispatchTestHarness;
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
    pub use super::worker::{NextDispatchPlan, PreparationCounts};

    /// Production timing policy constants shared by diagnostic benchmarks.
    pub const PRODUCTION_MIN_SPIN_THRESHOLD_US: u64 = super::config::MIN_CALIBRATED_SPIN_US;
    pub const PRODUCTION_SPIN_THRESHOLD_US: u64 = super::config::DEFAULT_SPIN_THRESHOLD_US;
    pub const PRODUCTION_CALIBRATION_SAMPLES: usize = super::config::CALIBRATION_SAMPLES;
    pub const PRODUCTION_CALIBRATION_BUDGET_US: u64 =
        super::config::CALIBRATION_MAX_STARTUP_BUDGET_US;
    pub const PRODUCTION_STARTUP_READINESS_RESERVE_US: u64 =
        super::config::STARTUP_READINESS_RESERVE_US;
    pub const LEGACY_ADAPTIVE_SPIN_FLOOR_US: u64 = 700;

    /// Apply the frozen production calibration policy to benchmark wake stats.
    pub fn calibrated_spin_threshold_us(stats: sky_dispatch_win32::wait::WakeErrorStats) -> u64 {
        super::worker::calibrated_spin_threshold_us(stats)
    }

    /// Derive the retired adaptive policy for A/B diagnostics only. It is not
    /// part of the production worker configuration or control path.
    pub fn legacy_adaptive_spin_threshold_us(wake_error_us: u64) -> u64 {
        super::worker::derive_spin_threshold_us(wake_error_us, LEGACY_ADAPTIVE_SPIN_FLOOR_US)
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
/// public path via `sky_player_rs::engine::observer_test_hooks::*` is stable
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
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::model::ActionKind;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcError, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::cpu::{current_process_cpu_time_us, current_thread_cpu_time_us};
use sky_dispatch_win32::event::OwnedEvent;
#[cfg(test)]
pub(crate) use sky_dispatch_win32::input::PlatformSendResult;
use sky_dispatch_win32::input::TrackedKeyState;
#[cfg(test)]
pub(crate) use sky_dispatch_win32::wait::WakeErrorStats;
use sky_dispatch_win32::wait::{HybridWaiter, WaitOutcome};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64, Ordering};

const LIFECYCLE_NEW: u8 = 0;
const LIFECYCLE_RUNNING: u8 = 1;
const LIFECYCLE_FINISHED: u8 = 2;
const LIFECYCLE_POISONED: u8 = 3;
const OUTCOME_NONE: u8 = 0;
const OUTCOME_FINISHED: u8 = 1;
const OUTCOME_QUIT: u8 = 2;
const OUTCOME_SKIPPED: u8 = 3;
const OUTCOME_ERROR: u8 = 4;
const PAUSED_POLL_US: u64 = 2_000;
const CPU_METRICS_SAMPLE_INTERVAL_US: u64 = 100_000;
#[cfg(test)]
mod tests;
