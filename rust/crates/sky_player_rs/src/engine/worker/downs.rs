use super::super::{
    ActionKind, DurationTicks, HARD_LATE_ABORT_THRESHOLD_US, LatencyClass, PlaybackClockState,
    QpcClock, QpcTicks, RuntimeDispatchCoordinator, SendLatencyEstimator, SharedMetrics,
    TRACE_KIND_DOWN, TRACE_KIND_UP, TelemetryCollector, TimelineTicks, TrackedKeyState,
    try_publish_metrics,
};
use super::down_outcome::{
    commit_suppressed_up_request, interpret_down_send_timing, publisher_down_send_outcome,
    read_qpc_us, record_blocked_unfocused_telemetry,
};
use super::{
    DispatchPath, DownAdmission, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    WorkerResources, WorkerRuntime, WorkerTimingState, build_dispatch_budget,
    ensure_preflight_for_target, final_down_admission, focus_matches, load_target_stamp,
    planning::NextDispatchPlan, suspend_live_input, target_stamp_still_current,
};
use crate::engine::telemetry::TRACE_KIND_MIXED;
use sky_dispatch_core::coordinator::PreparedBatch;
use sky_dispatch_core::model::ScanCodeBatch;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

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
/// Built once per authored epoch by `prepare_authored_batch_view`; the
/// send/admission/telemetry helpers consume it without re-querying the
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
        result_sent,
        result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
        &timing_proof,
    )
}
