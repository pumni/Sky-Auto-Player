mod admission;
mod boot;
mod cleanup;
mod control;
pub(crate) mod dispatch;
mod dispatch_loop;
#[cfg(not(any(test, feature = "test-support")))]
mod health;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod health;
mod orchestration;
mod startup;
mod timing;
mod wait;

pub(crate) use admission::{
    FinalControlSignals, FinalDownAdmissionResult, TargetStamp, ensure_preflight_for_target,
    enter_focus_pause, focus_matches, focus_matches_hwnd, load_target_stamp,
    target_stamp_still_current,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use cleanup::FinalizeTestObservation;
#[cfg(test)]
pub(crate) use cleanup::record_termination_error;
use cleanup::{
    FinalizeInput, FinalizePublication, FinalizeResources, FinalizeSignals, FinalizeState,
    FinalizeTiming, finalize_worker,
};
pub(crate) use cleanup::{describe_release_outcome, release_state_verified};
pub(crate) use control::TerminalTrigger;
use control::{
    CommandControl, CommandControlInput, CommandControlRuntime, CommandControlSignals,
    process_command_control,
};
pub(super) use dispatch::DispatchStep;
pub(crate) use dispatch::ObserverRuntime;
pub(crate) use dispatch::hold_forensics::ProductionHoldForensics;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_loop::DispatchAttemptOutcome;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_loop::apply_scheduler_resumable_lifecycle_transition_for_test;
#[cfg(test)]
pub(crate) use dispatch_loop::validate_resume_prerequisites;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_loop::{
    DispatchAttemptContext, DispatchControlSignals, dispatch_candidate, prepare_dispatch_candidate,
};
#[cfg(any(test, feature = "test-support"))]
pub use dispatch_loop::{DispatchCandidate, DispatchOutcome};
#[cfg(test)]
pub(crate) use dispatch_loop::{PreWaitAdmission, pre_wait_admission};

#[cfg(any(test, feature = "test-support"))]
#[cfg(test)]
pub(crate) use health::HealthWindowPolicy;
#[cfg(test)]
pub(crate) use health::record_input_path_health;
pub(crate) use health::{
    DispatchHealthObservation, DispatchHealthOptions, DispatchPath, HEALTH_WINDOW_CAPACITY,
    HealthWindow, focus_gate_matches, observe_dispatch_health, observe_wait_health,
    publish_backend_counters, publish_backend_metrics, record_lateness,
};
pub(crate) use startup::WorkerSchedulingGuards;
use startup::{StartupResources, initialize_startup};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use timing::signed_ticks_to_us;
#[cfg(test)]
pub(crate) use timing::{anchored_dispatch_target_ticks, deadline_target_ticks};
pub(crate) use timing::{
    signed_delta, signed_timeline_delta_ticks, supervisor_lease_expired, wait_failure_message,
    wake_lateness_ticks,
};
pub use wait::WaitObservation;
pub(crate) use wait::{
    WaitBoundary, WaitBoundaryInput, WaitDeadline, WaitMutable, WaitSignals, record_wait_failure,
    wait_for_next_boundary,
};

use super::shared::SessionShared;
use super::*;
#[cfg(any(test, feature = "test-support"))]
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn process_command_control_for_test(
    terminal_trigger: &mut Option<TerminalTrigger>,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    supervisor_expired: &crate::engine::shared::SupervisorLeaseState,
    system_power: &crate::engine::shared::SystemPowerState,
) -> bool {
    matches!(
        process_command_control(CommandControlInput {
            signals: CommandControlSignals {
                quit_requested,
                skip_requested,
                panic_requested,
                supervisor_expired,
                system_power,
            },
            runtime: CommandControlRuntime { terminal_trigger },
        }),
        CommandControl::Exit
    )
}

#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_worker_for_test(
    resources: WorkerResources,
    target: &crate::engine::shared::SessionTarget,
    skip_requested: &AtomicBool,
    quit_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    supervisor_expired: &crate::engine::shared::SupervisorLeaseState,
    system_power: &crate::engine::shared::SystemPowerState,
    metrics: &SharedMetrics,
    progress_clock: &super::shared::SharedProgressClock,
    worker_panicked: bool,
    terminal_trigger: Option<TerminalTrigger>,
    terminal_error: Option<String>,
    attempted_dispatch_anomaly: AttemptedDispatchAnomaly,
    cleanup_observation: &mut cleanup::FinalizeTestObservation,
) -> u8 {
    let telemetry_output = parking_lot::Mutex::new(None);
    let priority_acquired = parking_lot::Mutex::new(String::new());
    #[cfg(test)]
    let publish_count_before = metrics.publish_count.load(Ordering::Acquire);
    let result = catch_unwind(AssertUnwindSafe(|| {
        finalize_worker(FinalizeInput {
            resources: FinalizeResources {
                backend: resources.backend,
                scheduler: resources.scheduler,
                telemetry: resources.telemetry,
                playback: resources.playback,
                qpc_clock: resources.clock,
                scheduling: resources.scheduling,
                observer: None,
            },
            state: FinalizeState {
                worker_result: if worker_panicked {
                    Err(Box::new("test worker panic"))
                } else {
                    Ok(())
                },
                local_metrics: WorkerMetricsLocal::default(),
                pending_production_forensics: PendingProductionForensics::default(),
                abort_counts: HashMap::new(),
                terminal_error,
                secondary_errors: Vec::new(),
                attempted_dispatch_anomaly,
                last_published_error: None,
                terminal_trigger,
            },
            signals: FinalizeSignals {
                target,
                skip_requested,
                quit_requested,
                panic_requested,
                supervisor_expired,
                system_power,
                supervisor_expiry_reason_code: None,
            },
            publication: FinalizePublication {
                metrics,
                telemetry_output: &telemetry_output,
                priority_acquired: &priority_acquired,
                progress_clock,
            },
            timing: FinalizeTiming {
                start_wall_time_us: 0,
                start_thread_cpu_us: 0,
                start_process_cpu_us: 0,
            },
            cleanup_observation: Some(cleanup_observation),
        })
    }));
    #[cfg(test)]
    {
        cleanup_observation.terminal_metrics_publication_count = metrics
            .publish_count
            .load(Ordering::Acquire)
            .saturating_sub(publish_count_before);
    }
    cleanup_observation.generation_accounting = *metrics.generation_accounting.lock();
    match result {
        Ok(outcome) => outcome,
        Err(payload) => resume_unwind(payload),
    }
}

const MAX_ATTEMPTED_ANOMALY_RECORD_BYTES: usize = 512;

/// The first attempted sender anomaly is terminal for musical dispatch. Keep
/// only its bounded detail while preserving the existing terminal summary.
#[derive(Debug, Default)]
pub(crate) struct AttemptedDispatchAnomaly {
    recorded: bool,
    detail: Option<String>,
}

impl AttemptedDispatchAnomaly {
    pub(crate) fn record(&mut self, mut detail: String) {
        if self.recorded {
            return;
        }
        truncate_utf8(&mut detail, MAX_ATTEMPTED_ANOMALY_RECORD_BYTES);
        self.recorded = true;
        self.detail = Some(detail);
    }

    pub(crate) fn terminal_error_summary(&self) -> Option<String> {
        self.recorded.then(|| {
            format!(
                "post-call attempted transport anomaly: total=1 omitted=0 retained={}",
                usize::from(self.detail.is_some()),
            )
        })
    }

    pub(crate) fn append_detail_to(&mut self, destination: &mut Vec<String>) {
        if let Some(detail) = self.detail.take() {
            destination.push(detail);
        }
    }

    #[cfg(test)]
    pub(crate) fn is_recorded(&self) -> bool {
        self.recorded
    }

    #[cfg(test)]
    pub(crate) fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

fn truncate_utf8(value: &mut String, maximum_bytes: usize) {
    if value.len() <= maximum_bytes {
        return;
    }
    let mut boundary = maximum_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[cfg(test)]
mod attempted_dispatch_anomaly_tests {
    use super::{AttemptedDispatchAnomaly, MAX_ATTEMPTED_ANOMALY_RECORD_BYTES};

    #[test]
    fn retains_only_first_bounded_detail_and_preserves_terminal_latch() {
        let mut anomaly = AttemptedDispatchAnomaly::default();
        anomaly.record("x".repeat(MAX_ATTEMPTED_ANOMALY_RECORD_BYTES + 20));
        anomaly.record("later anomaly must not replace first detail".to_string());

        assert_eq!(
            anomaly.detail().map(str::len),
            Some(MAX_ATTEMPTED_ANOMALY_RECORD_BYTES)
        );
        assert_eq!(
            anomaly.terminal_error_summary().as_deref(),
            Some("post-call attempted transport anomaly: total=1 omitted=0 retained=1")
        );

        let mut secondary_errors = Vec::new();
        anomaly.append_detail_to(&mut secondary_errors);
        assert_eq!(secondary_errors.len(), 1);
        assert_eq!(
            anomaly.terminal_error_summary().as_deref(),
            Some("post-call attempted transport anomaly: total=1 omitted=0 retained=0")
        );

        let mut multibyte = AttemptedDispatchAnomaly::default();
        multibyte.record(format!("a{}", "é".repeat(300)));
        let retained = multibyte.detail().expect("bounded anomaly detail");
        assert_eq!(retained.len(), MAX_ATTEMPTED_ANOMALY_RECORD_BYTES - 1);
        assert!(retained.is_char_boundary(retained.len()));
    }
}

/// Mutable state owned exclusively by the worker thread.
///
/// This state deliberately lives outside the panic boundary so the worker's
/// backend, scheduler, and telemetry can still be finalized after an
/// injected or unexpected panic.
#[derive(Default)]
pub(crate) struct WorkerRuntime {
    pub(crate) production_forensics: ProductionHoldForensics,
    pending_production_forensics: Option<DeferredProductionPacketResult>,
    production_forensics_queue_overflowed: bool,
    verified_target: Option<TargetStamp>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) startup_ordering_hook: Option<Arc<StartupOrderingHook>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) final_gate_post_focus_race_hook: Option<super::config::FinalGateRaceHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) final_gate_post_lease_race_hook: Option<super::config::FinalGateRaceHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) final_gate_before_syscall_hook: Option<super::config::FinalGateBeforeSyscallHook>,
    #[cfg(test)]
    pub(crate) final_lease_qpc_sample_count_for_test: u64,
    #[cfg(test)]
    pub(crate) final_lease_qpc_sample_override_for_test:
        Option<Result<QpcTicks, sky_dispatch_win32::clock::QpcError>>,
    #[cfg(test)]
    pub(crate) final_admission_gate_hook_for_test:
        Option<Arc<dyn Fn(dispatch_loop::FinalAdmissionGateMarker) + Send + Sync>>,
    #[cfg(test)]
    pub(crate) precision_gate_qpc_sample_count_for_test: u8,
    #[cfg(test)]
    pub(crate) precision_gate_qpc_sample_failure_index_for_test: Option<u8>,
    focus_restore_started_ticks: Option<QpcTicks>,
    pub(crate) modifier_hold_started_ticks: Option<QpcTicks>,
    last_dispatch_deadline_wake_qpc: Option<QpcTicks>,
    pub(crate) pending_wait_observation: Option<WaitObservation>,
    /// Measured completion boundary of the most recent physical sender
    /// transaction, retained so sequential HOL evidence survives queue drops.
    pub(crate) last_physical_send_completion_qpc: Option<QpcTicks>,
    last_dispatch_deadline_target_qpc: Option<QpcTicks>,
    pub(crate) terminal_error: Option<String>,
    pub(crate) terminal_trigger: Option<TerminalTrigger>,
    pub(crate) manual_pause_suspension_pending: bool,
    /// True only after the first successful authored musical commit. A
    /// syscall attempt crosses the pre-roll boundary even when its transport
    /// evidence is anomalous; that onset is reconciled and never replayed.
    pub(crate) musical_physical_attempt_started: bool,
    /// The first attempted sender anomaly terminates musical dispatch; only
    /// bounded verified safety cleanup continues after that boundary.
    pub(crate) attempted_dispatch_anomaly: AttemptedDispatchAnomaly,
    pub(crate) allow_pre_epoch_startup_dispatch: bool,
}

/// Fixed-size packet evidence copied after journal commit and reduced at the
/// next non-precision worker boundary. At most one dispatch packet can be
/// pending because the worker drains before it begins another iteration.
#[derive(Clone, Copy)]
pub(crate) struct DeferredProductionPacketResult {
    packet: sky_dispatch_win32::input::PhysicalPacket,
    source_action_index: u32,
    target_qpc: QpcTicks,
    pre_call_qpc: Option<QpcTicks>,
    completion_qpc: QpcTicks,
    status: sky_dispatch_win32::input::SendTransactionStatus,
}

#[derive(Default)]
pub(crate) struct PendingProductionForensics {
    forensics: ProductionHoldForensics,
    packet: Option<DeferredProductionPacketResult>,
    queue_overflowed: bool,
}

impl PendingProductionForensics {
    pub(crate) fn drain_after_cleanup(&mut self, metrics: &mut WorkerMetricsLocal) {
        if let Some(evidence) = self.packet.take() {
            self.forensics.observe_packet_result(
                evidence.packet,
                evidence.source_action_index,
                evidence.target_qpc,
                evidence.pre_call_qpc,
                evidence.completion_qpc,
                evidence.status,
                metrics,
            );
        }
        if self.queue_overflowed {
            metrics.production_forensics_available = false;
        }
    }
}

impl WorkerRuntime {
    pub(crate) fn take_pending_production_forensics(&mut self) -> PendingProductionForensics {
        PendingProductionForensics {
            forensics: std::mem::take(&mut self.production_forensics),
            packet: self.pending_production_forensics.take(),
            queue_overflowed: self.production_forensics_queue_overflowed,
        }
    }

    pub(crate) fn defer_production_packet_result(
        &mut self,
        packet: sky_dispatch_win32::input::PhysicalPacket,
        source_action_index: u32,
        target_qpc: QpcTicks,
        pre_call_qpc: Option<QpcTicks>,
        completion_qpc: QpcTicks,
        status: sky_dispatch_win32::input::SendTransactionStatus,
    ) {
        let evidence = DeferredProductionPacketResult {
            packet,
            source_action_index,
            target_qpc,
            pre_call_qpc,
            completion_qpc,
            status,
        };
        if self
            .pending_production_forensics
            .replace(evidence)
            .is_some()
        {
            // Never report a complete production-forensics record after a
            // bounded queue overflow. Normal dispatch drains before another
            // send, so this is a fail-visible invariant guard.
            self.production_forensics_queue_overflowed = true;
        }
    }

    pub(crate) fn drain_production_forensics(&mut self, metrics: &mut WorkerMetricsLocal) {
        let mut pending = PendingProductionForensics {
            forensics: std::mem::take(&mut self.production_forensics),
            packet: self.pending_production_forensics.take(),
            queue_overflowed: self.production_forensics_queue_overflowed,
        };
        pending.drain_after_cleanup(metrics);
        self.production_forensics = pending.forensics;
        self.production_forensics_queue_overflowed = pending.queue_overflowed;
    }
}

impl WorkerRuntime {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn create_test_runtime(verified_target: Option<TargetStamp>) -> Self {
        Self {
            verified_target,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn verified_target_for_test(&self) -> Option<TargetStamp> {
        self.verified_target
    }
}

#[derive(Default)]
pub(super) struct WorkerErrorState {
    secondary: Vec<String>,
    last_published: Option<String>,
    abort_counts: HashMap<&'static str, u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct WorkerTimingState {
    pub(super) strict_timing: bool,
    pub(super) strict_down_completion_late_ticks: DurationTicks,
    pub(super) strict_up_completion_late_ticks: DurationTicks,
    pub(super) focus_restore_grace_ticks: DurationTicks,
    pub(crate) modifier_hold_timeout_ticks: DurationTicks,
    pub(crate) lease_timeout_ticks: DurationTicks,
    pub(crate) effective_spin_threshold_ticks: DurationTicks,
    /// First QPC tick whose floored public duration is strictly over the
    /// corresponding pre-call lateness bucket.  These are converted once at
    /// worker admission so the physical send path only compares integers.
    pub(super) pre_call_250us_ticks: DurationTicks,
    pub(super) pre_call_500us_ticks: DurationTicks,
    pub(super) pre_call_750us_ticks: DurationTicks,
    pub(super) pre_call_1000us_ticks: DurationTicks,
    pub(super) pre_call_1500us_ticks: DurationTicks,
    pub(super) pre_call_2000us_ticks: DurationTicks,
    pub(super) pre_call_2ms_ticks: DurationTicks,
    pub(super) pre_call_5ms_ticks: DurationTicks,
    pub(super) pre_call_10ms_ticks: DurationTicks,
    pub(super) cpu_metrics_sample_interval_ticks: DurationTicks,
    pub(super) start_wall_time_us: u64,
    pub(super) start_thread_cpu_us: u64,
    pub(super) start_process_cpu_us: u64,
    pub(super) last_cpu_metrics_sample_ticks: u64,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkerTimingState {
    pub(crate) fn create_test_timing() -> Self {
        Self {
            strict_timing: false,
            strict_down_completion_late_ticks: DurationTicks::ZERO,
            strict_up_completion_late_ticks: DurationTicks::ZERO,
            focus_restore_grace_ticks: DurationTicks::ZERO,
            modifier_hold_timeout_ticks: DurationTicks::from_raw(20_000_000),
            lease_timeout_ticks: DurationTicks::ZERO,
            effective_spin_threshold_ticks: DurationTicks::ZERO,
            // Test harnesses replace these with the captured clock-domain
            // values when they exercise lateness buckets.  MAX keeps a
            // synthetic timing state from classifying every sample as late.
            pre_call_250us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_500us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_750us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_1000us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_1500us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_2000us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_2ms_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_5ms_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_10ms_ticks: DurationTicks::from_raw(u64::MAX),
            cpu_metrics_sample_interval_ticks: DurationTicks::ZERO,
            start_wall_time_us: 0,
            start_thread_cpu_us: 0,
            start_process_cpu_us: 0,
            last_cpu_metrics_sample_ticks: 0,
        }
    }
}

/// Fixed observer guard converted to QPC ticks during worker admission.
pub(crate) struct WorkerHealthState {
    pub(super) options: DispatchHealthOptions,
    pub(super) sendinput_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) core_post_send_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) observer_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) wait_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
}

impl WorkerHealthState {
    pub(crate) fn new(options: DispatchHealthOptions) -> Self {
        Self {
            options,
            sendinput_window: HealthWindow::default(),
            core_post_send_window: HealthWindow::default(),
            observer_window: HealthWindow::default(),
            wait_window: HealthWindow::default(),
        }
    }
}

pub(crate) struct WorkerResources {
    pub(super) clock: QpcClock,
    pub(super) waiter: HybridWaiter,
    pub(super) backend: TrackedKeyState,
    pub(super) scheduler: sky_dispatch_core::executor::DispatchExecutor,
    pub(super) playback: PlaybackClockState,
    pub(super) telemetry: Arc<parking_lot::Mutex<TelemetryCollector>>,
    pub(super) scheduling: WorkerSchedulingGuards,
}

#[derive(Default)]
pub(super) struct WorkerCore {
    pub(super) resources: Option<WorkerResources>,
    pub(super) metrics: WorkerMetricsLocal,
    pub(super) timing: Option<WorkerTimingState>,
    pub(super) runtime: WorkerRuntime,
    pub(super) errors: WorkerErrorState,
    pub(super) observer: WorkerObserverState,
}

/// Deferred dispatch-observer state owned exclusively by the worker thread.
/// Production keeps both fields empty; strict diagnostics own the fixed queue
/// and consumer thread here, never in shared state.
#[derive(Default)]
pub(super) struct WorkerObserverState {
    pub(super) pending: Option<dispatch::PendingObservationQueue>,
    pub(super) runtime: Option<ObserverRuntime>,
}

pub(super) fn update_deferred_worker_metrics(
    local_metrics: &mut WorkerMetricsLocal,
    timing: &mut WorkerTimingState,
    qpc_clock: QpcClock,
    now_ticks: QpcTicks,
) {
    let Ok(wall_now_us) = qpc_clock.duration_to_us(DurationTicks::from_raw(now_ticks.as_u64()))
    else {
        return;
    };
    if cpu_metrics_sample_due(
        now_ticks.as_u64(),
        timing.last_cpu_metrics_sample_ticks,
        timing.cpu_metrics_sample_interval_ticks.as_u64(),
    ) {
        // The observer runs on a different thread.  Its sampling point can
        // update the process-wide value, but it must never publish that
        // thread's CPU time as the dispatch-worker CPU metric.  The worker
        // value is captured once during finalization on the owning thread.
        local_metrics.process_cpu_time_us =
            current_process_cpu_time_us().saturating_sub(timing.start_process_cpu_us);
        timing.last_cpu_metrics_sample_ticks = now_ticks.as_u64();
    }
    local_metrics.playback_wall_time_us = wall_now_us.saturating_sub(timing.start_wall_time_us);
    if local_metrics.idle_wake_count > 0 && local_metrics.playback_wall_time_us > 0 {
        local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
            / local_metrics.playback_wall_time_us as u128)
            as u64;
    }
}

pub(super) struct Worker<'a> {
    schedule: Option<sky_dispatch_core::executor::PreparedDispatchSchedule>,
    config: WorkerConfig,
    shared: &'a SessionShared,
    epoch_qpc: QpcTicks,
    instrument_key_profile: Option<sky_dispatch_win32::input::MaterializedInstrumentKeyProfile>,
    core: WorkerCore,
}

impl<'a> Worker<'a> {
    pub(super) fn new(
        admitted: super::config::AdmittedNativeSessionOptions,
        shared: &'a SessionShared,
    ) -> Self {
        let super::config::AdmittedNativeSessionOptions {
            options:
                NativeSessionOptions {
                    schedule,
                    backend,
                    profile,
                    timing,
                    focus,
                    wait,
                    telemetry,
                    priority,
                    #[cfg(any(test, feature = "test-support"))]
                    timing_observer_for_qualification,
                    #[cfg(any(test, feature = "test-support"))]
                    startup_ordering_hook,
                    #[cfg(any(test, feature = "test-support"))]
                        timer_lifecycle_context: _,
                    #[cfg(any(test, feature = "test-support"))]
                    prepared_packet_ambiguity_mask,
                    #[cfg(any(test, feature = "test-support"))]
                    preflight_user_held_mask,
                    #[cfg(any(test, feature = "test-support"))]
                    modifier_key_state_query_for_test,
                    instrument_key_profile: _,
                },
            instrument_key_profile,
        } = admitted;
        #[cfg(any(test, feature = "test-support"))]
        let runtime = WorkerRuntime {
            startup_ordering_hook,
            ..WorkerRuntime::default()
        };
        #[cfg(not(any(test, feature = "test-support")))]
        let runtime = WorkerRuntime::default();
        Self {
            schedule: Some(schedule),
            config: WorkerConfig {
                backend,
                profile,
                timing,
                focus,
                wait,
                telemetry,
                priority,
                #[cfg(any(test, feature = "test-support"))]
                timing_observer_for_qualification,
                #[cfg(any(test, feature = "test-support"))]
                prepared_packet_ambiguity_mask,
                #[cfg(any(test, feature = "test-support"))]
                preflight_user_held_mask,
                #[cfg(any(test, feature = "test-support"))]
                modifier_key_state_query_for_test,
            },
            shared,
            epoch_qpc: QpcTicks::ZERO,
            instrument_key_profile: Some(instrument_key_profile),
            core: WorkerCore {
                runtime,
                ..WorkerCore::default()
            },
        }
    }

    fn take_schedule(
        &mut self,
    ) -> Result<sky_dispatch_core::executor::PreparedDispatchSchedule, &'static str> {
        self.schedule
            .take()
            .ok_or("worker runtime plan was already consumed")
    }

    #[cfg(test)]
    pub(super) fn take_schedule_for_test(
        &mut self,
    ) -> Result<sky_dispatch_core::executor::PreparedDispatchSchedule, &'static str> {
        self.take_schedule()
    }

    pub(super) fn run(mut self) -> u8 {
        orchestration::run(&mut self)
    }
}
