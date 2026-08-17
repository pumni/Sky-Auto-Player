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
pub(crate) mod observation;
pub(crate) mod observer;
pub(crate) mod timing;

/// Outcome of one authored packet dispatch step.
#[derive(Debug)]
pub enum DispatchStep {
    NoWork,
    Dispatched,
    Continue,
    Terminate(String),
    TerminateStatic(&'static str),
}

pub(crate) struct AuthoredPacketContext<'a> {
    pub(crate) dispatch_plan: &'a NextDispatchPlan,
    pub(crate) effective_now_ticks: TimelineTicks,
    pub(crate) now_ticks: QpcTicks,
    pub(crate) physical_target_qpc: QpcTicks,
    pub(crate) startup_target_selected: bool,
    pub(crate) focus_loss_fault: bool,
    pub(crate) interrupt: &'a sky_dispatch_win32::event::OwnedEvent,
    pub(crate) supervisor_heartbeat_ticks: &'a std::sync::atomic::AtomicU64,
    pub(crate) lease_timeout_ticks: DurationTicks,
    /// Test-only direct-boundary admission for frozen-plan correctness tests.
    /// This field and its branch are absent from production builds.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) test_direct_boundary: bool,
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
    pub(crate) commit: PhysicalCommit,
}

/// `Err(None)` indicates an unrecoverable terminal step; `Ok(None)` means the
/// coordinator offered no authored work for this epoch (worker should advance
/// the wait deadline instead).
pub(super) type BatchViewResult = Result<Option<AuthoredBatchView>, DispatchStep>;

pub(crate) use authored::dispatch_authored_packet;
#[cfg(test)]
pub(crate) use authored::handle_final_focus_loss;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use observation::DispatchObservation;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use observer::drain_one_observer;
pub(crate) use observer::{ObserverRuntime, PendingObservationQueue, dispatch_stale_packet};

use super::super::{ActionKind, DurationTicks, QpcClock, QpcTicks, TimelineTicks, TrackedKeyState};
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

#[inline]
pub(crate) fn spin_and_send_prepared(
    qpc_clock: QpcClock,
    physical_target_qpc: QpcTicks,
    backend: &mut TrackedKeyState,
    prepared_packet: &sky_dispatch_win32::input::PreparedPhysicalPacket,
) -> Result<sky_dispatch_win32::input::SendTransactionOutcome, sky_dispatch_win32::clock::QpcError>
{
    loop {
        let now_ticks = qpc_clock.now()?;
        if now_ticks >= physical_target_qpc {
            #[cfg(any(test, feature = "test-support"))]
            return Ok(backend.send_prepared_physical_packet_with_start(prepared_packet, now_ticks));
            #[cfg(not(any(test, feature = "test-support")))]
            return Ok(backend.send_prepared_physical_packet(prepared_packet));
        }
        std::hint::spin_loop();
    }
}
