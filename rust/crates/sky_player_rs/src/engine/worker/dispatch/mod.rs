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
mod observer;
mod release;
mod timing;

/// Outcome of one dispatch-loop authored or pending-release step.
pub(crate) enum DispatchStep {
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
}

/// Snapshot of the prepared authored batch plus the projection of the
/// schedule view used by admission, send, and telemetry.
///
/// Built once per authored epoch by `timing::prepare_authored_batch_view`;
/// the send/admission/telemetry helpers consume it without re-querying the
/// coordinator schedule.
pub(super) struct AuthoredBatchView {
    pub(super) prepared_batch: PreparedBatch,
    pub(super) batch_source_action_index: u32,
    pub(super) batch_intent_count: usize,
    pub(super) batch_kind: ActionKind,
    pub(super) batch_scheduled_ticks: TimelineTicks,
    pub(super) batch_scheduled_us: u64,
    pub(super) authored_batch_scheduled_ticks: TimelineTicks,
    pub(super) authored_batch_scheduled_us: u64,
    pub(super) conflict_mask: u16,
    pub(super) dispatch_path: DispatchPath,
    pub(super) packet_mode: bool,
    pub(super) packet_masks: Option<sky_dispatch_win32::input::PhysicalPacket>,
    pub(super) scan_batch: ScanCodeBatch,
}

/// `Err(None)` indicates an unrecoverable terminal step; `Ok(None)` means the
/// coordinator offered no authored work for this epoch (worker should advance
/// the wait deadline instead).
pub(super) type BatchViewResult = Result<Option<AuthoredBatchView>, DispatchStep>;

pub(crate) use authored::dispatch_authored_packet;
pub(crate) use release::{PendingReleaseContext, dispatch_due_pending_releases};

use super::super::{ActionKind, LatencyClass, QpcTicks, TimelineTicks};
pub(super) use super::publish_backend_metrics;
use super::{DispatchPath, planning::NextDispatchPlan};
use sky_dispatch_core::coordinator::PreparedBatch;
use sky_dispatch_core::model::ScanCodeBatch;
