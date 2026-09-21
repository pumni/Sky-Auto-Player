//! Dispatch subtree: authored packet path, pure timing projection, and the
//! observer/publish stage.
//!
//! Structural ownership:
//! - `authored.rs` owns the authored physical packet dispatch, final
//!   admission, physical sender invocation, backend result handling, and
//!   coordinator commit for the note-on path.
//! - `timing.rs` owns pure typed timing projection (sender start/completion,
//!   completion errors, strict completion predicates, typed duration
//!   conversion). It must not import `SharedMetrics`, `TelemetryCollector`,
//!   `Mutex`, or Python types.
//! - `observer.rs` owns health observation, the telemetry observer stage,
//!   worker metric updates, and shared snapshot publication.

mod authored;
pub(crate) mod hold_forensics;
pub(crate) mod observation;
pub(crate) mod observer;
mod observer_trace;
mod observer_wake;
mod prepared;
#[cfg(test)]
mod prepared_characterization_tests;
mod recovery;
pub(crate) mod timing;

pub(crate) use recovery::{DownMissReason, classify_missed_down_boundary};

/// Outcome of one authored packet dispatch step.
#[derive(Debug)]
pub enum DispatchStep {
    NoWork,
    Dispatched,
    Continue,
    Terminate(String),
    TerminateStatic(&'static str),
}

/// Exact identity of an authored physical Down boundary that was frozen by
/// the coordinator and observed while still in the future.
///
/// The QPC target is deliberately not sufficient on its own: two distinct
/// authored boundaries may share a target after projection or test setup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PhysicalBoundaryStamp {
    pub(crate) first_batch_index: usize,
    pub(crate) packet_index: usize,
    pub(crate) packet_batch_count: usize,
    pub(crate) source_action_index: u32,
    pub(crate) up_mask: u16,
    pub(crate) down_mask: u16,
    pub(crate) physical_target_qpc: QpcTicks,
}

/// Prepared-normal future observation tied to one exact physical boundary and
/// the target generation that was verified before waiting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreparedDownAuthorization {
    pub(crate) boundary: PhysicalBoundaryStamp,
    pub(crate) target_generation: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DownBoundaryState {
    #[default]
    AwaitingFuture,
    FutureAuthorized(PhysicalBoundaryStamp),
}

impl DownBoundaryState {
    #[inline]
    pub(crate) const fn awaiting_future(self) -> bool {
        matches!(self, Self::AwaitingFuture)
    }

    #[inline]
    pub(crate) const fn authorization(self) -> Option<PhysicalBoundaryStamp> {
        match self {
            Self::FutureAuthorized(stamp) => Some(stamp),
            Self::AwaitingFuture => None,
        }
    }
}

/// Musical admission class for one prepared Down-bearing boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DownBoundaryAdmission {
    Authorized,
    UnobservedBacklog,
}

impl DownBoundaryAdmission {
    #[inline]
    pub(crate) const fn is_missed(self) -> bool {
        matches!(self, Self::UnobservedBacklog)
    }
}

/// A Down miss that has already been classified at its authored boundary,
/// with only its prepared Up-prefix still waiting for the physical hold floor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingUpRecovery {
    pub(crate) boundary: PhysicalBoundaryStamp,
    pub(crate) admission: DownBoundaryAdmission,
    /// Frozen coordinator token for resolving the already-missed Down if a
    /// lifecycle safety release interrupts the delayed Up-prefix recovery.
    pub(crate) authored_commit: PreparedAuthoredCommit,
}

impl PendingUpRecovery {
    #[inline]
    pub(crate) fn matches_authored_boundary(&self, boundary: PhysicalBoundaryStamp) -> bool {
        self.boundary.same_authored_boundary(boundary)
    }
}

impl PhysicalBoundaryStamp {
    #[inline]
    pub(crate) fn same_authored_boundary(self, other: Self) -> bool {
        self.first_batch_index == other.first_batch_index
            && self.packet_index == other.packet_index
            && self.packet_batch_count == other.packet_batch_count
            && self.source_action_index == other.source_action_index
            && self.up_mask == other.up_mask
            && self.down_mask == other.down_mask
    }
}

pub(crate) struct AuthoredPacketContext<'a> {
    pub(crate) dispatch_plan: &'a NextDispatchPlan,
    pub(crate) effective_now_ticks: TimelineTicks,
    pub(crate) now_ticks: QpcTicks,
    pub(crate) physical_timing_window: super::physical_timing_guard::PhysicalTimingWindow,
    pub(crate) down_admission: DownBoundaryAdmission,
    pub(crate) focus_loss_fault: bool,
    pub(crate) supervisor_expired: &'a std::sync::atomic::AtomicBool,
    /// QPC sample returned by the direct target wait. When present, the
    /// final admission gate reuses it instead of entering a second wait.
    pub(crate) boundary_crossing_qpc: Option<QpcTicks>,
    /// Test-only direct-boundary admission for frozen-plan correctness tests.
    /// This field and its branch are absent from production builds.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) test_direct_boundary: bool,
    /// Test-only control for whether the sender receives an injected start
    /// timestamp. The Phase-A acceptance boundary keeps direct admission but
    /// leaves this false so the production target-crossing sender owns QPC
    /// sampling.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) test_inject_sender_start: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryDescriptor {
    None,
    UpPrefix { up_len: u8, up_mask: u16 },
}

/// Snapshot of the prepared authored batch plus the projection of the
/// schedule view used by admission, send, and telemetry.
///
/// Built once per authored epoch by the typed frame-view helpers in
/// `timing`; the send/admission/telemetry helpers consume it without
/// re-querying the coordinator schedule.
#[cfg(not(any(test, feature = "test-support")))]
#[derive(Debug)]
pub(crate) struct AuthoredBatchView {
    pub(super) prepared_batch: PreparedBatch,
    pub(super) batch_source_action_index: u32,
    pub(super) batch_intent_count: usize,
    pub(super) batch_kind: ActionKind,
    pub(super) batch_scheduled_ticks: TimelineTicks,
    pub(super) authored_batch_scheduled_ticks: TimelineTicks,
    pub(super) conflict_mask: u16,
    pub(super) dispatch_path: DispatchPath,
    pub(super) packet_masks: PhysicalPacket,
    pub(super) prepared_packet: sky_dispatch_win32::input::PreparedPhysicalPacket,
    pub(super) recovery: RecoveryDescriptor,
    pub(super) commit: PhysicalCommit,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub(crate) struct AuthoredBatchView {
    pub(crate) prepared_batch: PreparedBatch,
    pub(crate) batch_source_action_index: u32,
    pub(crate) batch_intent_count: usize,
    pub(crate) batch_kind: ActionKind,
    pub(crate) batch_scheduled_ticks: TimelineTicks,
    pub(crate) authored_batch_scheduled_ticks: TimelineTicks,
    pub(crate) conflict_mask: u16,
    pub(crate) dispatch_path: DispatchPath,
    pub(crate) packet_masks: PhysicalPacket,
    pub(crate) prepared_packet: sky_dispatch_win32::input::PreparedPhysicalPacket,
    pub(crate) recovery: RecoveryDescriptor,
    pub(crate) commit: PhysicalCommit,
}

/// `Err(None)` indicates an unrecoverable terminal step; `Ok(None)` means the
/// coordinator offered no authored work for this epoch (worker should advance
/// the wait deadline instead).
pub(super) type BatchViewResult = Result<Option<AuthoredBatchView>, DispatchStep>;

pub(crate) use authored::dispatch_authored_packet;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use observation::DispatchObservation;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use observer::drain_one_observer;
pub(crate) use observer::{ObserverRuntime, PendingObservationQueue, dispatch_stale_packet};
pub(crate) use prepared::dispatch_prepared_normal_frame;

use super::super::{ActionKind, QpcTicks, TimelineTicks};
use super::DispatchPath;
use super::planning::NextDispatchPlan;
use sky_dispatch_core::coordinator::{PreparedAuthoredCommit, PreparedBatch};
use sky_dispatch_win32::input::PhysicalPacket;

#[derive(Clone, Debug)]
pub(crate) enum PhysicalCommit {
    Authored(PreparedAuthoredCommit),
    PendingRelease {
        release_mask: u16,
        due_ticks: TimelineTicks,
    },
    Coalesced {
        authored: PreparedAuthoredCommit,
        release_mask: u16,
        due_ticks: TimelineTicks,
    },
}
