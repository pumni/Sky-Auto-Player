mod admission;
mod boot;
mod cleanup;
mod control;
#[cfg(not(any(test, feature = "test-support")))]
mod dispatch;
// `pub(crate)` under test / test-support so the deterministic harness can
// exercise diagnostic observer primitives and slow-observer hooks without
// exposing those test seams in production builds.
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod dispatch;
mod dispatch_loop;
#[cfg(not(any(test, feature = "test-support")))]
mod health;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod health;
mod orchestration;
mod planning;
mod startup;
mod timing;
mod wait;

#[cfg(test)]
pub(crate) use admission::final_control_admission_with_lease;
pub(crate) use admission::{
    DownAdmission, FinalControlAdmission, FinalControlSignals, FinalTargetSignals, TargetStamp,
    ensure_preflight_for_target, final_control_admission_at, final_control_precheck,
    final_down_target_admission, focus_matches, focus_matches_hwnd, load_target_stamp,
    target_stamp_still_current,
};
use cleanup::{
    FinalizeInput, FinalizePublication, FinalizeResources, FinalizeSignals, FinalizeState,
    FinalizeTiming, finalize_worker,
};
pub(crate) use cleanup::{
    cancel_coordinator_or_terminal, describe_release_outcome, record_termination_error,
    release_runtime_outcome, release_state_verified, suspend_live_input,
};
use control::{
    CommandControl, CommandControlClock, CommandControlInput, CommandControlMetrics,
    CommandControlRuntime, CommandControlSignals, process_command_control,
};
pub(crate) use dispatch::ObserverRuntime;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch::drain_one_observer;
#[cfg(test)]
pub(crate) use dispatch::handle_final_focus_loss;
pub(crate) use dispatch::hold_forensics::ProductionHoldForensics;
pub(super) use dispatch::{
    AuthoredPacketContext, DispatchStep, DownBoundaryState, PhysicalBoundaryStamp,
    dispatch_authored_packet, dispatch_stale_packet,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_loop::dispatch_due_from_plan;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_loop::preflight_prepared_plan;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub(crate) use dispatch_loop::{preroll_manual_pause_cancels, startup_focus_loss_is_terminal};

#[cfg(any(test, feature = "test-support"))]
#[cfg(test)]
pub(crate) use health::HealthWindowPolicy;
#[cfg(test)]
pub(crate) use health::record_input_path_health;
pub(crate) use health::{
    DispatchHealthObservation, DispatchHealthOptions, DispatchPath, HEALTH_WINDOW_CAPACITY,
    HealthWindow, focus_gate_matches, observe_dispatch_health, observe_wait_health,
    publish_backend_metrics, record_lateness, record_sendinput_pre_call_lateness,
};
#[cfg(any(test, feature = "test-support"))]
pub use planning::NextDispatchPlan;
pub(crate) use planning::TargetProof;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use planning::plan_next_dispatch;
#[cfg(test)]
pub(crate) use planning::plan_structure_is_valid;
pub(crate) use planning::{PlanningInput, plan_next_dispatch_projected};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use sky_dispatch_win32::wait::WaitResult;
pub(crate) use startup::WorkerSchedulingGuards;
use startup::{StartupResources, initialize_startup};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use timing::derive_spin_threshold_us;
#[cfg(test)]
pub(crate) use timing::{
    adjust_spin_threshold, anchored_dispatch_target_ticks, anchored_dispatch_target_ticks_typed,
    deadline_target_ticks, exact_sender_durations,
};
pub(crate) use timing::{
    lease_bounded_ticks, signed_delta, signed_ticks_to_us, signed_timeline_delta_ticks,
    supervisor_lease_expired, wait_failure_message, wake_lateness_ticks,
};
pub(crate) use wait::{
    WaitBoundary, WaitBoundaryInput, WaitDeadline, WaitMutable, WaitSignals, WaitTiming,
    record_wait_failure, wait_for_next_boundary, wait_to_precision_boundary,
};

use super::shared::SessionShared;
use super::*;
#[cfg(feature = "test-support")]
use sky_dispatch_core::coordinator::AuthoredPreparationEvidence;
use sky_dispatch_core::model::RuntimeSchedule;
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering};

/// Test-only accounting for the immutable preparation boundary.
///
/// In production this is a zero-sized no-op, so the proof instrumentation
/// cannot add atomics or branches to the real-time path.
#[derive(Default)]
pub(crate) struct DispatchPreparationProbe {
    #[cfg(any(test, feature = "test-support"))]
    packet_header_reads: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    expected_up_intents: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    expected_down_intents: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    up_intent_visits: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    down_intent_visits: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    secondary_batch_visits: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    secondary_batch_visit_bound: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    registry_lookups: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    view_packet_calls: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    commit_freeze_calls: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    conflict_calls: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    input_build_calls: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    preflight_calls: AtomicU64,
}

impl DispatchPreparationProbe {
    #[inline]
    #[cfg(feature = "test-support")]
    pub(crate) fn record_authored_preparation(&self, evidence: AuthoredPreparationEvidence) {
        self.packet_header_reads
            .fetch_add(evidence.packet_header_reads, Ordering::Relaxed);
        self.expected_up_intents
            .fetch_add(evidence.expected_up_intents, Ordering::Relaxed);
        self.expected_down_intents
            .fetch_add(evidence.expected_down_intents, Ordering::Relaxed);
        self.up_intent_visits
            .fetch_add(evidence.up_intent_visits, Ordering::Relaxed);
        self.down_intent_visits
            .fetch_add(evidence.down_intent_visits, Ordering::Relaxed);
        self.secondary_batch_visits
            .fetch_add(evidence.secondary_batch_visits, Ordering::Relaxed);
        self.secondary_batch_visit_bound
            .fetch_add(evidence.secondary_batch_visit_bound, Ordering::Relaxed);
        self.registry_lookups
            .fetch_add(evidence.registry_lookups, Ordering::Relaxed);
        self.view_packet_calls
            .fetch_add(evidence.view_packet_calls, Ordering::Relaxed);
        self.commit_freeze_calls
            .fetch_add(evidence.commit_freeze_calls, Ordering::Relaxed);
    }

    #[inline]
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn record_packet_view(&self) {
        self.view_packet_calls.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn record_conflict(&self) {
        #[cfg(any(test, feature = "test-support"))]
        self.conflict_calls.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn record_input_build(&self) {
        #[cfg(any(test, feature = "test-support"))]
        self.input_build_calls.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn record_preflight(&self) {
        #[cfg(any(test, feature = "test-support"))]
        self.preflight_calls.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn counts(&self) -> PreparationCounts {
        PreparationCounts {
            packet_header_reads: self.packet_header_reads.load(Ordering::Relaxed),
            expected_up_intents: self.expected_up_intents.load(Ordering::Relaxed),
            expected_down_intents: self.expected_down_intents.load(Ordering::Relaxed),
            up_intent_visits: self.up_intent_visits.load(Ordering::Relaxed),
            down_intent_visits: self.down_intent_visits.load(Ordering::Relaxed),
            secondary_batch_visits: self.secondary_batch_visits.load(Ordering::Relaxed),
            secondary_batch_visit_bound: self.secondary_batch_visit_bound.load(Ordering::Relaxed),
            registry_lookups: self.registry_lookups.load(Ordering::Relaxed),
            view_packet_calls: self.view_packet_calls.load(Ordering::Relaxed),
            commit_freeze_calls: self.commit_freeze_calls.load(Ordering::Relaxed),
            conflict_calls: self.conflict_calls.load(Ordering::Relaxed),
            input_build_calls: self.input_build_calls.load(Ordering::Relaxed),
            preflight_calls: self.preflight_calls.load(Ordering::Relaxed),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn reset(&self) {
        self.packet_header_reads.store(0, Ordering::Relaxed);
        self.expected_up_intents.store(0, Ordering::Relaxed);
        self.expected_down_intents.store(0, Ordering::Relaxed);
        self.up_intent_visits.store(0, Ordering::Relaxed);
        self.down_intent_visits.store(0, Ordering::Relaxed);
        self.secondary_batch_visits.store(0, Ordering::Relaxed);
        self.secondary_batch_visit_bound.store(0, Ordering::Relaxed);
        self.registry_lookups.store(0, Ordering::Relaxed);
        self.view_packet_calls.store(0, Ordering::Relaxed);
        self.commit_freeze_calls.store(0, Ordering::Relaxed);
        self.conflict_calls.store(0, Ordering::Relaxed);
        self.input_build_calls.store(0, Ordering::Relaxed);
        self.preflight_calls.store(0, Ordering::Relaxed);
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparationCounts {
    pub packet_header_reads: u64,
    pub expected_up_intents: u64,
    pub expected_down_intents: u64,
    pub up_intent_visits: u64,
    pub down_intent_visits: u64,
    pub secondary_batch_visits: u64,
    pub secondary_batch_visit_bound: u64,
    pub registry_lookups: u64,
    pub view_packet_calls: u64,
    pub commit_freeze_calls: u64,
    pub conflict_calls: u64,
    pub input_build_calls: u64,
    pub preflight_calls: u64,
}

/// Mutable state owned exclusively by the worker thread.
///
/// This state deliberately lives outside the panic boundary so the worker's
/// backend, coordinator, and telemetry can still be finalized after an
/// injected or unexpected panic.
#[derive(Default)]
pub(crate) struct WorkerRuntime {
    pub(crate) preparation_probe: DispatchPreparationProbe,
    pub(crate) production_forensics: ProductionHoldForensics,
    verified_target: Option<TargetStamp>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) startup_ordering_hook: Option<Arc<StartupOrderingHook>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) restore_race_hook: Option<super::config::RestoreRaceHook>,
    focus_restore_started_ticks: Option<QpcTicks>,
    last_dispatch_deadline_wake_qpc: Option<QpcTicks>,
    /// Musical Down admission state. Up-only safety sends never mutate this
    /// state; authorization is tied to the exact frozen authored boundary.
    pub(crate) down_boundary_state: DownBoundaryState,
    pub(crate) last_dispatch_was_missed_down: bool,
    future_physical_wait_target_qpc: Option<QpcTicks>,
    last_dispatch_deadline_target_qpc: Option<QpcTicks>,
    pub(crate) force_full_cleanup: bool,
    pub(crate) terminal_error: Option<String>,
    focus_loss_fault_injected: bool,
    /// True only after the first successful authored musical commit. A
    /// final foreground mismatch before that point is startup failure, not a
    /// pause/rebase event.
    pub(crate) musical_physical_commit_started: bool,
    allow_pre_epoch_startup_dispatch: bool,
    pending_wait_observation: Option<wait::WaitObservation>,
    chord_integrity_lost: u64,
}

impl WorkerRuntime {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn create_test_runtime(verified_target: Option<TargetStamp>) -> Self {
        Self {
            verified_target,
            ..Self::default()
        }
    }

    #[allow(dead_code)]
    pub(crate) fn chord_integrity_lost_count(&self) -> u64 {
        self.chord_integrity_lost
    }

    #[allow(dead_code)]
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn set_deadline_wake_qpc_for_test(&mut self, ticks: Option<QpcTicks>) {
        self.last_dispatch_deadline_wake_qpc = ticks;
        self.last_dispatch_deadline_target_qpc = ticks;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn set_deadline_wait_evidence_for_test(
        &mut self,
        wake_qpc: Option<QpcTicks>,
        target_qpc: Option<QpcTicks>,
    ) {
        self.last_dispatch_deadline_wake_qpc = wake_qpc;
        self.last_dispatch_deadline_target_qpc = target_qpc;
    }

    #[inline]
    pub(crate) fn observe_future_down_boundary(&mut self, boundary: PhysicalBoundaryStamp) {
        if self.down_boundary_state != DownBoundaryState::FutureAuthorized(boundary) {
            self.down_boundary_state = DownBoundaryState::FutureAuthorized(boundary);
        }
    }

    #[inline]
    pub(crate) fn authorize_down_boundary(&mut self, boundary: PhysicalBoundaryStamp) -> bool {
        self.down_boundary_state.authorization() == Some(boundary)
    }

    #[inline]
    pub(crate) fn mark_down_commit_started(&mut self, late_rescue_consumed: bool) {
        self.down_boundary_state = DownBoundaryState::AwaitingFuture {
            late_rescue_available: !late_rescue_consumed,
        };
    }

    #[inline]
    pub(crate) fn mark_down_boundary_missed(&mut self) {
        if self.down_boundary_state.awaiting_future() {
            self.down_boundary_state = DownBoundaryState::AwaitingFuture {
                late_rescue_available: false,
            };
        }
    }

    #[inline]
    pub(crate) fn invalidate_down_authorization(&mut self) {
        if self.down_boundary_state.awaiting_future() {
            self.down_boundary_state = DownBoundaryState::AwaitingFuture {
                late_rescue_available: false,
            };
        }
    }

    #[inline]
    pub(crate) fn try_consume_late_discovery_rescue(
        &mut self,
        lateness: DurationTicks,
        grace: DurationTicks,
    ) -> bool {
        if !self.down_boundary_state.late_rescue_available() || lateness > grace {
            return false;
        }
        self.down_boundary_state = DownBoundaryState::AwaitingFuture {
            late_rescue_available: false,
        };
        true
    }

    #[inline]
    pub(crate) fn consume_late_discovery_rescue_credit(&mut self) {
        if self.down_boundary_state.late_rescue_available() {
            self.down_boundary_state = DownBoundaryState::AwaitingFuture {
                late_rescue_available: false,
            };
        }
    }

    /// Model `WaitBoundary::Due { wait_result: None }` after a future plan
    /// was classified but before the waiter could block. The timing observer
    /// evidence is cleared; exact Down authorization is intentionally kept in
    /// `down_boundary_state`.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn record_due_without_wait_for_test(&mut self) {
        self.last_dispatch_deadline_wake_qpc = None;
        self.last_dispatch_deadline_target_qpc = None;
        self.future_physical_wait_target_qpc = None;
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
    pub(super) down_late_grace_ticks: DurationTicks,
    pub(super) strict_down_completion_late_ticks: DurationTicks,
    pub(super) strict_up_completion_late_ticks: DurationTicks,
    pub(super) admission_guard_ticks: DurationTicks,
    pub(super) focus_restore_grace_ticks: DurationTicks,
    pub(super) paused_poll_ticks: DurationTicks,
    pub(crate) lease_timeout_ticks: DurationTicks,
    pub(crate) effective_spin_threshold_ticks: DurationTicks,
    /// First QPC tick whose floored public duration is strictly over the
    /// corresponding pre-call lateness bucket.  These are converted once at
    /// worker admission so the physical send path only compares integers.
    pub(super) pre_call_2ms_ticks: DurationTicks,
    pub(super) pre_call_5ms_ticks: DurationTicks,
    pub(super) pre_call_10ms_ticks: DurationTicks,
    pub(super) start_wall_time_us: u64,
    pub(super) start_thread_cpu_us: u64,
    pub(super) start_process_cpu_us: u64,
    pub(super) last_cpu_metrics_sample_us: u64,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkerTimingState {
    pub(crate) fn create_test_timing() -> Self {
        Self {
            strict_timing: false,
            down_late_grace_ticks: DurationTicks::ZERO,
            strict_down_completion_late_ticks: DurationTicks::ZERO,
            strict_up_completion_late_ticks: DurationTicks::ZERO,
            admission_guard_ticks: DurationTicks::ZERO,
            focus_restore_grace_ticks: DurationTicks::ZERO,
            paused_poll_ticks: DurationTicks::ZERO,
            lease_timeout_ticks: DurationTicks::ZERO,
            effective_spin_threshold_ticks: DurationTicks::ZERO,
            // Test harnesses replace these with the captured clock-domain
            // values when they exercise lateness buckets.  MAX keeps a
            // synthetic timing state from classifying every sample as late.
            pre_call_2ms_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_5ms_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_10ms_ticks: DurationTicks::from_raw(u64::MAX),
            start_wall_time_us: 0,
            start_thread_cpu_us: 0,
            start_process_cpu_us: 0,
            last_cpu_metrics_sample_us: 0,
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
    pub(super) coordinator: RuntimeDispatchCoordinator,
    pub(super) playback: PlaybackClockState,
    pub(super) telemetry: Arc<parking_lot::Mutex<TelemetryCollector>>,
    pub(super) scheduling: WorkerSchedulingGuards,
}

#[derive(Default)]
pub(super) struct WorkerCore {
    pub(super) resources: Option<WorkerResources>,
    pub(super) metrics: WorkerMetricsLocal,
    pub(super) health: Option<WorkerHealthState>,
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
    wall_now_us: u64,
) {
    if cpu_metrics_sample_due(
        wall_now_us,
        timing.last_cpu_metrics_sample_us,
        CPU_METRICS_SAMPLE_INTERVAL_US,
    ) {
        // The observer runs on a different thread.  Its sampling point can
        // update the process-wide value, but it must never publish that
        // thread's CPU time as the dispatch-worker CPU metric.  The worker
        // value is captured once during finalization on the owning thread.
        local_metrics.process_cpu_time_us =
            current_process_cpu_time_us().saturating_sub(timing.start_process_cpu_us);
        timing.last_cpu_metrics_sample_us = wall_now_us;
    }
    local_metrics.playback_wall_time_us = wall_now_us.saturating_sub(timing.start_wall_time_us);
    if local_metrics.playback_wall_time_us > 0 {
        local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
            / local_metrics.playback_wall_time_us as u128)
            as u64;
    }
}

pub(super) struct Worker<'a> {
    schedule: Option<RuntimeSchedule>,
    config: WorkerConfig,
    shared: &'a SessionShared,
    epoch_qpc: QpcTicks,
    core: WorkerCore,
}

impl<'a> Worker<'a> {
    pub(super) fn new(
        options: NativeSessionOptions,
        shared: &'a SessionShared,
        epoch_qpc: QpcTicks,
    ) -> Self {
        let NativeSessionOptions {
            schedule,
            backend,
            profile,
            timing,
            focus,
            wait,
            telemetry,
            priority,
            #[cfg(any(test, feature = "test-support"))]
            startup_ordering_hook,
            #[cfg(any(test, feature = "test-support"))]
            restore_race_hook,
            #[cfg(any(test, feature = "test-support"))]
                timer_lifecycle_context: _,
        } = options;
        #[cfg(any(test, feature = "test-support"))]
        let runtime = WorkerRuntime {
            startup_ordering_hook,
            restore_race_hook,
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
            },
            shared,
            epoch_qpc,
            core: WorkerCore {
                runtime,
                ..WorkerCore::default()
            },
        }
    }

    fn take_schedule(&mut self) -> Result<RuntimeSchedule, &'static str> {
        self.schedule
            .take()
            .ok_or("worker runtime schedule was already consumed")
    }

    #[cfg(test)]
    pub(super) fn take_schedule_for_test(&mut self) -> Result<RuntimeSchedule, &'static str> {
        self.take_schedule()
    }

    pub(super) fn run(mut self) -> u8 {
        orchestration::run(&mut self)
    }
}

#[cfg(test)]
mod observer_profile_tests {
    use super::{
        DownBoundaryState, PhysicalBoundaryStamp, QpcTicks, WorkerObserverState, WorkerRuntime,
    };
    use sky_dispatch_core::time::DurationTicks;

    fn boundary(source_action_index: u32) -> PhysicalBoundaryStamp {
        PhysicalBoundaryStamp {
            first_batch_index: 0,
            packet_index: 0,
            packet_batch_count: 1,
            source_action_index,
            up_mask: 0,
            down_mask: 1,
            physical_target_qpc: QpcTicks::from_raw(100),
        }
    }

    #[test]
    fn default_production_observer_state_has_no_queue_or_thread() {
        let state = WorkerObserverState::default();
        assert!(state.pending.is_none());
        assert!(state.runtime.is_none());
    }

    #[test]
    fn late_rescue_cutoff_matrix_is_inclusive_and_one_shot() {
        let grace = DurationTicks::from_raw(500);
        for lateness in [0, 1, 100, 499, 500] {
            let mut runtime = WorkerRuntime::create_test_runtime(None);
            runtime.mark_down_commit_started(false);
            assert!(
                runtime
                    .try_consume_late_discovery_rescue(DurationTicks::from_raw(lateness), grace,)
            );
            assert!(!runtime.try_consume_late_discovery_rescue(DurationTicks::from_raw(1), grace,));
            assert!(matches!(
                runtime.down_boundary_state,
                DownBoundaryState::AwaitingFuture {
                    late_rescue_available: false
                }
            ));
        }
        let mut beyond = WorkerRuntime::create_test_runtime(None);
        beyond.mark_down_commit_started(false);
        assert!(!beyond.try_consume_late_discovery_rescue(DurationTicks::from_raw(501), grace,));
    }

    #[test]
    fn future_observation_rearms_rescue_only_after_boundary_commit() {
        let mut runtime = WorkerRuntime::create_test_runtime(None);
        runtime.mark_down_commit_started(false);
        assert!(runtime.try_consume_late_discovery_rescue(
            DurationTicks::from_raw(1),
            DurationTicks::from_raw(500),
        ));
        let stamp = boundary(7);
        runtime.observe_future_down_boundary(stamp);
        assert!(runtime.authorize_down_boundary(stamp));
        assert!(!runtime.try_consume_late_discovery_rescue(
            DurationTicks::from_raw(1),
            DurationTicks::from_raw(500),
        ));
        runtime.mark_down_commit_started(false);
        assert!(runtime.try_consume_late_discovery_rescue(
            DurationTicks::from_raw(1),
            DurationTicks::from_raw(500),
        ));
    }

    #[test]
    fn randomized_rescue_sequence_never_catches_up_without_future_observation() {
        let grace = DurationTicks::from_raw(500);
        let mut runtime = WorkerRuntime::create_test_runtime(None);
        runtime.mark_down_commit_started(false);
        let mut state = 0x1357_9bdf_u64;
        let mut rescue_sent_since_future = false;
        for _ in 0..10_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            if state & 0b111 == 0 {
                runtime.observe_future_down_boundary(boundary((state >> 8) as u32));
                runtime.mark_down_commit_started(false);
                rescue_sent_since_future = false;
                continue;
            }
            let rescued = runtime
                .try_consume_late_discovery_rescue(DurationTicks::from_raw(state % 501), grace);
            if rescued {
                assert!(!rescue_sent_since_future);
                rescue_sent_since_future = true;
            }
        }
    }
}
