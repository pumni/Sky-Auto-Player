use super::config::StartupCheckpoint;
use super::test_support::MockSendInputCapture;
use super::test_support::command_timing::{
    CommandTimingError, CommandTimingLookup as PauseTimingLookup, PauseTimingPhase,
};
use super::{
    BackendConfig, CommandTimingResult, CommandTimingState, DispatchProfile, EnginePollStatus,
    FaultInjectionScript, FocusOptions, HealthWindow, HealthWindowPolicy, InjectedSendOutcome,
    NATIVE_TELEMETRY_SCHEMA_VERSION, NativeDispatchSession, NativeSessionOptions, PriorityOptions,
    RtTraceRecord, SharedMetrics, StartupOrderingHook, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN,
    TelemetryCollector, TelemetryMode, TelemetryOptions, TimingOptions, TraceContext,
    TraceDelivery, TraceTiming, WaitOptions, Worker, WorkerMetricsLocal,
    anchored_dispatch_target_ticks, cpu_metrics_sample_due, deadline_target_ticks,
    focus_gate_matches, record_input_path_health, record_termination_error, trace_outcome_code,
    try_publish_metrics,
};
use sky_dispatch_core::model::OnsetChordInput;
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::{
    QpcClock, QpcTicks, qpc_frequency, qpc_ticks_to_us, qpc_us_to_ticks,
};
use sky_dispatch_win32::input::{InstrumentKeyProfileSpec, PhysicalKey, PhysicalPacket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(super) use super::{NativeTelemetrySummary, TestWaitPolicy, config};

#[test]
fn native_session_rejects_prepared_plan_from_another_qpc_domain() {
    let schedule = support::startup_boundary_schedule();
    let prepared = super::super::adapter_support::PreparedDispatchSchedule::prepare(
        &schedule,
        std::num::NonZeroU64::new(1).expect("nonzero fixture QPC frequency"),
        0,
    )
    .expect("fixture plan is valid in its prepared domain");
    let mut options = support::test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.schedule = prepared;

    let result = super::NativeDispatchSession::new_with_power_endpoint(
        options,
        super::SystemPowerEndpoint::new().expect("test power endpoint"),
    );
    let error = match result {
        Ok(_) => panic!("a QPC-domain mismatch must be rejected before session creation"),
        Err(error) => error,
    };
    assert!(error.contains("prepared dispatch plan no longer matches the QPC domain"));
}

mod modifier_replan;
mod startup_admission;
mod supervisor_retirement;
pub(crate) mod support;
mod telemetry_metrics;
