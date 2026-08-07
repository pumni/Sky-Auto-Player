use super::super::{
    ActionKind, DurationTicks, HARD_LATE_ABORT_THRESHOLD_US, LatencyClass, PlaybackClockState,
    QpcClock, QpcTicks, RtTraceRecord, RuntimeDispatchCoordinator, STRICT_SATURATION_ABORT_STREAK,
    SendLatencyEstimator, SharedMetrics, TRACE_FLAG_ANOMALY, TRACE_FLAG_DEFERRED,
    TRACE_FLAG_RECOVERY, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TRACE_KIND_UP, TelemetryCollector,
    TimelineTicks, TraceContext, TraceDelivery, TraceTiming, TrackedKeyState, trace_outcome_code,
    try_publish_metrics,
};
use super::{
    DispatchHealthObservation, DispatchPath, DownAdmission, WorkerConfig, WorkerHealthState,
    WorkerMetricsLocal, WorkerResources, WorkerRuntime, WorkerTimingState, build_dispatch_budget,
    cancel_coordinator_or_terminal, describe_release_outcome, ensure_preflight_for_target,
    estimator_kind_for_path, final_down_admission, focus_matches, load_target_stamp,
    observe_dispatch_health, planning::NextDispatchPlan, record_lateness, record_lead_saturation,
    record_termination_error, release_runtime_outcome, release_state_verified, signed_delta,
    signed_ticks_to_us, signed_timeline_delta_ticks, suspend_live_input,
    target_stamp_still_current, update_estimator_after_send_class,
};
use crate::engine::telemetry::TRACE_KIND_MIXED;
use sky_dispatch_core::coordinator::{PendingDispatchPlan, PendingRelease, PreparedBatch};
use sky_dispatch_core::model::ScanCodeBatch;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

pub(crate) enum DispatchStep {
    NoWork,
    Dispatched,
    Continue,
    Terminate(String),
}

pub(crate) struct PendingReleaseContext<'a> {
    pub(crate) due_pending: SmallVec<[PendingRelease; 15]>,
    pub(crate) pending_plan: Option<&'a PendingDispatchPlan>,
    pub(crate) lead_up_ticks: DurationTicks,
    pub(crate) lead_up: u64,
    pub(crate) latency_class: LatencyClass,
}

/// Evidence captured from the note-off SendInput call plus the timeline
/// projections used by downstream reconciliation.
struct ReleaseSend {
    started_us: u64,
    actual_ticks: TimelineTicks,
    completed_effective_ticks: TimelineTicks,
    completed_effective_us: u64,
    sender_started_effective_ticks: Option<TimelineTicks>,
    last_win32_error: Option<u32>,
    completed_us: u64,
    sent_count: usize,
    skipped_count: usize,
    attempts: u8,
    is_success: bool,
}

/// Per-event timing reconciliation derived from the pending release batch
/// and the SendInput note-off evidence.
struct ReleaseReconciliation {
    recovery_required: bool,
    bookkeeping_completed_us: u64,
    first_index: usize,
    effective_deadline_ticks: TimelineTicks,
    scheduled_ticks: TimelineTicks,
    scheduled_us: u64,
    deferred_by_us: u64,
    up_completion_lateness_ticks: Option<DurationTicks>,
    up_completion_error_ticks: i64,
    up_authored_completion_error_ticks: i64,
    up_completion_error_us: i64,
    clean_up_sample: bool,
}

/// Strict/SLO flags computed after estimator and health state have observed
/// the send; the orchestrator uses them for terminal decisions.
struct ReleaseOutcomeFlags {
    strict_up_completion_late: bool,
    saturation_abort: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_due_pending_releases(
    ctx: PendingReleaseContext<'_>,
    config: &WorkerConfig,
    resources: &mut WorkerResources,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    secondary_errors: &mut Vec<String>,
    last_published_error: &mut Option<String>,
    target_hwnd: &AtomicIsize,
    metrics: &SharedMetrics,
) -> DispatchStep {
    let PendingReleaseContext {
        due_pending,
        pending_plan,
        lead_up_ticks,
        lead_up,
        latency_class,
    } = ctx;

    let WorkerResources {
        clock: qpc_clock,
        backend,
        coordinator,
        playback: clock_state,
        estimator,
        telemetry,
        ..
    } = resources;
    let qpc_clock = *qpc_clock;

    let scan_codes: SmallVec<[u16; 15]> = due_pending.iter().map(|p| p.scan_code).collect();
    let scan_count = scan_codes.len();
    let frozen_budget = build_dispatch_budget(
        estimator,
        DispatchPath::UpOnly {
            up_count: scan_count,
        },
        latency_class,
        health.options,
        config.timing.strict_timing,
    );

    let send = match prepare_release_send(qpc_clock, backend, clock_state, runtime, &scan_codes) {
        Ok(value) => value,
        Err(step) => return step,
    };

    let reconciliation = match reconcile_release_recovery(
        coordinator,
        qpc_clock,
        clock_state,
        timing,
        local_metrics,
        &due_pending,
        &send,
        lead_up_ticks,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };

    if let Err(step) = record_release_telemetry(
        telemetry,
        &due_pending,
        &send,
        &reconciliation,
        lead_up_ticks,
    ) {
        return step;
    }

    let flags = match observe_release_send_health(
        qpc_clock,
        clock_state,
        config,
        timing,
        estimator,
        health,
        backend,
        runtime,
        local_metrics,
        metrics,
        last_published_error,
        &send,
        &reconciliation,
        &frozen_budget,
        lead_up,
        latency_class,
        pending_plan,
        scan_count,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };

    if reconciliation.recovery_required {
        return finalize_release_recovery(
            backend,
            coordinator,
            runtime,
            secondary_errors,
            target_hwnd,
            &send,
        );
    }
    if flags.strict_up_completion_late {
        let first = &due_pending[reconciliation.first_index];
        return DispatchStep::Terminate(format!(
            "strict timing completion SLO exceeded for note-off at action {}: completion was {}us late",
            first.source_action_index, reconciliation.up_completion_error_us
        ));
    }
    if flags.saturation_abort {
        return DispatchStep::Terminate(format!(
            "strict timing SLO exceeded: note-off lead saturated with positive residual for {} consecutive dispatches",
            STRICT_SATURATION_ABORT_STREAK
        ));
    }

    DispatchStep::Dispatched
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
/// Built once per authored epoch by `prepare_authored_batch_view`; the
/// send/admission/telemetry helpers consume it without re-querying the
/// coordinator schedule.
struct AuthoredBatchView {
    prepared_batch: PreparedBatch,
    batch_source_action_index: u32,
    batch_intent_count: usize,
    batch_kind: ActionKind,
    batch_scheduled_ticks: TimelineTicks,
    batch_scheduled_us: u64,
    authored_batch_scheduled_ticks: TimelineTicks,
    authored_batch_scheduled_us: u64,
    conflict_mask: u16,
    dispatch_path: DispatchPath,
    packet_mode: bool,
    packet_masks: Option<sky_dispatch_win32::input::PhysicalPacket>,
    scan_batch: ScanCodeBatch,
}

/// `Err(None)` indicates an unrecoverable terminal step; `Ok(None)` means the
/// coordinator offered no authored work for this epoch (worker should advance
/// the wait deadline instead).
type BatchViewResult = Result<Option<AuthoredBatchView>, DispatchStep>;

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
) -> DispatchStep {
    let AuthoredPacketContext {
        dispatch_plan,
        effective_now_ticks,
        now_ticks,
        latency_class,
        focus_loss_fault,
    } = ctx;

    let WorkerResources {
        clock: qpc_clock,
        backend,
        coordinator,
        playback: clock_state,
        estimator,
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
    local_metrics.timeline_rebase_total_us =
        match qpc_clock.duration_to_us(coordinator.timeline_rebase_total_ticks()) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "timeline rebase telemetry conversion failure: {error:?}"
                ));
            }
        };
    local_metrics.timeline_rebase_max_us =
        match qpc_clock.duration_to_us(coordinator.timeline_rebase_max_ticks()) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "timeline rebase telemetry conversion failure: {error:?}"
                ));
            }
        };
    local_metrics.timeline_rebase_last_reason = match coordinator.last_timeline_rebase_reason() {
        None => 0,
        Some(sky_dispatch_core::coordinator::TimelineRebaseReason::WorkerLate) => 1,
        Some(sky_dispatch_core::coordinator::TimelineRebaseReason::ReleaseFloor) => 2,
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
            estimator,
            telemetry,
            effective_now_ticks,
            now_ticks,
            lead_down,
            lead_down_saturated,
            lead_down_ticks,
            latency_class,
            focus_loss_fault,
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

/// Project the prepared authored batch into a snapshot used by admission, send,
/// and telemetry.  Built once per epoch so the worker does not re-query the
/// coordinator schedule across multiple invariants within a single loop epoch
/// (D1 — one immutable dispatch plan per epoch).
fn prepare_authored_batch_view(
    coordinator: &mut RuntimeDispatchCoordinator,
    qpc_clock: QpcClock,
    prepared_batch: PreparedBatch,
) -> BatchViewResult {
    let batch_index = prepared_batch.index;
    let batch_scheduled_ticks = prepared_batch.effective_scheduled_ticks;
    let packet_mode = match prepared_batch.packet_kind {
        Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly)
            if prepared_batch.packet_batch_count == 1 =>
        {
            false
        }
        Some(_) => true,
        None => false,
    };
    let (
        batch_kind,
        dispatch_path,
        batch_source_action_index,
        batch_intent_count,
        conflict_mask,
        scan_batch,
        packet_masks,
    ) = if packet_mode {
        let packet_view = match coordinator
            .schedule
            .view_packet_ticks(prepared_batch.packet_index, batch_scheduled_ticks)
        {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "runtime packet view failure: {error}"
                )));
            }
        };
        let conflict_mask = coordinator
            .check_packet_down_conflicts(packet_view.up_mask(), packet_view.down_intents);
        let up_count = packet_view.up_mask().count_ones() as usize;
        let down_count = packet_view.down_mask().count_ones() as usize;
        let dispatch_path = match prepared_batch.packet_kind {
            Some(sky_dispatch_core::model::PhysicalPacketKind::UpOnly) => {
                DispatchPath::UpOnly { up_count }
            }
            Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly) => {
                DispatchPath::DownOnly { down_count }
            }
            Some(sky_dispatch_core::model::PhysicalPacketKind::Mixed) => DispatchPath::Mixed {
                up_count,
                down_count,
            },
            None => DispatchPath::DownOnly { down_count: 0 },
        };
        (
            if matches!(dispatch_path, DispatchPath::UpOnly { .. }) {
                ActionKind::Up
            } else {
                ActionKind::Down
            },
            dispatch_path,
            packet_view
                .header
                .down_source_action_index
                .or_else(|| {
                    coordinator
                        .schedule
                        .batches
                        .get(packet_view.header.first_batch_index as usize)
                        .map(|batch| batch.source_action_index)
                })
                .unwrap_or(0),
            up_count + down_count,
            conflict_mask,
            packet_view.down_scan_code_batch(),
            Some(sky_dispatch_win32::input::PhysicalPacket::new(
                packet_view.up_mask(),
                packet_view.down_mask(),
            )),
        )
    } else {
        let batch_view = match coordinator
            .schedule
            .view_batch_ticks(batch_index, batch_scheduled_ticks)
        {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "runtime schedule view failure: {error}"
                )));
            }
        };
        let conflict_mask = coordinator.check_down_conflicts_compact(batch_view.intents);
        (
            batch_view.kind(),
            match batch_view.kind() {
                ActionKind::Up => DispatchPath::UpOnly {
                    up_count: batch_view.intents.len(),
                },
                ActionKind::Down => DispatchPath::DownOnly {
                    down_count: batch_view.intents.len(),
                },
            },
            batch_view.source_action_index(),
            batch_view.intents.len(),
            conflict_mask,
            batch_view.scan_code_batch_excluding_mask(conflict_mask),
            None,
        )
    };
    let batch_scheduled_us = match qpc_clock.timeline_to_us(batch_scheduled_ticks) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "schedule telemetry conversion failure: {error:?}"
            )));
        }
    };
    let authored_batch_scheduled_ticks = coordinator.batch_scheduled_ticks[batch_index];
    let authored_batch_scheduled_us = match qpc_clock.timeline_to_us(authored_batch_scheduled_ticks)
    {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "authored schedule telemetry conversion failure: {error:?}"
            )));
        }
    };
    Ok(Some(AuthoredBatchView {
        prepared_batch,
        batch_source_action_index,
        batch_intent_count,
        batch_kind,
        batch_scheduled_ticks,
        batch_scheduled_us,
        authored_batch_scheduled_ticks,
        authored_batch_scheduled_us,
        conflict_mask,
        dispatch_path,
        packet_mode,
        packet_masks,
        scan_batch,
    }))
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
    estimator: &mut SendLatencyEstimator,
    telemetry: &mut TelemetryCollector,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    lead_down: u64,
    lead_down_saturated: bool,
    lead_down_ticks: DurationTicks,
    latency_class: LatencyClass,
    focus_loss_fault: bool,
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
        latency_class,
        estimator,
        health,
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
        last_published_error,
        metrics,
        qpc_clock,
        backend,
        coordinator,
        clock_state,
        estimator,
        telemetry,
        effective_now_ticks,
        lead_down,
        lead_down_saturated,
        lead_down_ticks,
        latency_class,
        &admission,
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
    latency_class: LatencyClass,
    estimator: &SendLatencyEstimator,
    health: &mut WorkerHealthState,
) -> Result<AdmissionOutcome, DispatchStep> {
    if !view
        .packet_masks
        .is_some_and(|packet| packet.down_mask == 0)
        && !focus_matches(config.focus.require_focus, focus_active, target_hwnd)
    {
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
    if !view
        .packet_masks
        .is_some_and(|packet| packet.down_mask == 0)
        && focus_loss_fault
        && !runtime.focus_loss_fault_injected
    {
        runtime.focus_loss_fault_injected = true;
        return Err(DispatchStep::Terminate(
            "focus lost after due check before SendInput boundary".to_string(),
        ));
    }
    let preflight_target = load_target_stamp(target_hwnd, target_generation);
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
    if view.packet_masks.is_some() || !view.scan_batch.is_empty() {
        let admission = if view
            .packet_masks
            .is_some_and(|packet| packet.down_mask == 0)
        {
            DownAdmission::Allowed
        } else {
            final_down_admission(
                preflight_target,
                config.focus.require_focus,
                focus_active,
                target_hwnd,
                target_generation,
                quit_requested,
                skip_requested,
                panic_requested,
                desired_pause,
            )
        };
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
                if let Err(error) =
                    suspend_live_input(backend, coordinator, target_hwnd.load(Ordering::Acquire))
                {
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
            DownAdmission::TargetChanged
            | DownAdmission::PauseRequested
            | DownAdmission::QuitRequested
            | DownAdmission::SkipRequested
            | DownAdmission::PanicRequested => {
                runtime.verified_target = None;
                return Ok(AdmissionOutcome::TargetChanged);
            }
        }
        let frozen_budget = build_dispatch_budget(
            estimator,
            view.dispatch_path,
            latency_class,
            health.options,
            config.timing.strict_timing,
        );
        let trace_kind = match view.prepared_batch.packet_kind {
            Some(sky_dispatch_core::model::PhysicalPacketKind::UpOnly) => TRACE_KIND_UP,
            Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly) => TRACE_KIND_DOWN,
            Some(sky_dispatch_core::model::PhysicalPacketKind::Mixed) => TRACE_KIND_MIXED,
            None => TRACE_KIND_DOWN,
        };
        return Ok(AdmissionOutcome::Allowed {
            frozen_budget,
            trace_kind,
        });
    }
    Ok(AdmissionOutcome::ConflictReject)
}

#[allow(clippy::too_many_arguments)]
fn record_down_send_outcome(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    estimator: &mut SendLatencyEstimator,
    telemetry: &mut TelemetryCollector,
    effective_now_ticks: TimelineTicks,
    lead_down: u64,
    lead_down_saturated: bool,
    lead_down_ticks: DurationTicks,
    latency_class: LatencyClass,
    admission: &AdmissionOutcome,
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
    let trace_kind = *trace_kind;
    let result_success = result.is_success();
    let result_started_ticks = result.evidence.started_ticks;
    let result_completed_ticks = result.evidence.completed_ticks;
    let result_completed_us = result.completed_us();
    let result_sent = result.sent_scan_codes();
    let result_skipped_duplicates = result.skipped_duplicates();
    let result_send_attempts = result.evidence.attempts;
    let result_retry_reason = result.evidence.retry_reason;
    let result_chord_integrity_lost = matches!(
        result.status,
        sky_dispatch_win32::input::SendTransactionStatus::IntegrityLost
    );
    let result_last_win32_error = result.evidence.last_win32_error;
    if !result_success {
        return DispatchStep::Terminate(format!(
            "authored Down send integrity failure at action {}",
            view.batch_source_action_index
        ));
    }
    finalize_down_send_outcome(
        view,
        config,
        health,
        timing,
        runtime,
        local_metrics,
        last_published_error,
        metrics,
        qpc_clock,
        backend,
        coordinator,
        clock_state,
        estimator,
        telemetry,
        effective_now_ticks,
        lead_down,
        lead_down_saturated,
        lead_down_ticks,
        latency_class,
        frozen_budget,
        trace_kind,
        result_success,
        result_started_ticks,
        result_completed_ticks,
        result_completed_us,
        &result_sent,
        &result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
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
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    estimator: &mut SendLatencyEstimator,
    telemetry: &mut TelemetryCollector,
    effective_now_ticks: TimelineTicks,
    lead_down: u64,
    lead_down_saturated: bool,
    lead_down_ticks: DurationTicks,
    latency_class: LatencyClass,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    trace_kind: u8,
    result_success: bool,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
    result_completed_us: u64,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: sky_dispatch_win32::input::PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
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
        local_metrics,
        result_success,
        result_started_ticks,
        result_completed_ticks,
        result_sent,
        result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        lead_down_saturated,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };
    publisher_down_send_outcome(
        view,
        config,
        health,
        runtime,
        local_metrics,
        last_published_error,
        metrics,
        qpc_clock,
        backend,
        clock_state,
        estimator,
        telemetry,
        effective_now_ticks,
        lead_down,
        lead_down_saturated,
        lead_down_ticks,
        latency_class,
        frozen_budget,
        trace_kind,
        result_success,
        result_completed_us,
        result_sent,
        result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
        &timing_proof,
    )
}

/// Timing-derived evidence captured from the note-on SendInput call:
/// projections used across telemetry, estimator update, and the terminal
/// SLO decision.
struct DownSendTiming {
    sender_started_effective_ticks: TimelineTicks,
    completed_effective_ticks: TimelineTicks,
    completed_effective: u64,
    sender_duration_us: u64,
    requested_count: usize,
    delivered_count: usize,
    completion_error_ticks_value: i64,
    authored_completion_error_ticks_value: i64,
    completion_error_us: i64,
    estimator_kind: Option<ActionKind>,
    clean_directional_sample: bool,
    recovered_zero_progress: bool,
    recovered_partial_up: bool,
    recovered_retry_late: bool,
    strict_completion_late: bool,
    retry_late_abort: bool,
    saturation_abort: bool,
    bookkeeping_completed_us: u64,
}

/// Resolves the QPC evidence, commits the prepared batch, computes timing
/// SLO flags, the saturation-abort streak, and the bookkeeping completion
/// marker.  Mutates `coordinator` (commit) and `health` (saturation streak).
/// Does not record telemetry or call the estimator.
#[allow(clippy::too_many_arguments)]
fn interpret_down_send_timing(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    qpc_clock: QpcClock,
    coordinator: &mut RuntimeDispatchCoordinator,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    local_metrics: &mut WorkerMetricsLocal,
    result_success: bool,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: sky_dispatch_win32::input::PacketRetryReason,
    result_chord_integrity_lost: bool,
    lead_down_saturated: bool,
) -> Result<DownSendTiming, DispatchStep> {
    let sender_started_ticks = match result_started_ticks {
        Some(ticks) => ticks,
        None => {
            return Err(DispatchStep::Terminate(
                "SendInput note-on succeeded without a QPC start boundary".to_string(),
            ));
        }
    };
    let completed_qpc_ticks = match result_completed_ticks {
        Some(ticks) => ticks,
        None => {
            return Err(DispatchStep::Terminate(
                "SendInput note-on completed without a QPC completion boundary".to_string(),
            ));
        }
    };
    let sender_duration_ticks =
        match completed_qpc_ticks.checked_duration_since(sender_started_ticks) {
            Ok(duration) => duration,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-on QPC ordering failure: {error}"
                )));
            }
        };
    let sender_duration_us = match qpc_clock.duration_to_us(sender_duration_ticks) {
        Ok(duration) => duration,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-on sender duration conversion failure: {error:?}"
            )));
        }
    };
    let sender_started_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        sender_started_ticks,
        runtime.allow_pre_epoch_startup_dispatch,
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
    };
    let completed_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        completed_qpc_ticks,
        runtime.allow_pre_epoch_startup_dispatch,
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
    };
    let completed_effective = match qpc_clock.duration_to_us(
        match completed_effective_ticks.checked_duration_since(TimelineTicks::ZERO) {
            Ok(dur) => dur,
            Err(_) => DurationTicks::ZERO,
        },
    ) {
        Ok(us) => us,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock conversion failure: {error:?}"
            )));
        }
    };
    runtime.last_send_qpc_ticks = Some(completed_qpc_ticks);
    let commit_result = if view.packet_mode {
        coordinator.commit_packet_success(
            view.prepared_batch,
            sender_started_effective_ticks,
            completed_effective_ticks,
        )
    } else {
        coordinator.commit_down_success(
            view.prepared_batch,
            result_sent,
            sender_started_effective_ticks,
            completed_effective_ticks,
        )
    };
    if let Err(error) = commit_result {
        return Err(DispatchStep::Terminate(format!(
            "coordinator activation failure: {error}"
        )));
    }
    let completion_lateness_ticks = completed_effective_ticks
        .checked_duration_since(view.batch_scheduled_ticks)
        .ok();
    let completion_error_ticks_value =
        match signed_timeline_delta_ticks(completed_effective_ticks, view.batch_scheduled_ticks) {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-on timing conversion failure: {error}"
                )));
            }
        };
    let authored_completion_error_ticks_value = match signed_timeline_delta_ticks(
        completed_effective_ticks,
        view.authored_batch_scheduled_ticks,
    ) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-on authored timing conversion failure: {error}"
            )));
        }
    };
    let completion_error_us = match signed_ticks_to_us(qpc_clock, completion_error_ticks_value) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-on timing conversion failure: {error}"
            )));
        }
    };
    let requested_count = view.dispatch_path.event_count();
    let delivered_count = if view.packet_mode {
        usize::from(result_success) * requested_count
    } else {
        result_sent.len()
    };
    let estimator_kind = estimator_kind_for_path(view.dispatch_path);
    let clean_directional_sample = result_success
        && result_skipped_duplicates.is_empty()
        && result_send_attempts == 1
        && !result_chord_integrity_lost
        && !matches!(view.dispatch_path, DispatchPath::Mixed { .. })
        && estimator_kind.is_some()
        && delivered_count == requested_count;
    let recovered_zero_progress = matches!(
        result_retry_reason,
        sky_dispatch_win32::input::PacketRetryReason::ZeroProgress
    );
    let recovered_partial_up = matches!(
        (view.dispatch_path, result_retry_reason),
        (
            DispatchPath::UpOnly { .. },
            sky_dispatch_win32::input::PacketRetryReason::PartialProgress { .. }
        )
    ) && result_success;
    let recovered_retry_late = recovered_zero_progress
        && result_success
        && completion_lateness_ticks.is_some_and(|late| late > timing.retry_late_threshold_ticks);
    let retry_late_abort = config.timing.strict_timing && recovered_retry_late;
    let strict_completion_late = config.timing.strict_timing
        && clean_directional_sample
        && completion_lateness_ticks.is_some_and(|late| {
            late > match view.dispatch_path {
                DispatchPath::UpOnly { .. } => timing.strict_up_completion_late_ticks,
                DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                    timing.strict_down_completion_late_ticks
                }
            }
        });
    if recovered_retry_late {
        local_metrics.recovered_zero_progress_but_late = local_metrics
            .recovered_zero_progress_but_late
            .saturating_add(1);
    }
    let saturation_abort = match view.dispatch_path {
        DispatchPath::UpOnly { .. } => {
            health.down_saturation_positive_streak = 0;
            health.up_saturation_positive_streak =
                if lead_down_saturated && completion_lateness_ticks.is_some() {
                    health.up_saturation_positive_streak.saturating_add(1)
                } else {
                    0
                };
            config.timing.strict_timing
                && health.up_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK
        }
        DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
            health.up_saturation_positive_streak = 0;
            health.down_saturation_positive_streak =
                if lead_down_saturated && completion_lateness_ticks.is_some() {
                    health.down_saturation_positive_streak.saturating_add(1)
                } else {
                    0
                };
            config.timing.strict_timing
                && health.down_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK
        }
    };
    let bookkeeping_completed_us = read_qpc_us(qpc_clock, clock_state)?;
    if recovered_zero_progress && result_success {
        local_metrics.recovered_zero_progress_retries = local_metrics
            .recovered_zero_progress_retries
            .saturating_add(1);
    }
    if recovered_partial_up {
        local_metrics.recovered_partial_up_retries =
            local_metrics.recovered_partial_up_retries.saturating_add(1);
    }
    Ok(DownSendTiming {
        sender_started_effective_ticks,
        completed_effective_ticks,
        completed_effective,
        sender_duration_us,
        requested_count,
        delivered_count,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        completion_error_us,
        estimator_kind,
        clean_directional_sample,
        recovered_zero_progress,
        recovered_partial_up,
        recovered_retry_late,
        strict_completion_late,
        retry_late_abort,
        saturation_abort,
        bookkeeping_completed_us,
    })
}

/// Owner of: telemetry record, estimator update, lateness derivation, metric
/// publish, dispatch health observation, and SLO terminal decisions for the
/// note-on send.  All values derived from `interpret_down_send_timing` are
/// already snapshotted — this function performs no further QPC resolution
/// beyond the two wall-clock samples needed for metric publication and the
/// iteration-ready boundary.
#[allow(clippy::too_many_arguments)]
fn publisher_down_send_outcome(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    clock_state: &PlaybackClockState,
    estimator: &mut SendLatencyEstimator,
    telemetry: &mut TelemetryCollector,
    effective_now_ticks: TimelineTicks,
    lead_down: u64,
    lead_down_saturated: bool,
    lead_down_ticks: DurationTicks,
    latency_class: LatencyClass,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    trace_kind: u8,
    result_success: bool,
    result_completed_us: u64,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: sky_dispatch_win32::input::PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    timing_proof: &DownSendTiming,
) -> DispatchStep {
    let DownSendTiming {
        sender_started_effective_ticks,
        completed_effective_ticks,
        completed_effective,
        sender_duration_us,
        requested_count,
        delivered_count,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        completion_error_us,
        estimator_kind,
        clean_directional_sample,
        recovered_zero_progress,
        recovered_partial_up,
        recovered_retry_late,
        strict_completion_late,
        retry_late_abort,
        saturation_abort,
        bookkeeping_completed_us,
    } = *timing_proof;
    let (_down_outcome, force_dispatch_publish) = match record_down_send_telemetry(
        view,
        telemetry,
        trace_kind,
        effective_now_ticks,
        lead_down_ticks,
        result_success,
        result_completed_us,
        result_sent,
        result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
        sender_started_effective_ticks,
        completed_effective_ticks,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        bookkeeping_completed_us,
        requested_count,
        recovered_retry_late,
        recovered_partial_up,
        strict_completion_late,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };
    let mut force_dispatch_publish = force_dispatch_publish;
    if config.estimator.enable_adaptive_lead && lead_down_saturated {
        match view.dispatch_path {
            DispatchPath::UpOnly { .. } => record_lead_saturation(
                &mut local_metrics.lead_saturation_count_up,
                &mut local_metrics.positive_residual_at_cap,
                view.batch_intent_count,
                signed_delta(completed_effective, view.batch_scheduled_us),
            ),
            DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => record_lead_saturation(
                &mut local_metrics.lead_saturation_count_down,
                &mut local_metrics.positive_residual_at_cap,
                view.batch_intent_count,
                signed_delta(completed_effective, view.batch_scheduled_us),
            ),
        }
    }
    runtime.pending_pre_send_spin_us = 0;
    let send_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.send_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.bookkeeping_warn_threshold_us = frozen_budget.bookkeeping_warn_us;
    match view.dispatch_path {
        DispatchPath::DownOnly { .. } => {
            local_metrics.send_down_warn_threshold_us = frozen_budget.send_warn_us;
        }
        DispatchPath::UpOnly { .. } => {
            local_metrics.send_up_warn_threshold_us = frozen_budget.send_warn_us;
        }
        DispatchPath::Mixed { .. } => {
            local_metrics.send_mixed_warn_threshold_us = frozen_budget.send_warn_us;
        }
    }
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    if config.estimator.enable_adaptive_lead
        && let Some(kind) = estimator_kind
        && let Err(error) = update_estimator_after_send_class(
            estimator,
            kind,
            sender_duration_us,
            delivered_count,
            view.batch_intent_count,
            lead_down,
            completion_error_us,
            clean_directional_sample,
            latency_class,
        )
    {
        return DispatchStep::Terminate(format!("estimator update failure: {error}"));
    }
    record_lateness(
        signed_delta(completed_effective, view.authored_batch_scheduled_us),
        false,
        false,
        local_metrics,
    );
    let _ = recovered_zero_progress;
    let terminal_dispatch = result_chord_integrity_lost
        || retry_late_abort
        || strict_completion_late
        || saturation_abort;
    if terminal_dispatch {
        force_dispatch_publish = true;
    }
    super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    let current_us = match read_qpc_us(qpc_clock, clock_state) {
        Ok(us) => us,
        Err(step) => return step,
    };
    try_publish_metrics(local_metrics, metrics, current_us, force_dispatch_publish);
    let iteration_ready_us = match read_qpc_us(qpc_clock, clock_state) {
        Ok(us) => us,
        Err(step) => return step,
    };
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: sender_duration_us,
            post_send_duration_us: iteration_ready_us.saturating_sub(result_completed_us),
            path: frozen_budget.path,
            send_warn_us: send_warn_threshold_us,
            bookkeeping_warn_us: frozen_budget.bookkeeping_warn_us,
            elapsed_us: completed_effective,
        },
        health.options.window_policy(),
        &mut health.send_pure_window,
        &mut health.bookkeeping_window,
        local_metrics,
    );
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

/// Computes the down_outcome label, the boolean force-publish flag, and the
/// trace flags; pushes the `RtTraceRecord::dispatched` entry for one note-on
/// packet.  Returns the outcome label so callers can use it for additional
/// accounting without recomputing the predicate tree.
#[allow(clippy::too_many_arguments)]
fn record_down_send_telemetry(
    view: &AuthoredBatchView,
    telemetry: &mut TelemetryCollector,
    trace_kind: u8,
    effective_now_ticks: TimelineTicks,
    lead_down_ticks: DurationTicks,
    result_success: bool,
    result_completed_us: u64,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: sky_dispatch_win32::input::PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    sender_started_effective_ticks: TimelineTicks,
    completed_effective_ticks: TimelineTicks,
    completion_error_ticks_value: i64,
    authored_completion_error_ticks_value: i64,
    bookkeeping_completed_us: u64,
    requested_count: usize,
    recovered_retry_late: bool,
    recovered_partial_up: bool,
    strict_completion_late: bool,
) -> Result<(&'static str, bool), DispatchStep> {
    let down_outcome = if recovered_retry_late {
        "recovered_zero_progress_but_late"
    } else if recovered_partial_up {
        "recovered_partial_up_retry"
    } else if strict_completion_late {
        "strict_completion_slo_exceeded"
    } else if result_chord_integrity_lost {
        "chord_integrity_lost"
    } else if view.packet_masks.is_some_and(|_| result_success)
        || (view.packet_masks.is_none() && result_sent.len() == view.scan_batch.len())
    {
        "sent"
    } else {
        "partial_note_on"
    };
    let force_publish = !result_success
        || !matches!(
            result_retry_reason,
            sky_dispatch_win32::input::PacketRetryReason::None
        )
        || result_chord_integrity_lost;
    let mut trace_flags = 0u8;
    let send_completed_fully = view.packet_masks.is_some_and(|_| result_success)
        || (view.packet_masks.is_none() && result_sent.len() == view.scan_batch.len());
    if send_completed_fully {
        trace_flags |= TRACE_FLAG_SENT_FULL;
    }
    if recovered_retry_late || result_chord_integrity_lost {
        trace_flags |= TRACE_FLAG_RECOVERY;
    }
    if down_outcome != "sent" {
        trace_flags |= TRACE_FLAG_ANOMALY;
    }
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: view.batch_source_action_index,
                kind: trace_kind,
                outcome: trace_outcome_code(down_outcome),
                polyphony: view.batch_intent_count,
                flags: trace_flags,
                win32_error: result_last_win32_error.unwrap_or(0),
            },
            TraceTiming {
                authored_ticks: view.authored_batch_scheduled_ticks,
                effective_deadline_ticks: view.batch_scheduled_ticks,
                wake_ticks: effective_now_ticks,
                send_started_ticks: Some(sender_started_effective_ticks),
                send_completed_ticks: Some(completed_effective_ticks),
                bookkeeping_duration_us: bookkeeping_completed_us
                    .saturating_sub(result_completed_us),
                completion_error_ticks: completion_error_ticks_value,
                authored_completion_error_ticks: authored_completion_error_ticks_value,
                applied_lead_ticks: lead_down_ticks,
            },
            TraceDelivery {
                requested: view.batch_intent_count,
                sent: if view.packet_masks.is_some() && result_success {
                    requested_count
                } else {
                    result_sent.len()
                },
                skipped: result_skipped_duplicates.len(),
                send_attempts: usize::from(result_send_attempts),
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok((down_outcome, force_publish))
}

#[allow(clippy::too_many_arguments)]
fn commit_suppressed_up_request(
    view: &AuthoredBatchView,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    qpc_clock: QpcClock,
    telemetry: &mut TelemetryCollector,
    backend: &TrackedKeyState,
    local_metrics: &mut WorkerMetricsLocal,
    metrics: &SharedMetrics,
    last_published_error: &mut Option<String>,
    effective_now_ticks: TimelineTicks,
    lead_down_ticks: DurationTicks,
) -> DispatchStep {
    let (_, suppressed) = match coordinator.commit_up_request(view.prepared_batch) {
        Ok(value) => value,
        Err(error) => {
            return DispatchStep::Terminate(format!(
                "coordinator note-off request failure: {error}"
            ));
        }
    };
    if !suppressed.is_empty()
        && let Err(error) = telemetry.try_push(|| {
            RtTraceRecord::dispatched(
                TraceContext {
                    event_index: view.batch_source_action_index,
                    kind: TRACE_KIND_UP,
                    outcome: trace_outcome_code("suppressed_stale_up"),
                    polyphony: suppressed.len(),
                    flags: TRACE_FLAG_ANOMALY,
                    win32_error: 0,
                },
                TraceTiming {
                    authored_ticks: view.authored_batch_scheduled_ticks,
                    effective_deadline_ticks: view.batch_scheduled_ticks,
                    wake_ticks: effective_now_ticks,
                    send_started_ticks: None,
                    send_completed_ticks: None,
                    bookkeeping_duration_us: 0,
                    completion_error_ticks: 0,
                    authored_completion_error_ticks: 0,
                    applied_lead_ticks: lead_down_ticks,
                },
                TraceDelivery {
                    requested: 0,
                    sent: 0,
                    skipped: 0,
                    send_attempts: 0,
                },
            )
        })
    {
        return DispatchStep::Terminate(format!("native telemetry record overflow: {error}"));
    }
    super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    let current_us = match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
            }) {
                Ok(us) => us,
                Err(error) => {
                    return DispatchStep::Terminate(format!(
                        "QPC us conversion failure: {error:?}"
                    ));
                }
            }
        }
        Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
    };
    try_publish_metrics(local_metrics, metrics, current_us, !suppressed.is_empty());
    DispatchStep::Dispatched
}

fn record_blocked_unfocused_telemetry(
    telemetry: &mut TelemetryCollector,
    view: &AuthoredBatchView,
    effective_now_ticks: TimelineTicks,
    lead_down_ticks: DurationTicks,
) -> Result<(), DispatchStep> {
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: view.batch_source_action_index,
                kind: TRACE_KIND_DOWN,
                outcome: trace_outcome_code("blocked_unfocused"),
                polyphony: view.batch_intent_count,
                flags: TRACE_FLAG_ANOMALY,
                win32_error: 0,
            },
            TraceTiming {
                authored_ticks: view.authored_batch_scheduled_ticks,
                effective_deadline_ticks: view.batch_scheduled_ticks,
                wake_ticks: effective_now_ticks,
                send_started_ticks: None,
                send_completed_ticks: None,
                bookkeeping_duration_us: 0,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
                applied_lead_ticks: lead_down_ticks,
            },
            TraceDelivery {
                requested: 0,
                sent: 0,
                skipped: 0,
                send_attempts: 0,
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
}

fn read_qpc_us(qpc_clock: QpcClock, clock_state: &PlaybackClockState) -> Result<u64, DispatchStep> {
    match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => DurationTicks::ZERO,
            }) {
                Ok(us) => Ok(us),
                Err(error) => Err(DispatchStep::Terminate(format!(
                    "QPC us conversion failure: {error:?}"
                ))),
            }
        }
        Err(error) => Err(DispatchStep::Terminate(format!("QPC failure: {error:?}"))),
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_release_send(
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    scan_codes: &SmallVec<[u16; 15]>,
) -> Result<ReleaseSend, DispatchStep> {
    let started_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "QPC failure before note-off: {error:?}"
            )));
        }
    };
    let started_us = match qpc_clock.duration_to_us(
        match started_ticks.checked_duration_since(clock_state.epoch) {
            Ok(dur) => dur,
            Err(_) => DurationTicks::ZERO,
        },
    ) {
        Ok(us) => us,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "QPC us conversion failure before note-off: {error:?}"
            )));
        }
    };
    let actual_ticks = match clock_state
        .get_elapsed_allow_pre_epoch(started_ticks, runtime.allow_pre_epoch_startup_dispatch)
    {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
    };
    let result = backend.key_up(scan_codes);
    if let Some(error) = backend.timing_error.take() {
        return Err(DispatchStep::Terminate(format!(
            "QPC failure after note-off: {error:?}"
        )));
    }
    let completed_qpc_ticks = match result.evidence.completed_ticks {
        Some(ticks) => ticks,
        None => {
            return Err(DispatchStep::Terminate(
                "SendInput note-off completed without a QPC completion boundary".to_string(),
            ));
        }
    };
    let completed_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        completed_qpc_ticks,
        runtime.allow_pre_epoch_startup_dispatch,
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
    };
    let sender_started_effective_ticks = match result.evidence.started_ticks {
        Some(ticks) => match clock_state
            .get_elapsed_allow_pre_epoch(ticks, runtime.allow_pre_epoch_startup_dispatch)
        {
            Ok(value) => Some(value),
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "playback clock failure: {error}"
                )));
            }
        },
        None => None,
    };
    let completed_effective_us = match qpc_clock.duration_to_us(
        match completed_effective_ticks.checked_duration_since(TimelineTicks::ZERO) {
            Ok(dur) => dur,
            Err(_) => DurationTicks::ZERO,
        },
    ) {
        Ok(us) => us,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock conversion failure: {error:?}"
            )));
        }
    };
    runtime.last_send_qpc_ticks = Some(completed_qpc_ticks);
    let sent_count = result.sent_scan_codes().len();
    let skipped_count = result.skipped_duplicates().len();
    let last_win32_error = result.evidence.last_win32_error;
    let completed_us = result.completed_us();
    let attempts = result.evidence.attempts;
    let is_success = result.is_success();
    Ok(ReleaseSend {
        started_us,
        actual_ticks,
        completed_effective_ticks,
        completed_effective_us,
        sender_started_effective_ticks,
        last_win32_error,
        completed_us,
        sent_count,
        skipped_count,
        attempts,
        is_success,
    })
}

#[allow(clippy::too_many_arguments)]
fn reconcile_release_recovery(
    coordinator: &mut RuntimeDispatchCoordinator,
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    timing: &WorkerTimingState,
    local_metrics: &mut WorkerMetricsLocal,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    lead_up_ticks: DurationTicks,
) -> Result<ReleaseReconciliation, DispatchStep> {
    let sent_codes: SmallVec<[u16; 15]> = due_pending.iter().map(|p| p.scan_code).collect();
    let skipped_codes: SmallVec<[u16; 15]> = SmallVec::new();
    let recovery_required = match coordinator.requeue_failed_releases_ticks(
        due_pending,
        sent_codes.as_slice(),
        skipped_codes.as_slice(),
        send.actual_ticks,
        send.completed_effective_ticks,
        &timing.retry_backoff_ticks,
        send.last_win32_error,
    ) {
        Ok(required) => required,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "coordinator recovery failure: {error}"
            )));
        }
    };
    let sent_codes2: SmallVec<[u16; 15]> = due_pending.iter().map(|p| p.scan_code).collect();
    let skipped_codes2: SmallVec<[u16; 15]> = SmallVec::new();
    if let Err(error) = coordinator.complete_releases(
        due_pending,
        sent_codes2.as_slice(),
        skipped_codes2.as_slice(),
    ) {
        return Err(DispatchStep::Terminate(format!(
            "coordinator release completion failure: {error}"
        )));
    }
    if !recovery_required {
        match coordinator.finish_release_recovery_ticks(send.completed_effective_ticks) {
            Ok(Some(recovery_pause_ticks)) => {
                let recovery_pause_us = match qpc_clock.duration_to_us(recovery_pause_ticks) {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(DispatchStep::Terminate(format!(
                            "recovery telemetry conversion failure: {error:?}"
                        )));
                    }
                };
                local_metrics.total_us = local_metrics.total_us.saturating_add(recovery_pause_us);
            }
            Ok(None) => {}
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "coordinator recovery completion failure: {error}"
                )));
            }
        }
    }
    reconcile_release_outcome(
        qpc_clock,
        clock_state,
        due_pending,
        send,
        lead_up_ticks,
        recovery_required,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconcile_release_outcome(
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    lead_up_ticks: DurationTicks,
    recovery_required: bool,
) -> Result<ReleaseReconciliation, DispatchStep> {
    let bookkeeping_completed_us = match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => DurationTicks::ZERO,
            }) {
                Ok(us) => us,
                Err(error) => {
                    return Err(DispatchStep::Terminate(format!(
                        "bookkeeping QPC us conversion failure: {error:?}"
                    )));
                }
            }
        }
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "bookkeeping QPC failure: {error:?}"
            )));
        }
    };
    let mut first_index: Option<usize> = None;
    let mut first_deadline: Option<TimelineTicks> = None;
    for (index, pending) in due_pending.iter().enumerate() {
        let deadline = match pending.get_effective_release_ticks(lead_up_ticks) {
            Ok(deadline) => deadline,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "pending release deadline failure: {error}"
                )));
            }
        };
        let is_better = match first_index {
            None => true,
            Some(best_index) => match first_deadline {
                Some(best) => {
                    (deadline, pending.source_action_index, pending.scan_code)
                        < (
                            best,
                            due_pending[best_index].source_action_index,
                            due_pending[best_index].scan_code,
                        )
                }
                None => {
                    return Err(DispatchStep::Terminate(
                        "pending release first-deadline state is inconsistent".to_string(),
                    ));
                }
            },
        };
        if is_better {
            first_index = Some(index);
            first_deadline = Some(deadline);
        }
    }
    let Some(first_index) = first_index else {
        return Err(DispatchStep::Terminate(
            "coordinator returned an empty pending release batch".to_string(),
        ));
    };
    let Some(effective_deadline_ticks) = first_deadline else {
        return Err(DispatchStep::Terminate(
            "coordinator returned no release deadline".to_string(),
        ));
    };
    let first = &due_pending[first_index];
    let Some(scheduled_ticks) = due_pending
        .iter()
        .map(|pending| pending.scheduled_release_ticks)
        .min()
    else {
        return Err(DispatchStep::Terminate(
            "pending release batch has no scheduled timestamp".to_string(),
        ));
    };
    let scheduled_us = match qpc_clock.timeline_to_us(scheduled_ticks) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "pending release telemetry conversion failure: {error:?}"
            )));
        }
    };
    let mut deferred_by_us = 0u64;
    for pending in due_pending {
        let ready_ticks = pending
            .release_not_before_ticks
            .max(pending.next_retry_ticks);
        let deferred_ticks = match ready_ticks
            .checked_duration_since(pending.scheduled_release_ticks)
        {
            Ok(value) => value,
            Err(sky_dispatch_core::time::TimeArithmeticError::NegativeOrder) => DurationTicks::ZERO,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "pending deferral arithmetic failure: {error}"
                )));
            }
        };
        let deferred_us = match qpc_clock.duration_to_us(deferred_ticks) {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "pending deferral conversion failure: {error:?}"
                )));
            }
        };
        deferred_by_us = deferred_by_us.max(deferred_us);
    }
    let mixed_source = due_pending.iter().any(|pending| {
        pending.source_action_index != first.source_action_index
            || pending.reason_id != first.reason_id
    });
    let up_completion_lateness_ticks = send
        .completed_effective_ticks
        .checked_duration_since(scheduled_ticks)
        .ok();
    let up_completion_error_ticks =
        match signed_timeline_delta_ticks(send.completed_effective_ticks, effective_deadline_ticks)
        {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-off timing conversion failure: {error}"
                )));
            }
        };
    let up_authored_completion_error_ticks =
        match signed_timeline_delta_ticks(send.completed_effective_ticks, scheduled_ticks) {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-off authored timing conversion failure: {error}"
                )));
            }
        };
    let up_completion_error_us = match signed_ticks_to_us(qpc_clock, up_completion_error_ticks) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-off timing conversion failure: {error}"
            )));
        }
    };
    let scan_count = due_pending.len();
    let clean_up_sample = send.is_success
        && send.sent_count == scan_count
        && send.skipped_count == 0
        && send.attempts == 1
        && deferred_by_us == 0
        && !mixed_source;
    Ok(ReleaseReconciliation {
        recovery_required,
        bookkeeping_completed_us,
        first_index,
        effective_deadline_ticks,
        scheduled_ticks,
        scheduled_us,
        deferred_by_us,
        up_completion_lateness_ticks,
        up_completion_error_ticks,
        up_authored_completion_error_ticks,
        up_completion_error_us,
        clean_up_sample,
    })
}

#[allow(clippy::too_many_arguments)]
fn record_release_telemetry(
    telemetry: &mut TelemetryCollector,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    reconciliation: &ReleaseReconciliation,
    lead_up_ticks: DurationTicks,
) -> Result<(), DispatchStep> {
    let first = &due_pending[reconciliation.first_index];
    let scan_count = due_pending.len();
    let release_outcome = release_runtime_outcome(
        reconciliation.deferred_by_us,
        send.sent_count,
        scan_count,
        reconciliation.recovery_required,
    );
    let mut trace_flags = 0u8;
    if send.sent_count == scan_count {
        trace_flags |= TRACE_FLAG_SENT_FULL;
    }
    if release_outcome == "deferred_release" || release_outcome == "failed_note_off" {
        trace_flags |= TRACE_FLAG_RECOVERY;
    }
    if reconciliation.deferred_by_us > 0 {
        trace_flags |= TRACE_FLAG_DEFERRED;
    }
    if release_outcome != "sent" {
        trace_flags |= TRACE_FLAG_ANOMALY;
    }
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: first.source_action_index,
                kind: TRACE_KIND_UP,
                outcome: trace_outcome_code(release_outcome),
                polyphony: scan_count,
                flags: trace_flags,
                win32_error: send.last_win32_error.unwrap_or(0),
            },
            TraceTiming {
                authored_ticks: reconciliation.scheduled_ticks,
                effective_deadline_ticks: reconciliation.effective_deadline_ticks,
                wake_ticks: send.actual_ticks,
                send_started_ticks: send.sender_started_effective_ticks,
                send_completed_ticks: Some(send.completed_effective_ticks),
                bookkeeping_duration_us: reconciliation
                    .bookkeeping_completed_us
                    .saturating_sub(send.completed_us),
                completion_error_ticks: reconciliation.up_completion_error_ticks,
                authored_completion_error_ticks: reconciliation.up_authored_completion_error_ticks,
                applied_lead_ticks: lead_up_ticks,
            },
            TraceDelivery {
                requested: scan_count,
                sent: send.sent_count,
                skipped: send.skipped_count,
                send_attempts: usize::from(send.attempts),
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn observe_release_send_health(
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    config: &WorkerConfig,
    timing: &WorkerTimingState,
    estimator: &mut SendLatencyEstimator,
    health: &mut WorkerHealthState,
    backend: &TrackedKeyState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    metrics: &SharedMetrics,
    last_published_error: &mut Option<String>,
    send: &ReleaseSend,
    reconciliation: &ReleaseReconciliation,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    lead_up: u64,
    latency_class: LatencyClass,
    pending_plan: Option<&PendingDispatchPlan>,
    scan_count: usize,
) -> Result<ReleaseOutcomeFlags, DispatchStep> {
    let up_saturated_positive = pending_plan.is_some_and(|plan| plan.lead_saturated)
        && reconciliation.up_completion_lateness_ticks.is_some();
    health.up_saturation_positive_streak = if up_saturated_positive {
        health.up_saturation_positive_streak.saturating_add(1)
    } else {
        0
    };
    let saturation_abort = config.timing.strict_timing
        && health.up_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK;
    let strict_up_completion_late = config.timing.strict_timing
        && reconciliation.clean_up_sample
        && reconciliation
            .up_completion_lateness_ticks
            .is_some_and(|late| late > timing.strict_up_completion_late_ticks);
    if config.estimator.enable_adaptive_lead && pending_plan.is_some_and(|plan| plan.lead_saturated)
    {
        record_lead_saturation(
            &mut local_metrics.lead_saturation_count_up,
            &mut local_metrics.positive_residual_at_cap,
            scan_count,
            signed_delta(send.completed_effective_us, reconciliation.scheduled_us),
        );
    }
    let send_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.send_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.bookkeeping_warn_threshold_us = frozen_budget.bookkeeping_warn_us;
    local_metrics.send_up_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    if config.estimator.enable_adaptive_lead
        && let Err(error) = update_estimator_after_send_class(
            estimator,
            ActionKind::Up,
            send.completed_us.saturating_sub(send.started_us),
            send.sent_count,
            scan_count,
            lead_up,
            reconciliation.up_completion_error_us,
            reconciliation.clean_up_sample,
            latency_class,
        )
    {
        return Err(DispatchStep::Terminate(format!(
            "estimator update failure: {error}"
        )));
    }
    record_lateness(
        signed_delta(send.completed_effective_us, reconciliation.scheduled_us),
        true,
        reconciliation.deferred_by_us > 0,
        local_metrics,
    );
    super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    let current_us = match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => DurationTicks::ZERO,
            }) {
                Ok(us) => us,
                Err(error) => {
                    return Err(DispatchStep::Terminate(format!(
                        "QPC us conversion failure: {error:?}"
                    )));
                }
            }
        }
        Err(error) => return Err(DispatchStep::Terminate(format!("QPC failure: {error:?}"))),
    };
    try_publish_metrics(
        local_metrics,
        metrics,
        current_us,
        !reconciliation.clean_up_sample || reconciliation.recovery_required,
    );
    let iteration_ready_us = match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => DurationTicks::ZERO,
            }) {
                Ok(us) => us,
                Err(error) => {
                    return Err(DispatchStep::Terminate(format!(
                        "QPC us conversion failure: {error:?}"
                    )));
                }
            }
        }
        Err(error) => return Err(DispatchStep::Terminate(format!("QPC failure: {error:?}"))),
    };
    runtime.pending_pre_send_spin_us = 0;
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: send.completed_us.saturating_sub(send.started_us),
            post_send_duration_us: iteration_ready_us.saturating_sub(send.completed_us),
            path: frozen_budget.path,
            send_warn_us: send_warn_threshold_us,
            bookkeeping_warn_us: frozen_budget.bookkeeping_warn_us,
            elapsed_us: send.completed_effective_us,
        },
        health.options.window_policy(),
        &mut health.send_pure_window,
        &mut health.bookkeeping_window,
        local_metrics,
    );
    Ok(ReleaseOutcomeFlags {
        strict_up_completion_late,
        saturation_abort,
    })
}

fn finalize_release_recovery(
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    runtime: &mut WorkerRuntime,
    secondary_errors: &mut Vec<String>,
    target_hwnd: &AtomicIsize,
    send: &ReleaseSend,
) -> DispatchStep {
    runtime.verified_target = None;
    runtime.force_full_cleanup = true;
    let mut term_err = Some(format!(
        "note-off recovery exhausted after {} retries{}",
        sky_dispatch_core::coordinator::MAX_RELEASE_RETRIES,
        send.last_win32_error
            .map_or(String::new(), |error| format!(" (Win32 error {error})"))
    ));
    let recovery_cleanup = backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire));
    if !release_state_verified(backend, &recovery_cleanup) {
        record_termination_error(
            &mut term_err,
            secondary_errors,
            format!(
                "recovery cleanup release verification failed: {}",
                describe_release_outcome(&recovery_cleanup)
            ),
        );
    }
    cancel_coordinator_or_terminal(
        coordinator,
        &mut runtime.force_full_cleanup,
        &mut term_err,
        secondary_errors,
    );
    DispatchStep::Terminate(term_err.unwrap_or_else(|| "recovery failure".to_string()))
}
