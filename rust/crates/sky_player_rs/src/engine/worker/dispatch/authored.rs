use super::super::super::{
    ActionKind, DurationTicks, HARD_LATE_ABORT_THRESHOLD_US, LatencyClass, PlaybackClockState,
    QpcClock, QpcTicks, RuntimeDispatchCoordinator, STRICT_SATURATION_ABORT_STREAK, SharedMetrics,
    TRACE_KIND_DOWN, TRACE_KIND_UP, TelemetryCollector, TimelineTicks, TrackedKeyState,
    try_publish_metrics,
};
use super::super::{
    DispatchPath, DownAdmission, FinalControlAdmission, FinalControlSignals, FinalTargetSignals,
    WorkerConfig, WorkerHealthState, WorkerMetricsLocal, WorkerResources, WorkerRuntime,
    WorkerTimingState, ensure_preflight_for_target, final_control_admission_with_lease,
    final_down_target_admission, focus_matches, load_target_stamp, suspend_live_input,
    target_stamp_still_current,
};
use super::observer::{
    commit_suppressed_up_request, publisher_down_send_outcome, record_blocked_unfocused_telemetry,
};
use super::timing::{interpret_down_send_timing, prepare_authored_batch_view, read_qpc_us};
use super::{AuthoredBatchView, AuthoredPacketContext, DispatchStep, PendingObservationQueue};
use crate::engine::telemetry::TRACE_KIND_MIXED;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_authored_packet(
    ctx: AuthoredPacketContext<'_>,
    config: &WorkerConfig,
    resources: &mut WorkerResources,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    metrics: &SharedMetrics,
    observer: &mut PendingObservationQueue,
) -> DispatchStep {
    let AuthoredPacketContext {
        dispatch_plan,
        effective_now_ticks,
        now_ticks,
        latency_class,
        focus_loss_fault,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
    } = ctx;

    let WorkerResources {
        clock: qpc_clock,
        backend,
        coordinator,
        playback: clock_state,
        telemetry,
        ..
    } = resources;
    let qpc_clock = *qpc_clock;

    let (lead_down, lead_down_saturated, lead_down_ticks) = match dispatch_plan.authored.as_ref() {
        Some(authored) => (
            authored.lead_us,
            authored.lead_saturated,
            authored.lead_ticks,
        ),
        None => (0, false, DurationTicks::ZERO),
    };
    let Some(frozen_budget) = dispatch_plan.authored_budget.as_ref() else {
        return DispatchStep::Terminate("authored dispatch plan has no health budget".to_string());
    };

    let prepared_batch =
        match coordinator.prepare_next_due_authored(effective_now_ticks, lead_down_ticks) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "coordinator authored-prepare failure: {error}"
                ));
            }
        };
    local_metrics.timeline_rebase_count = coordinator.timeline_rebase_count();
    local_metrics.timeline_rebase_total_ticks = coordinator.timeline_rebase_total_ticks();
    local_metrics.timeline_rebase_max_ticks = coordinator.timeline_rebase_max_ticks();
    local_metrics.timeline_rebase_last_reason = match coordinator.last_timeline_rebase_reason() {
        None => 0,
        Some(sky_dispatch_core::coordinator::TimelineRebaseReason::ReleaseRecovery) => 3,
    };

    let Some(prepared_batch) = prepared_batch else {
        return DispatchStep::NoWork;
    };

    let view = match prepare_authored_batch_view(coordinator, qpc_clock, prepared_batch) {
        Ok(Some(view)) => view,
        Ok(None) => return DispatchStep::NoWork,
        Err(step) => return step,
    };

    let has_down_events = view.packet_masks.is_some() || view.batch_kind == ActionKind::Down;
    if has_down_events {
        return commit_down_send_outcome(
            &view,
            config,
            health,
            timing,
            runtime,
            local_metrics,
            last_published_error,
            focus_active,
            target_hwnd,
            target_generation,
            quit_requested,
            skip_requested,
            panic_requested,
            desired_pause,
            metrics,
            qpc_clock,
            backend,
            coordinator,
            clock_state,
            telemetry,
            effective_now_ticks,
            now_ticks,
            lead_down,
            lead_down_saturated,
            lead_down_ticks,
            latency_class,
            focus_loss_fault,
            frozen_budget,
            supervisor_heartbeat_ticks,
            lease_timeout_ticks,
            observer,
        );
    }
    commit_suppressed_up_request(
        &view,
        coordinator,
        clock_state,
        qpc_clock,
        telemetry,
        backend,
        local_metrics,
        metrics,
        last_published_error,
        effective_now_ticks,
        lead_down_ticks,
    )
}

/// Admission gate + SendInput call + telemetry + estimator update + health
/// observation for an authored Down/Mixed batch.  Owns the physical send
/// boundary; the orchestrator only sees a `DispatchStep` outcome.
#[allow(clippy::too_many_arguments)]
fn commit_down_send_outcome(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    metrics: &SharedMetrics,
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    telemetry: &mut TelemetryCollector,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    lead_down: u64,
    lead_down_saturated: bool,
    lead_down_ticks: DurationTicks,
    latency_class: LatencyClass,
    focus_loss_fault: bool,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    observer: &mut PendingObservationQueue,
) -> DispatchStep {
    let has_conflicts = view.conflict_mask != 0;
    let admission = match admit_authored_down(
        view,
        config,
        qpc_clock,
        backend,
        coordinator,
        clock_state,
        telemetry,
        runtime,
        local_metrics,
        last_published_error,
        focus_active,
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        metrics,
        effective_now_ticks,
        now_ticks,
        lead_down_ticks,
        timing,
        has_conflicts,
        focus_loss_fault,
        frozen_budget,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
    ) {
        Ok(admission) => admission,
        Err(step) => return step,
    };
    record_down_send_outcome(
        view,
        config,
        health,
        timing,
        runtime,
        local_metrics,
        qpc_clock,
        backend,
        coordinator,
        clock_state,
        effective_now_ticks,
        lead_down,
        lead_down_saturated,
        lead_down_ticks,
        latency_class,
        &admission,
        observer,
    )
}

/// Outcome returned by `admit_authored_down`.  `Allowed` carries the frozen
/// dispatch budget plus the trace kind for the upcoming send; every other
/// variant is a non-terminal redirect (`Continue`) handled by the worker.
enum AdmissionOutcome {
    Allowed {
        frozen_budget: crate::engine::worker::health::FrozenDispatchBudget,
        trace_kind: u8,
    },
    BlockedUnfocused,
    FocusLost,
    TargetChanged,
    ControlRejected,
    ConflictReject,
}

/// Pre-send admission gate: focus, preflight, hard-late abort, conflict
/// detection, and final down-admission.  Performs short-circuit telemetry +
/// metric publish for the unfocused path.  Does not call `SendInput`.
#[allow(clippy::too_many_arguments)]
fn admit_authored_down(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    telemetry: &mut TelemetryCollector,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    metrics: &SharedMetrics,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    lead_down_ticks: DurationTicks,
    timing: &WorkerTimingState,
    has_conflicts: bool,
    focus_loss_fault: bool,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
) -> Result<AdmissionOutcome, DispatchStep> {
    let trace_kind = trace_kind_for_view(view);
    let has_down_events = view
        .packet_masks
        .is_some_and(|packet| packet.down_mask != 0)
        || view.batch_kind == ActionKind::Down;
    let has_physical_packet = view.packet_masks.is_some() || !view.scan_batch.is_empty();
    if has_down_events && !focus_matches(config.focus.require_focus, focus_active) {
        if let Err(error) =
            suspend_live_input(backend, coordinator, target_hwnd.load(Ordering::Acquire))
        {
            return Err(DispatchStep::Terminate(format!(
                "focus suspension failed: {error}"
            )));
        }
        if let Err(error) = clock_state.enter_pause("focus", now_ticks) {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
        runtime.focus_restore_started_ticks = None;
        record_blocked_unfocused_telemetry(telemetry, view, effective_now_ticks, lead_down_ticks)?;
        super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
        let current_us = read_qpc_us(qpc_clock, clock_state)?;
        try_publish_metrics(local_metrics, metrics, current_us, true);
        return Ok(AdmissionOutcome::BlockedUnfocused);
    }
    if has_down_events && focus_loss_fault && !runtime.focus_loss_fault_injected {
        runtime.focus_loss_fault_injected = true;
        return Err(DispatchStep::Terminate(
            "focus lost after due check before SendInput boundary".to_string(),
        ));
    }
    let preflight_target = load_target_stamp(target_hwnd, target_generation);
    if has_down_events {
        if let Err(error) =
            ensure_preflight_for_target(backend, preflight_target, &mut runtime.verified_target)
        {
            runtime.verified_target = None;
            return Err(DispatchStep::Terminate(format!(
                "instrument key preflight failed; release the 15 instrument keys before playback: {error}"
            )));
        }
        if !target_stamp_still_current(target_hwnd, target_generation, preflight_target) {
            runtime.verified_target = None;
            return Ok(AdmissionOutcome::TargetChanged);
        }
    }
    if config.timing.strict_timing
        && effective_now_ticks
            .checked_duration_since(view.authored_batch_scheduled_ticks)
            .is_ok_and(|late| late > timing.hard_late_abort_threshold_ticks)
    {
        return Err(DispatchStep::Terminate(format!(
            "authored Down exceeded hard lateness safety threshold of {}us",
            HARD_LATE_ABORT_THRESHOLD_US
        )));
    }
    if has_conflicts {
        local_metrics.authored_conflict_events =
            local_metrics.authored_conflict_events.saturating_add(1);
        local_metrics.authored_chords_rejected =
            local_metrics.authored_chords_rejected.saturating_add(1);
        local_metrics.authored_keys_rejected = local_metrics
            .authored_keys_rejected
            .saturating_add(view.batch_intent_count as u64);
        return Err(DispatchStep::Terminate(format!(
            "unexpected blocked authored Down at action {}",
            view.batch_source_action_index
        )));
    }
    if has_physical_packet {
        let (control_admission, _) = final_control_admission_with_lease(
            qpc_clock,
            lease_timeout_ticks,
            FinalControlSignals {
                quit_requested,
                skip_requested,
                panic_requested,
                desired_pause,
                supervisor_heartbeat_ticks,
            },
        )
        .map_err(|error| {
            DispatchStep::Terminate(format!("lease admission QPC failure: {error:?}"))
        })?;
        if !matches!(control_admission, FinalControlAdmission::Allowed) {
            runtime.verified_target = None;
            return Ok(AdmissionOutcome::ControlRejected);
        }

        if has_down_events {
            let admission = final_down_target_admission(FinalTargetSignals {
                expected: preflight_target,
                require_focus: config.focus.require_focus,
                focus_active,
                target_hwnd,
                target_generation,
            });
            match admission {
                DownAdmission::Allowed => {}
                DownAdmission::FocusLost => {
                    runtime.verified_target = None;
                    let focus_ticks = match qpc_clock.now() {
                        Ok(ticks) => ticks,
                        Err(error) => {
                            return Err(DispatchStep::Terminate(format!("QPC failure: {error:?}")));
                        }
                    };
                    if let Err(error) = suspend_live_input(
                        backend,
                        coordinator,
                        target_hwnd.load(Ordering::Acquire),
                    ) {
                        return Err(DispatchStep::Terminate(format!(
                            "focus suspension failed: {error}"
                        )));
                    }
                    if let Err(error) = clock_state.enter_pause("focus", focus_ticks) {
                        return Err(DispatchStep::Terminate(format!(
                            "playback clock failure after final focus check: {error}"
                        )));
                    }
                    runtime.focus_restore_started_ticks = None;
                    super::publish_backend_metrics(
                        backend,
                        local_metrics,
                        metrics,
                        last_published_error,
                    );
                    let current_us = read_qpc_us(qpc_clock, clock_state)?;
                    try_publish_metrics(local_metrics, metrics, current_us, true);
                    return Ok(AdmissionOutcome::FocusLost);
                }
                DownAdmission::TargetChanged => {
                    runtime.verified_target = None;
                    return Ok(AdmissionOutcome::TargetChanged);
                }
            }
        }
        return Ok(finalize_allowed_admission(frozen_budget, trace_kind));
    }
    Ok(AdmissionOutcome::ConflictReject)
}

/// Contstructs the allowed admission from the frozen dispatch budget and the
/// packet trace kind.  Extracted to keep `admit_authored_down` under the per
/// dispatch-function line limit.
fn trace_kind_for_view(view: &AuthoredBatchView) -> u8 {
    match view.prepared_batch.packet_kind {
        Some(sky_dispatch_core::model::PhysicalPacketKind::UpOnly) => TRACE_KIND_UP,
        Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly) => TRACE_KIND_DOWN,
        Some(sky_dispatch_core::model::PhysicalPacketKind::Mixed) => TRACE_KIND_MIXED,
        None => TRACE_KIND_DOWN,
    }
}

fn finalize_allowed_admission(
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    trace_kind: u8,
) -> AdmissionOutcome {
    AdmissionOutcome::Allowed {
        frozen_budget: *frozen_budget,
        trace_kind,
    }
}

#[allow(clippy::too_many_arguments)]
fn record_down_send_outcome(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    effective_now_ticks: TimelineTicks,
    lead_down: u64,
    lead_down_saturated: bool,
    lead_down_ticks: DurationTicks,
    latency_class: LatencyClass,
    admission: &AdmissionOutcome,
    observer: &mut PendingObservationQueue,
) -> DispatchStep {
    let AdmissionOutcome::Allowed {
        frozen_budget,
        trace_kind,
    } = admission
    else {
        return DispatchStep::Continue;
    };
    let result = if let Some(packet) = view.packet_masks {
        backend.key_down_physical_packet(packet)
    } else {
        backend.key_down(view.scan_batch.as_slice())
    };
    if let Some(error) = backend.timing_error.take() {
        return DispatchStep::Terminate(format!("QPC failure after note-on: {error:?}"));
    }
    let result_success = result.is_success();
    let result_started_ticks = result.evidence.started_ticks;
    let result_completed_ticks = result.evidence.completed_ticks;
    let result_sent = result.sent_scan_codes();
    let result_skipped_duplicates = result.skipped_duplicates();
    let result_send_attempts = result.evidence.attempts;
    let result_retry_reason = result.evidence.retry_reason;
    let result_chord_integrity_lost = matches!(
        result.status,
        sky_dispatch_win32::input::SendTransactionStatus::IntegrityLost
    );
    if result_chord_integrity_lost {
        runtime.chord_integrity_lost = runtime.chord_integrity_lost.saturating_add(1);
        local_metrics.chord_integrity_lost = local_metrics.chord_integrity_lost.saturating_add(1);
    }
    let result_last_win32_error = result.evidence.last_win32_error;
    if !result_success {
        return DispatchStep::Terminate(format!(
            "authored Down send integrity failure at action {}",
            view.batch_source_action_index
        ));
    }
    let trace_kind = *trace_kind;
    finalize_down_send_outcome(
        view,
        config,
        health,
        timing,
        runtime,
        local_metrics,
        qpc_clock,
        coordinator,
        clock_state,
        effective_now_ticks,
        lead_down,
        lead_down_saturated,
        lead_down_ticks,
        latency_class,
        frozen_budget,
        trace_kind,
        result_success,
        result.status,
        result_started_ticks,
        result_completed_ticks,
        &result_sent,
        &result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
fn finalize_down_send_outcome(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    effective_now_ticks: TimelineTicks,
    lead_down: u64,
    lead_down_saturated: bool,
    lead_down_ticks: DurationTicks,
    latency_class: LatencyClass,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    trace_kind: u8,
    result_success: bool,
    result_status: sky_dispatch_win32::input::SendTransactionStatus,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: sky_dispatch_win32::input::PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    observer: &mut PendingObservationQueue,
) -> DispatchStep {
    let timing_proof = match interpret_down_send_timing(
        view,
        config,
        clock_state,
        runtime,
        qpc_clock,
        coordinator,
        health,
        timing,
        result_success,
        result_status,
        result_started_ticks,
        result_completed_ticks,
        result_sent,
        result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
        lead_down_saturated,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };
    publisher_down_send_outcome(
        view,
        runtime,
        health,
        local_metrics,
        qpc_clock,
        effective_now_ticks,
        lead_down,
        lead_down_saturated,
        lead_down_ticks,
        latency_class,
        frozen_budget,
        trace_kind,
        result_success,
        result_sent,
        result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
        observer,
        &timing_proof,
    )
}

pub(super) fn resolve_slo_terminal_step(
    result_chord_integrity_lost: bool,
    retry_late_abort: bool,
    strict_completion_late: bool,
    saturation_abort: bool,
    completion_error_us: i64,
    view: &AuthoredBatchView,
    runtime: &mut WorkerRuntime,
) -> DispatchStep {
    if result_chord_integrity_lost {
        runtime.verified_target = None;
        return DispatchStep::Terminate(format!(
            "SendInput split authored chord at action {}",
            view.batch_source_action_index
        ));
    }
    if retry_late_abort {
        return DispatchStep::Terminate(format!(
            "strict timing rejected zero-progress retry at action {}: completion was {}us late",
            view.batch_source_action_index, completion_error_us
        ));
    }
    if strict_completion_late {
        let timing_label = if matches!(view.dispatch_path, DispatchPath::UpOnly { .. }) {
            "note-off"
        } else {
            "note-on"
        };
        return DispatchStep::Terminate(format!(
            "strict timing completion SLO exceeded for {timing_label} at action {}: completion was {}us late",
            view.batch_source_action_index, completion_error_us
        ));
    }
    if saturation_abort {
        let timing_label = if matches!(view.dispatch_path, DispatchPath::UpOnly { .. }) {
            "note-off"
        } else {
            "note-on"
        };
        return DispatchStep::Terminate(format!(
            "strict timing SLO exceeded: {timing_label} lead saturated with positive residual for {} consecutive dispatches",
            STRICT_SATURATION_ABORT_STREAK
        ));
    }
    DispatchStep::Dispatched
}
