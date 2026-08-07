use super::super::{
    ActionKind, DurationTicks, LatencyClass, PlaybackClockState, QpcClock, QpcTicks, RtTraceRecord,
    RuntimeDispatchCoordinator, STRICT_SATURATION_ABORT_STREAK, SendLatencyEstimator,
    SharedMetrics, TRACE_FLAG_ANOMALY, TRACE_FLAG_RECOVERY, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN,
    TRACE_KIND_UP, TelemetryCollector, TimelineTicks, TraceContext, TraceDelivery, TraceTiming,
    TrackedKeyState, trace_outcome_code, try_publish_metrics,
};
use super::downs::{AuthoredBatchView, DispatchStep};
use super::{
    DispatchHealthObservation, DispatchPath, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    WorkerRuntime, WorkerTimingState, estimator_kind_for_path, observe_dispatch_health,
    record_lateness, record_lead_saturation, signed_delta, signed_ticks_to_us,
    signed_timeline_delta_ticks, update_estimator_after_send_class,
};
use sky_dispatch_win32::input::PacketRetryReason;
use smallvec::SmallVec;

/// Owner of: telemetry record, estimator update, lateness derivation, metric
/// publish, dispatch health observation, and SLO terminal decisions for the
/// note-on send.  All values derived from `interpret_down_send_timing` are
/// already snapshotted — this function performs no further QPC resolution
/// beyond the two wall-clock samples needed for metric publication and the
/// iteration-ready boundary.
#[allow(clippy::too_many_arguments)]
pub(super) fn publisher_down_send_outcome(
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
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
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
        completed_effective,
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
            post_send_duration_us: iteration_ready_us.saturating_sub(completed_effective),
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
pub(super) fn record_down_send_telemetry(
    view: &AuthoredBatchView,
    telemetry: &mut TelemetryCollector,
    trace_kind: u8,
    effective_now_ticks: TimelineTicks,
    lead_down_ticks: DurationTicks,
    result_success: bool,
    completed_effective: u64,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
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
        || !matches!(result_retry_reason, PacketRetryReason::None)
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
                    .saturating_sub(completed_effective),
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
pub(super) fn commit_suppressed_up_request(
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
    let current_us = match read_qpc_us(qpc_clock, clock_state) {
        Ok(us) => us,
        Err(step) => return step,
    };
    try_publish_metrics(local_metrics, metrics, current_us, !suppressed.is_empty());
    DispatchStep::Dispatched
}

pub(super) fn record_blocked_unfocused_telemetry(
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

/// Timing-derived evidence captured from the note-on SendInput call:
/// projections used across telemetry, estimator update, and the terminal
/// SLO decision.
pub(super) struct DownSendTiming {
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
pub(super) fn interpret_down_send_timing(
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
    result_retry_reason: PacketRetryReason,
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
    let recovered_zero_progress = matches!(result_retry_reason, PacketRetryReason::ZeroProgress);
    let recovered_partial_up = matches!(
        (view.dispatch_path, result_retry_reason),
        (
            DispatchPath::UpOnly { .. },
            PacketRetryReason::PartialProgress { .. }
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

pub(super) fn read_qpc_us(
    qpc_clock: QpcClock,
    clock_state: &PlaybackClockState,
) -> Result<u64, DispatchStep> {
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
