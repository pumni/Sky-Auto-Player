use super::*;

mod diagnostics;
mod presentation;
mod retirement;
mod session;
mod timing_advisory;

pub(super) use diagnostics::publish_diagnostics_snapshot_inner_cancellable;
#[cfg(test)]
pub(super) use diagnostics::publish_final_diagnostics_snapshot_for_active;
#[cfg(test)]
pub(super) use diagnostics::{
    DiagnosticsBackendHealth, NativeDiagnosticsSample, diagnostics_backend_status,
    publish_diagnostics_snapshot_for_active, sender_sample_summary,
};
#[cfg(test)]
pub(super) use presentation::playback_health_from_snapshot;
pub(super) use presentation::{
    dry_run_elapsed, finalize_active_presentation, publish_active_playback_state,
    publish_playback_state, set_playback_state,
};
#[cfg(all(test, feature = "tauri-test"))]
pub(super) use presentation::{native_event_delivery_failure, publish_playback_snapshot};
pub(super) use retirement::{
    DryRunTerminalRequest, PlaybackActivation, PlaybackHandoffDecision, TerminalReleaseEvidence,
    TerminalRetirement, WorkerQuiescenceProof, decide_playback_handoff, physical_startup_failure,
    publish_terminal_event, release_terminal_ownership, release_terminal_resources,
    terminal_presentation_completion,
};
#[cfg(test)]
pub(super) use retirement::{
    publish_retirement_barrier, publish_terminal_poll_result, terminal_publication_after_worker,
    terminal_release_evidence, terminal_release_evidence_after_startup,
};
pub(super) use session::{
    cleanup_failed_event_delivery, is_failure_status, opaque_native_id, playback_event_state,
    settings_fingerprint, terminal_success_outcome, validate_playback_start_request,
    validate_public_playback_start_request,
};
#[cfg(test)]
pub(super) use session::{
    plan_fingerprint, retain_prepared_capacity, sender_trace_export_json,
    target_integrity_startup_failure,
};

/// Application-side playback control plane.  The realtime worker remains in
/// `sky_player`; this service owns prepared-plan admission, the single active
/// session rule, and the bounded supervisor loop around that worker.
pub(super) struct NativePlaybackService {
    pub(super) prepared: Mutex<VecDeque<(String, NativePreparedPlan)>>,
    pub(super) active: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    pub(super) last_terminal: Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    pub(super) sender_trace_state: Arc<Mutex<SenderTraceState>>,
    pub(super) diagnostics_gate: Arc<DiagnosticsPublicationGate>,
    pub(super) prepare_publication: Mutex<PreparePublicationState>,
    pub(super) activity: ActivityCoordinator,
    #[cfg(test)]
    pub(super) startup_failure: Option<String>,
    #[cfg(test)]
    pub(super) settings_invalidation_count: AtomicU64,
    #[cfg(test)]
    pub(super) prepare_before_publication_test_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    pub(super) prepare_before_model_test_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(all(test, feature = "tauri-test"))]
    pub(super) monitor_progress_test_hook: Option<Arc<dyn Fn(bool) + Send + Sync>>,
    #[cfg(all(test, feature = "tauri-test"))]
    pub(super) presentation_test_hook: Option<PlaybackPresentationTestHook>,
    #[cfg(test)]
    pub(super) monitor_join_test_hook: Option<PlaybackJoinTestHook>,
    #[cfg(test)]
    pub(super) supervisor_spawn_failure: bool,
    #[cfg(all(test, feature = "tauri-test"))]
    pub(super) test_player: Option<Arc<NativeDispatchSession>>,
    #[cfg(all(test, feature = "tauri-test"))]
    pub(super) before_arm_test_hook: Option<PlaybackBeforeArmTestHook>,
    #[cfg(all(test, feature = "tauri-test"))]
    pub(super) monitor_retired_test_hook: Option<PlaybackRetiredTestHook>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CompletedSenderTrace {
    pub(super) session_id: String,
    pub(super) song_id: String,
    pub(super) song_title: String,
    pub(super) plan_fingerprint: String,
    pub(super) trace_json: String,
}

#[derive(Default)]
pub(super) struct SenderTraceState {
    pub(super) collecting_session_id: Option<String>,
    pub(super) completed: Option<CompletedSenderTrace>,
}

const DIAGNOSTICS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct DiagnosticsPublicationState {
    pub(super) enabled: bool,
    pub(super) epoch: u64,
    pub(super) sequence: u64,
    pub(super) last_emit_at: Option<Instant>,
}

pub(super) struct DiagnosticsPublicationGate {
    pub(super) state: Mutex<DiagnosticsPublicationState>,
}

#[derive(Clone)]
pub(super) struct NativePreparedPlan {
    pub(super) song_id: String,
    pub(super) generation: u64,
    pub(super) settings_fingerprint: String,
    pub(super) song: Song,
    pub(super) source_path: PathBuf,
    pub(super) source_hash: String,
    pub(super) dto: PreparedPlaybackDto,
    pub(super) variants: HashMap<PlaybackDecision, NativePlaybackVariant>,
}

#[derive(Clone)]
pub(super) struct NativePlaybackVariant {
    pub(super) config: PlaybackConfigDto,
    pub(super) schedule: ScheduleMetadata,
    pub(super) fingerprint: String,
    pub(super) timing_policy: MaterializedTimingPolicy,
    pub(super) timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
}

pub(super) struct NativePreparedInput {
    pub(super) song_id: String,
    pub(super) generation: u64,
    pub(super) config: PlaybackConfigDto,
    pub(super) song: Song,
    pub(super) sheet_analysis: Option<crate::commands::SheetAnalysisDto>,
    pub(super) schedule: ScheduleMetadata,
    pub(super) risk: RiskReport,
    pub(super) timing_policy: MaterializedTimingPolicy,
    pub(super) timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
    pub(super) settings_fingerprint: String,
    pub(super) publication_fingerprint: String,
    pub(super) source_path: PathBuf,
    pub(super) source_hash: String,
    pub(super) plan_identity: String,
    pub(super) plan_source: Option<super::NativeSheetSource>,
    pub(super) source_epoch: u64,
}

/// Prepare requests, source invalidation, and cache publication share this
/// short-lived gate. Expensive compilation and advisory simulation never run
/// while it is held.
#[derive(Default)]
pub(super) struct PreparePublicationState {
    pub(super) active_request: Option<Arc<AtomicBool>>,
    pub(super) source_epoch: u64,
    pub(super) catalog_generation: Option<u64>,
    pub(super) settings_fingerprint: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(all(test, feature = "tauri-test"))]
pub(super) enum PlaybackPresentationStage {
    BeforeWorkSelection,
    AfterWorkSelection,
    SnapshotProjection,
    DiagnosticsProjection,
}

#[derive(Clone, Copy)]
pub(super) struct PlaybackPresentationFrame {
    pub(super) order: u64,
    pub(super) elapsed_us: u64,
    pub(super) pre_roll_remaining_us: u64,
    pub(super) status: EnginePollStatus,
}

pub(super) enum PlaybackPresentationMessage {
    State {
        order: u64,
        state: PlaybackEventState,
        message: Option<String>,
        outcome: Option<String>,
    },
    Finalize {
        order: u64,
        finalization: PlaybackPresentationFinalization,
        completed: PlaybackPresentationCompletion,
    },
}

pub(super) struct PlaybackPresentationFinalization {
    pub(super) state: PlaybackEventState,
    pub(super) message: Option<String>,
    pub(super) outcome: Option<String>,
    pub(super) publication: PlaybackTerminalPublication,
}

#[derive(Default)]
pub(super) struct PlaybackPresentationQueue {
    pub(super) latest_snapshot: Mutex<Option<PlaybackPresentationFrame>>,
    pub(super) next_order: AtomicU64,
    pub(super) snapshot_coalesced: AtomicU64,
    pub(super) snapshot_dropped: AtomicU64,
    pub(super) closed: AtomicBool,
    pub(super) failed: AtomicBool,
    pub(super) failure_reason: Mutex<Option<String>>,
    pub(super) worker_thread: OnceLock<thread::Thread>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PlaybackPresentationMetrics {
    pub(super) snapshot_coalesced: u64,
    pub(super) snapshot_dropped: u64,
}

pub(super) struct NativePlaybackPresentation {
    pub(super) critical: mpsc::SyncSender<PlaybackPresentationMessage>,
    pub(super) queue: Arc<PlaybackPresentationQueue>,
    pub(super) worker: Mutex<Option<thread::JoinHandle<()>>>,
}

pub(super) struct NativeActivePlayback {
    pub(super) activity: ActivityCoordinator,
    pub(super) session_id: String,
    pub(super) prepared_id: String,
    pub(super) song_id: String,
    pub(super) title: String,
    pub(super) total_us: u64,
    pub(super) config: PlaybackConfigDto,
    pub(super) timing_policy: MaterializedTimingPolicy,
    pub(super) timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
    pub(super) plan_fingerprint: String,
    pub(super) physical: bool,
    pub(super) activity_lease: Mutex<Option<PhysicalActivityLease>>,
    pub(super) target_hwnd: Option<isize>,
    pub(super) state: Mutex<PlaybackSessionState>,
    pub(super) pending: Mutex<Option<PlaybackPendingControl>>,
    pub(super) player: Option<Arc<NativeDispatchSession>>,
    pub(super) power_lifecycle: Arc<PowerLifecycleDiagnostics>,
    pub(super) power_request: Mutex<Option<SystemRequiredPowerRequest>>,
    pub(super) suspend_resume_registration: Mutex<Option<SuspendResumeRegistration>>,
    pub(super) started_at: Instant,
    pub(super) paused_since: Mutex<Option<Instant>>,
    pub(super) paused_total: Mutex<Duration>,
    pub(super) stop_requested: AtomicBool,
    pub(super) dry_run_terminal_request: AtomicU8,
    pub(super) done: AtomicBool,
    pub(super) worker_quiescent: AtomicBool,
    pub(super) release_verified: AtomicBool,
    pub(super) sequence: AtomicU64,
    pub(super) event_delivery_failure: Arc<AtomicBool>,
    pub(super) presentation: OnceLock<Arc<NativePlaybackPresentation>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(super) struct NativePlaybackPrepareRequest {
    pub(super) song_id: String,
    pub(super) generation: u64,
    pub(super) config: PlaybackConfigDto,
    pub(super) conversion_mode: Option<crate::commands::SheetConversionModeDto>,
    pub(super) target_pitch: Option<crate::commands::SheetPitchDto>,
    pub(super) conversion_confirmation_fingerprint: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(super) struct NativePlaybackStartRequest {
    pub(super) prepared_id: String,
    pub(super) decisions: Vec<PlaybackDecisionAcceptanceDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(super) struct NativePlaybackSessionRequest {
    pub(super) session_id: String,
}

impl NativePlaybackPrepareRequest {
    pub(super) fn into_public(self) -> PlaybackPrepareRequest {
        PlaybackPrepareRequest {
            song_id: self.song_id,
            generation: self.generation,
            config: self.config,
            conversion_mode: self.conversion_mode,
            target_pitch: self.target_pitch,
            conversion_confirmation_fingerprint: self.conversion_confirmation_fingerprint,
        }
    }
}

impl NativePlaybackStartRequest {
    pub(super) fn into_public(self) -> PlaybackStartRequest {
        PlaybackStartRequest {
            prepared_id: self.prepared_id,
            decisions: self.decisions,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PlaybackTerminalPublication {
    Finished { outcome: String, message: String },
    Failed { code: String, message: String },
}
