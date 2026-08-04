use super::*;
use std::sync::{Condvar, Mutex as StdMutex};

/// Cross-thread session resources with one explicit owner.
///
/// The worker receives a borrow of this aggregate for its lifetime instead of
/// receiving a separate list of atomics and synchronization primitives. The
/// individual resources retain their existing types and ordering semantics.
pub(super) struct SessionShared {
    pub(super) interrupt: OwnedEvent,
    pub(super) desired_pause: AtomicBool,
    pub(super) quit_requested: AtomicBool,
    pub(super) skip_requested: AtomicBool,
    pub(super) panic_requested: AtomicBool,
    pub(super) focus_active: AtomicBool,
    pub(super) target_hwnd: AtomicIsize,
    pub(super) target_generation: AtomicU64,
    pub(super) lifecycle: AtomicU8,
    pub(super) terminal_outcome: AtomicU8,
    pub(super) metrics: SharedMetrics,
    pub(super) completed: (StdMutex<bool>, Condvar),
    pub(super) telemetry_output: Mutex<Option<NativeTelemetryOutput>>,
    pub(super) priority_acquired: Mutex<String>,
    pub(super) estimator_output: Mutex<Option<String>>,
    pub(super) supervisor_heartbeat_ticks: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) command_timing: CommandTimingState,
}
