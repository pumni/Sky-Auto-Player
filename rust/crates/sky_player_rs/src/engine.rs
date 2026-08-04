//! End-to-end real-time native dispatch session engine.

mod config;
mod session;
mod shared;
mod snapshot;
pub(crate) mod telemetry;
mod test_support;
mod worker;

pub use config::DispatchProfile;
use config::WorkerConfig;
pub use session::NativeDispatchSession;
pub use snapshot::EngineSnapshot;
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
pub(crate) use test_support::{CommandTimingCleanup, CommandTimingState, PauseTimingLookup};
pub use test_support::{FaultInjectionScript, InjectedSendOutcome};
#[cfg(test)]
pub(crate) use worker::*;

use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::{ActionKind, RuntimeSchedule};
use sky_dispatch_core::time::{
    DurationTicks, SEND_COLD_THRESHOLD_US, TimeArithmeticError, TimelineTicks,
};
#[cfg(test)]
use sky_dispatch_win32::clock::qpc_us_to_ticks;
use sky_dispatch_win32::clock::{QpcClock, QpcError, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::cpu::{current_process_cpu_time_us, current_thread_cpu_time_us};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::input::{PlatformSendResult, ReleaseAllOutcome, TrackedKeyState};
use sky_dispatch_win32::mmcss::{MmcssGuard, PriorityMode};
use sky_dispatch_win32::power::PowerThrottlingGuard;
use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure, WaitOutcome, WakeErrorStats};
use smallvec::SmallVec;
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
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
const CORE_WARMUP_SPIN_MAX_US: u64 = 500;
const CPU_METRICS_SAMPLE_INTERVAL_US: u64 = 100_000;
const INPUT_PATH_WINDOW_CAPACITY: usize = 64;
const STRICT_RETRY_LATE_THRESHOLD_US: u64 = 2_000;
const HARD_LATE_ABORT_THRESHOLD_US: u64 = 20_000;
const STRICT_SATURATION_ABORT_STREAK: u8 = 3;
const STARTUP_WAKE_GUARD_US: u64 = 1_000;
const RELEASE_RETRY_BACKOFF_US: [u64; 4] = [2_000, 5_000, 10_000, 20_000];
#[cfg(test)]
mod tests;
