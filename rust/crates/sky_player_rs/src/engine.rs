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
use config::WorkerConfig;
pub(crate) use config::{
    BackendConfig, EstimatorOptions, FocusOptions, NativeSessionOptions, PriorityOptions,
    TelemetryOptions, TimingOptions, WaitOptions,
};
pub use session::NativeDispatchSession;
pub use snapshot::{EngineProgressSnapshot, EngineSnapshot};
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
    pub use super::worker::NextDispatchPlan;
    pub use super::worker::dispatch::DispatchStep;
    pub use super::worker::dispatch::observation::{
        DispatchObservation, DownObservation, DownTraceObservation, OBSERVATION_QUEUE_CAPACITY,
        UpObservation, UpTraceObservation,
    };
    pub use super::worker::dispatch::observer::PendingObservationQueue;
    pub use super::worker::dispatch::timing::{
        EstimatorObservationEvidence, is_clean_estimator_observation,
    };
    pub use super::worker::health::DispatchPath;
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

    /// Override the worker's initial observer budget in microseconds (0 disables).
    pub fn set_observer_initial_budget_override_us(us: u64) {
        super::worker::dispatch::observer::set_observer_initial_budget_override_us(us);
    }

    /// Clear artificial cost and budget override after a scenario.
    pub fn reset_observer_test_hooks() {
        super::worker::dispatch::observer::reset_observer_test_hooks();
    }

    /// Inject a post-send telemetry error only when release recovery is
    /// exhausted. Test-only: production never enables this hook.
    pub fn set_release_telemetry_failure_on_recovery(enabled: bool) {
        super::worker::dispatch::set_release_telemetry_failure_on_recovery(enabled);
    }

    /// Inject a post-send observer error only when release recovery is
    /// exhausted. Test-only: production never enables this hook.
    pub fn set_release_observer_failure_on_recovery(enabled: bool) {
        super::worker::dispatch::set_release_observer_failure_on_recovery(enabled);
    }

    /// Return whether exhausted release recovery completed before the ready
    /// boundary was sampled in the current test scenario.
    pub fn release_recovery_completed_before_ready() -> bool {
        super::worker::dispatch::release_recovery_completed_before_ready()
    }
}

use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::ActionKind;
use sky_dispatch_core::time::{DurationTicks, SEND_COLD_THRESHOLD_US, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcError, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::cpu::{current_process_cpu_time_us, current_thread_cpu_time_us};
use sky_dispatch_win32::event::OwnedEvent;
#[cfg(test)]
pub(crate) use sky_dispatch_win32::input::PlatformSendResult;
use sky_dispatch_win32::input::TrackedKeyState;
#[cfg(test)]
pub(crate) use sky_dispatch_win32::wait::WakeErrorStats;
use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure, WaitOutcome};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

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
const STRICT_RETRY_LATE_THRESHOLD_US: u64 = 2_000;
const HARD_LATE_ABORT_THRESHOLD_US: u64 = 20_000;
const STRICT_SATURATION_ABORT_STREAK: u8 = 3;
const STARTUP_WAKE_GUARD_US: u64 = 1_000;
const RELEASE_RETRY_BACKOFF_US: [u64; 4] = [2_000, 5_000, 10_000, 20_000];
#[cfg(test)]
mod tests;
