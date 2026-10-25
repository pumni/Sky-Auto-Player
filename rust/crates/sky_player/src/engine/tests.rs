use super::config::StartupCheckpoint;
use super::shared::SessionTarget;
use super::telemetry::metrics::RecentLatencyRing;
use super::test_support::command_timing::{
    CommandTimingError, CommandTimingLookup as PauseTimingLookup, PauseTimingPhase,
};
use super::test_support::{MockSendInputCapture, ProductionDispatchTestHarness};
use super::{
    BackendConfig, CommandTimingResult, CommandTimingState, DispatchProfile, EnginePollStatus,
    FaultInjectionScript, FinalControlAdmission, FinalControlSignals, FinalDownVerdict,
    FinalTargetSignals, FocusOptions, HealthWindow, HealthWindowPolicy, InjectedSendOutcome,
    NativeDispatchSession, NativeSessionOptions, OwnerAdmissionFailure, PlatformSendResult,
    PriorityOptions, RtTraceRecord, SharedMetrics, StartupOrderingHook, TRACE_FLAG_SENT_FULL,
    TRACE_KIND_DOWN, TRACE_SEND_STATUS_NOT_ATTEMPTED, TargetStamp, TelemetryCollector,
    TelemetryMode, TelemetryOptions, TimestampProvenance, TimingOptions, TraceContext,
    TraceDelivery, TraceTiming, TrackedKeyState, WaitOptions, WakeErrorStats, Worker,
    WorkerMetricsLocal, adjust_spin_threshold, anchored_dispatch_target_ticks,
    cpu_metrics_sample_due, deadline_target_ticks, derive_spin_threshold_us,
    ensure_preflight_for_target, exact_sender_durations, final_control_precheck,
    final_down_target_admission, focus_gate_matches, focus_matches, focus_matches_hwnd,
    record_input_path_health, record_termination_error, release_runtime_outcome,
    signed_timeline_delta_ticks, supervisor_lease_expired, target_stamp_still_current,
    trace_outcome_code, trace_send_status_code, try_publish_metrics, wake_lateness_ticks,
};
use sky_app_core::timing::MaterializedTimingPolicy;
use sky_dispatch_core::clock::PauseReason;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::{
    DurationTicks, QpcClock, QpcTicks, qpc_frequency, qpc_ticks_to_us, qpc_us_to_ticks,
};
use sky_dispatch_win32::input::{
    InstrumentKeyProfileSpec, InstrumentPhysicalState, PacketRetryReason, PhysicalKey,
    PhysicalPacket, PreparedPhysicalPacket, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU16, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(super) use super::{
    CoordinatorError, EngineProgressSnapshot, EngineSnapshot, NativeTelemetrySummary,
    OUTCOME_ERROR, OUTCOME_QUIT, OUTCOME_SKIPPED, OUTCOME_SYSTEM_SUSPENDED, PlaybackClockState,
    TestWaitPolicy, config, session, shared, target, telemetry, test_support, worker,
};

mod lease_supervision;
mod native_dispatch;
mod pause_suspend;
mod preparation_dispatch;
mod release_cleanup;
mod startup_admission;
mod support;
mod target_focus_admission;
mod telemetry_metrics;
mod timing_wait;
