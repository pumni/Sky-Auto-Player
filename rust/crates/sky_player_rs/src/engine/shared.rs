use super::*;
use std::sync::{Arc, Condvar, Mutex as StdMutex};

/// Cross-thread session resources with one explicit owner.
///
/// The worker receives a borrow of this aggregate for its lifetime instead of
/// receiving a separate list of atomics and synchronization primitives. The
/// individual resources retain their existing types and ordering semantics.
pub(super) struct SessionShared {
    pub(super) interrupt: Arc<OwnedEvent>,
    pub(super) desired_pause: Arc<AtomicBool>,
    pub(super) quit_requested: Arc<AtomicBool>,
    pub(super) skip_requested: Arc<AtomicBool>,
    pub(super) panic_requested: Arc<AtomicBool>,
    pub(super) focus_active: Arc<AtomicBool>,
    pub(super) target_hwnd: Arc<AtomicIsize>,
    pub(super) target_generation: Arc<AtomicU64>,
    pub(super) lifecycle: Arc<AtomicU8>,
    pub(super) terminal_outcome: Arc<AtomicU8>,
    pub(super) metrics: Arc<SharedMetrics>,
    pub(super) completed: Arc<(StdMutex<bool>, Condvar)>,
    pub(super) telemetry_output: Arc<Mutex<Option<NativeTelemetryOutput>>>,
    pub(super) priority_acquired: Arc<Mutex<String>>,
    pub(super) estimator_output: Arc<Mutex<Option<String>>>,
    pub(super) supervisor_heartbeat_ticks: Arc<AtomicU64>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) command_timing: Arc<CommandTimingState>,
}
