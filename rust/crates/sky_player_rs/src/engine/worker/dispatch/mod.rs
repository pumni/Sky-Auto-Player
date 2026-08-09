//! Dispatch subtree: authored note-on path, pending-release note-off path,
//! pure timing projection, and the observer/publish stage.
//!
//! Structural ownership:
//! - `authored.rs` owns the authored physical packet dispatch, final
//!   admission, physical sender invocation, backend result handling, and
//!   coordinator commit for the note-on path.
//! - `release.rs` owns pending-release physical send, `ReleaseTransportEvidence`,
//!   retry/recovery reconciliation, and the coordinator pending-release
//!   transition.
//! - `timing.rs` owns pure typed timing projection (sender start/completion,
//!   completion errors, strict completion predicates, typed duration
//!   conversion). It must not import `SharedMetrics`, `TelemetryCollector`,
//!   `Mutex`, or Python types.
//! - `observer.rs` owns estimator observation, health observation, the
//!   telemetry observer stage, worker metric updates, and shared snapshot
//!   publication.

mod authored;
pub(crate) mod observation;
pub(crate) mod observer;
mod release;
pub(crate) mod timing;

/// Outcome of one dispatch-loop authored or pending-release step.
#[derive(Debug)]
pub enum DispatchStep {
    NoWork,
    Dispatched,
    Continue,
    Terminate(String),
}

pub(crate) struct AuthoredPacketContext<'a> {
    pub(crate) dispatch_plan: &'a NextDispatchPlan,
    pub(crate) effective_now_ticks: TimelineTicks,
    pub(crate) now_ticks: QpcTicks,
    pub(crate) latency_class: LatencyClass,
    pub(crate) focus_loss_fault: bool,
    pub(crate) supervisor_heartbeat_ticks: &'a std::sync::atomic::AtomicU64,
    pub(crate) lease_timeout_ticks: DurationTicks,
}

/// Snapshot of the prepared authored batch plus the projection of the
/// schedule view used by admission, send, and telemetry.
///
/// Built once per authored epoch by `timing::prepare_authored_batch_view`;
/// the send/admission/telemetry helpers consume it without re-querying the
/// coordinator schedule.
#[cfg(not(any(test, feature = "test-support")))]
pub(super) struct AuthoredBatchView {
    pub(super) prepared_batch: PreparedBatch,
    pub(super) batch_source_action_index: u32,
    pub(super) batch_intent_count: usize,
    pub(super) batch_kind: ActionKind,
    pub(super) batch_scheduled_ticks: TimelineTicks,
    pub(super) authored_batch_scheduled_ticks: TimelineTicks,
    pub(super) conflict_mask: u16,
    pub(super) dispatch_path: DispatchPath,
    pub(super) packet_masks: Option<PhysicalPacket>,
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) struct AuthoredBatchView {
    pub(crate) prepared_batch: PreparedBatch,
    pub(crate) batch_source_action_index: u32,
    pub(crate) batch_intent_count: usize,
    pub(crate) batch_kind: ActionKind,
    pub(crate) batch_scheduled_ticks: TimelineTicks,
    pub(crate) authored_batch_scheduled_ticks: TimelineTicks,
    pub(crate) conflict_mask: u16,
    pub(crate) dispatch_path: DispatchPath,
    pub(crate) packet_masks: Option<PhysicalPacket>,
}

/// `Err(None)` indicates an unrecoverable terminal step; `Ok(None)` means the
/// coordinator offered no authored work for this epoch (worker should advance
/// the wait deadline instead).
pub(super) type BatchViewResult = Result<Option<AuthoredBatchView>, DispatchStep>;

pub(crate) use authored::dispatch_authored_packet;
pub(crate) use observation::DispatchObservation;
pub(crate) use observer::{PendingObservationQueue, drain_one_observer, observer_has_safe_slack};
pub(crate) use release::{PendingReleaseContext, dispatch_due_pending_releases};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use release::{
    release_recovery_completed_before_ready, set_release_observer_failure_on_recovery,
    set_release_telemetry_failure_on_recovery,
};

use super::super::{ActionKind, DurationTicks, LatencyClass, QpcTicks, TimelineTicks};
use super::DispatchPath;
use super::planning::NextDispatchPlan;
pub(super) use super::publish_backend_metrics;
use sky_dispatch_core::coordinator::PreparedBatch;
use sky_dispatch_win32::input::PhysicalPacket;
