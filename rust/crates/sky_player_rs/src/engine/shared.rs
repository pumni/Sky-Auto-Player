#[cfg(any(test, feature = "test-support"))]
use super::CommandTimingState;
use super::{NativeTelemetryOutput, SharedMetrics};
use parking_lot::Mutex;
use sky_dispatch_win32::event::OwnedEvent;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64};
use std::sync::{Condvar, Mutex as StdMutex};

/// Cross-thread session resources with one explicit owner.
///
/// The worker receives a borrow of this aggregate for its lifetime instead of
/// receiving a separate list of atomics and synchronization primitives. The
/// individual resources retain their existing types and ordering semantics.
pub(super) struct SessionCommands {
    pub(super) interrupt: OwnedEvent,
    pub(super) desired_pause: AtomicBool,
    pub(super) quit_requested: AtomicBool,
    pub(super) skip_requested: AtomicBool,
    pub(super) panic_requested: AtomicBool,
    pub(super) focus_active: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) command_timing: CommandTimingState,
}

pub(super) struct SessionTarget {
    pub(super) target_hwnd: AtomicIsize,
    pub(super) target_generation: AtomicU64,
}

pub(super) struct SessionLifecycle {
    pub(super) lifecycle: AtomicU8,
    pub(super) terminal_outcome: AtomicU8,
    pub(super) completed: (StdMutex<bool>, Condvar),
}

pub(super) struct SessionPublication {
    pub(super) metrics: SharedMetrics,
    pub(super) telemetry_output: Mutex<Option<NativeTelemetryOutput>>,
    pub(super) priority_acquired: Mutex<String>,
    pub(super) estimator_output: Mutex<Option<String>>,
    pub(super) supervisor_heartbeat_ticks: AtomicU64,
}

pub(super) struct SessionShared {
    pub(super) commands: SessionCommands,
    pub(super) target: SessionTarget,
    pub(super) lifecycle: SessionLifecycle,
    pub(super) publication: SessionPublication,
}
