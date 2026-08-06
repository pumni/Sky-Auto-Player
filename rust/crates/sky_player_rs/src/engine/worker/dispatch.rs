use super::super::{
    ActionKind, DurationTicks, HARD_LATE_ABORT_THRESHOLD_US, LatencyClass, QpcTicks,
    RtTraceRecord, STRICT_SATURATION_ABORT_STREAK, SharedMetrics, TRACE_FLAG_ANOMALY,
    TRACE_FLAG_DEFERRED, TRACE_FLAG_RECOVERY, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN,
    TRACE_KIND_UP, TimelineTicks, TraceContext, TraceDelivery, TraceTiming, trace_outcome_code,
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
use sky_dispatch_core::coordinator::{PendingDispatchPlan, PendingRelease};
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

    let scan_codes: SmallVec<[u16; 15]> = due_pending.iter().map(|p| p.scan_code).collect();
    let frozen_budget = build_dispatch_budget(
        estimator,
        DispatchPath::UpOnly {
            up_count: scan_codes.len(),
        },
        latency_class,
        health.options,
        config.timing.strict_timing,
    );
    let started_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return DispatchStep::Terminate(format!("QPC failure before note-off: {error:?}"));
        }
    };
    let started_us = match qpc_clock.duration_to_us(
        match started_ticks.checked_duration_since(clock_state.epoch) {
            Ok(dur) => dur,
            Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
        },
    ) {
        Ok(us) => us,
        Err(error) => {
            return DispatchStep::Terminate(format!("QPC us conversion failure before note-off: {error:?}"));
        }
    };
    let actual_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        started_ticks,
        runtime.allow_pre_epoch_startup_dispatch,
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return DispatchStep::Terminate(format!("playback clock failure: {error}"));
        }
    };
    let result = backend.key_up(&scan_codes);
    if let Some(error) = backend.timing_error.take() {
        return DispatchStep::Terminate(format!("QPC failure after note-off: {error:?}"));
    }
    let completed_qpc_ticks = match result.evidence.completed_ticks {
        Some(ticks) => ticks,
        None => {
            return DispatchStep::Terminate(
                "SendInput note-off completed without a QPC completion boundary".to_string(),
            );
        }
    };
    let completed_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        completed_qpc_ticks,
        runtime.allow_pre_epoch_startup_dispatch,
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return DispatchStep::Terminate(format!("playback clock failure: {error}"));
        }
    };
    let sender_started_effective_ticks = match result.evidence.started_ticks {
        Some(ticks) => match clock_state.get_elapsed_allow_pre_epoch(
            ticks,
            runtime.allow_pre_epoch_startup_dispatch,
        ) {
            Ok(value) => Some(value),
            Err(error) => {
                return DispatchStep::Terminate(format!("playback clock failure: {error}"));
            }
        },
        None => None,
    };
    let completed_effective = match qpc_clock.duration_to_us(
        match completed_effective_ticks.checked_duration_since(TimelineTicks::ZERO) {
            Ok(dur) => dur,
            Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
        },
    ) {
        Ok(us) => us,
        Err(error) => {
            return DispatchStep::Terminate(format!("playback clock conversion failure: {error:?}"));
        }
    };
    runtime.last_send_qpc_ticks = Some(completed_qpc_ticks);
    let sent_codes = result.sent_scan_codes();
    let skipped_codes = result.skipped_duplicates();
    let recovery_required = match coordinator.requeue_failed_releases_ticks(
        &due_pending,
        sent_codes.as_slice(),
        skipped_codes.as_slice(),
        actual_ticks,
        completed_effective_ticks,
        &timing.retry_backoff_ticks,
        result.evidence.last_win32_error,
    ) {
        Ok(required) => required,
        Err(error) => {
            return DispatchStep::Terminate(format!("coordinator recovery failure: {error}"));
        }
    };
    if let Err(error) = coordinator.complete_releases(
        &due_pending,
        sent_codes.as_slice(),
        skipped_codes.as_slice(),
    ) {
        return DispatchStep::Terminate(format!("coordinator release completion failure: {error}"));
    }
    if !recovery_required {
        match coordinator.finish_release_recovery_ticks(completed_effective_ticks) {
            Ok(Some(recovery_pause_ticks)) => {
                let recovery_pause_us = match qpc_clock.duration_to_us(recovery_pause_ticks) {
                    Ok(value) => value,
                    Err(error) => {
                        return DispatchStep::Terminate(format!(
                            "recovery telemetry conversion failure: {error:?}"
                        ));
                    }
                };
                local_metrics.total_us =
                    local_metrics.total_us.saturating_add(recovery_pause_us);
            }
            Ok(None) => {}
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "coordinator recovery completion failure: {error}"
                ));
            }
        }
    }
    let bookkeeping_completed_us = match qpc_clock.now() {
        Ok(now) => match qpc_clock.duration_to_us(
            match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
            },
        ) {
            Ok(us) => us,
            Err(error) => return DispatchStep::Terminate(format!("bookkeeping QPC us conversion failure: {error:?}")),
        },
        Err(error) => return DispatchStep::Terminate(format!("bookkeeping QPC failure: {error:?}")),
    };
    let mut first_index: Option<usize> = None;
    let mut first_deadline: Option<TimelineTicks> = None;
    for (index, pending) in due_pending.iter().enumerate() {
        let deadline = match pending.get_effective_release_ticks(lead_up_ticks) {
            Ok(deadline) => deadline,
            Err(error) => {
                return DispatchStep::Terminate(format!("pending release deadline failure: {error}"));
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
                    return DispatchStep::Terminate(
                        "pending release first-deadline state is inconsistent".to_string(),
                    );
                }
            },
        };
        if is_better {
            first_index = Some(index);
            first_deadline = Some(deadline);
        }
    }
    let Some(first_index) = first_index else {
        return DispatchStep::Terminate("coordinator returned an empty pending release batch".to_string());
    };
    let Some(effective_deadline_ticks) = first_deadline else {
        return DispatchStep::Terminate("coordinator returned no release deadline".to_string());
    };
    let first = &due_pending[first_index];
    let Some(scheduled_ticks) = due_pending
        .iter()
        .map(|pending| pending.scheduled_release_ticks)
        .min()
    else {
        return DispatchStep::Terminate("pending release batch has no scheduled timestamp".to_string());
    };
    let scheduled_us = match qpc_clock.timeline_to_us(scheduled_ticks) {
        Ok(value) => value,
        Err(error) => {
            return DispatchStep::Terminate(format!(
                "pending release telemetry conversion failure: {error:?}"
            ));
        }
    };
    let mut deferred_by_us = 0u64;
    for pending in &due_pending {
        let ready_ticks = pending
            .release_not_before_ticks
            .max(pending.next_retry_ticks);
        let deferred_ticks =
            match ready_ticks.checked_duration_since(pending.scheduled_release_ticks) {
                Ok(value) => value,
                Err(sky_dispatch_core::time::TimeArithmeticError::NegativeOrder) => {
                    DurationTicks::ZERO
                }
                Err(error) => {
                    return DispatchStep::Terminate(format!(
                        "pending deferral arithmetic failure: {error}"
                    ));
                }
            };
        let deferred_us = match qpc_clock.duration_to_us(deferred_ticks) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "pending deferral conversion failure: {error:?}"
                ));
            }
        };
        deferred_by_us = deferred_by_us.max(deferred_us);
    }
    let mixed_source = due_pending.iter().any(|pending| {
        pending.source_action_index != first.source_action_index
            || pending.reason_id != first.reason_id
    });
    let up_completion_lateness_ticks = completed_effective_ticks
        .checked_duration_since(scheduled_ticks)
        .ok();
    let up_completion_error_ticks = match signed_timeline_delta_ticks(
        completed_effective_ticks,
        effective_deadline_ticks,
    ) {
        Ok(value) => value,
        Err(error) => {
            return DispatchStep::Terminate(format!("note-off timing conversion failure: {error}"));
        }
    };
    let up_authored_completion_error_ticks =
        match signed_timeline_delta_ticks(completed_effective_ticks, scheduled_ticks) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "note-off authored timing conversion failure: {error}"
                ));
            }
        };
    let up_completion_error_us =
        match signed_ticks_to_us(*qpc_clock, up_completion_error_ticks) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!("note-off timing conversion failure: {error}"));
            }
        };
    let clean_up_sample = result.is_success()
        && result.sent_scan_codes().len() == scan_codes.len()
        && result.skipped_duplicates().is_empty()
        && result.evidence.attempts == 1
        && deferred_by_us == 0
        && !mixed_source;
    let strict_up_completion_late = config.timing.strict_timing
        && clean_up_sample
        && up_completion_lateness_ticks
            .is_some_and(|late| late > timing.strict_up_completion_late_ticks);
    let up_saturated_positive = pending_plan
        .as_ref()
        .is_some_and(|plan| plan.lead_saturated)
        && up_completion_lateness_ticks.is_some();
    health.up_saturation_positive_streak = if up_saturated_positive {
        health.up_saturation_positive_streak.saturating_add(1)
    } else {
        0
    };
    let saturation_abort = config.timing.strict_timing
        && health.up_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK;
    let release_outcome = if strict_up_completion_late {
        "strict_completion_slo_exceeded"
    } else {
        release_runtime_outcome(
            deferred_by_us,
            result.sent_scan_codes().len(),
            scan_codes.len(),
            recovery_required,
        )
    };
    let mut trace_flags = 0;
    if result.sent_scan_codes().len() == scan_codes.len() {
        trace_flags |= TRACE_FLAG_SENT_FULL;
    }
    if release_outcome == "deferred_release" || release_outcome == "failed_note_off" {
        trace_flags |= TRACE_FLAG_RECOVERY;
    }
    if deferred_by_us > 0 {
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
                polyphony: scan_codes.len(),
                flags: trace_flags,
                win32_error: result.evidence.last_win32_error.unwrap_or(0),
            },
            TraceTiming {
                authored_ticks: scheduled_ticks,
                effective_deadline_ticks,
                wake_ticks: actual_ticks,
                send_started_ticks: sender_started_effective_ticks,
                send_completed_ticks: Some(completed_effective_ticks),
                bookkeeping_duration_us: bookkeeping_completed_us
                    .saturating_sub(result.completed_us()),
                completion_error_ticks: up_completion_error_ticks,
                authored_completion_error_ticks: up_authored_completion_error_ticks,
                applied_lead_ticks: lead_up_ticks,
            },
            TraceDelivery {
                requested: scan_codes.len(),
                sent: result.sent_scan_codes().len(),
                skipped: result.skipped_duplicates().len(),
                send_attempts: usize::from(result.evidence.attempts),
            },
        )
    }) {
        return DispatchStep::Terminate(format!("native telemetry record overflow: {error}"));
    }
    if config.estimator.enable_adaptive_lead
        && pending_plan
            .as_ref()
            .is_some_and(|plan| plan.lead_saturated)
    {
        record_lead_saturation(
            &mut local_metrics.lead_saturation_count_up,
            &mut local_metrics.positive_residual_at_cap,
            scan_codes.len(),
            signed_delta(completed_effective, scheduled_us),
        );
    }
    runtime.pending_pre_send_spin_us = 0;
    let send_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.send_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.bookkeeping_warn_threshold_us = frozen_budget.bookkeeping_warn_us;
    local_metrics.send_up_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    let send_duration_us = result.completed_us().saturating_sub(started_us);
    if config.estimator.enable_adaptive_lead
        && let Err(error) = update_estimator_after_send_class(
            estimator,
            ActionKind::Up,
            result.completed_us().saturating_sub(started_us),
            result.sent_scan_codes().len(),
            scan_codes.len(),
            lead_up,
            up_completion_error_us,
            clean_up_sample,
            latency_class,
        )
    {
        return DispatchStep::Terminate(format!("estimator update failure: {error}"));
    }
    let deferred_release = deferred_by_us > 0;
    record_lateness(
        signed_delta(completed_effective, scheduled_us),
        true,
        deferred_release,
        local_metrics,
    );
    super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    let current_us = match qpc_clock.now() {
        Ok(now) => match qpc_clock.duration_to_us(
            match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
            },
        ) {
            Ok(us) => us,
            Err(error) => return DispatchStep::Terminate(format!("QPC us conversion failure: {error:?}")),
        },
        Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
    };
    try_publish_metrics(
        local_metrics,
        metrics,
        current_us,
        !clean_up_sample || recovery_required,
    );
    let iteration_ready_us = match qpc_clock.now() {
        Ok(now) => match qpc_clock.duration_to_us(
            match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
            },
        ) {
            Ok(us) => us,
            Err(error) => return DispatchStep::Terminate(format!("QPC us conversion failure: {error:?}")),
        },
        Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
    };
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us,
            post_send_duration_us: iteration_ready_us.saturating_sub(result.completed_us()),
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
    if recovery_required {
        runtime.verified_target = None;
        runtime.force_full_cleanup = true;
        let mut term_err = Some(format!(
            "note-off recovery exhausted after {} retries{}",
            sky_dispatch_core::coordinator::MAX_RELEASE_RETRIES,
            result
                .evidence
                .last_win32_error
                .map_or(String::new(), |error| format!(" (Win32 error {error})"))
        ));
        let recovery_cleanup =
            backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire));
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
        return DispatchStep::Terminate(term_err.unwrap_or_else(|| "recovery failure".to_string()));
    }
    if strict_up_completion_late {
        return DispatchStep::Terminate(format!(
            "strict timing completion SLO exceeded for note-off at action {}: completion was {}us late",
            first.source_action_index, up_completion_error_us
        ));
    }
    if saturation_abort {
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

    let (lead_down, lead_down_saturated, lead_down_ticks) = match dispatch_plan.authored.as_ref() {
        Some(authored) => (authored.lead_us, authored.lead_saturated, authored.lead_ticks),
        None => (0, false, DurationTicks::ZERO),
    };

    let prepared_batch = match coordinator.prepare_next_due_authored(effective_now_ticks, lead_down_ticks) {
        Ok(value) => value,
        Err(error) => {
            return DispatchStep::Terminate(format!("coordinator authored-prepare failure: {error}"));
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
                return DispatchStep::Terminate(format!("runtime packet view failure: {error}"));
            }
        };
        let conflict_mask = coordinator.check_packet_down_conflicts(
            packet_view.up_mask(),
            packet_view.down_intents,
        );
        let up_count = packet_view.up_mask().count_ones() as usize;
        let down_count = packet_view.down_mask().count_ones() as usize;
        let dispatch_path = match prepared_batch.packet_kind {
            Some(sky_dispatch_core::model::PhysicalPacketKind::UpOnly) => {
                DispatchPath::UpOnly { up_count }
            }
            Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly) => {
                DispatchPath::DownOnly { down_count }
            }
            Some(sky_dispatch_core::model::PhysicalPacketKind::Mixed) => {
                DispatchPath::Mixed {
                    up_count,
                    down_count,
                }
            }
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
                return DispatchStep::Terminate(format!("runtime schedule view failure: {error}"));
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
            return DispatchStep::Terminate(format!("schedule telemetry conversion failure: {error:?}"));
        }
    };
    let authored_batch_scheduled_ticks = coordinator.batch_scheduled_ticks[batch_index];
    let authored_batch_scheduled_us =
        match qpc_clock.timeline_to_us(authored_batch_scheduled_ticks) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "authored schedule telemetry conversion failure: {error:?}"
                ));
            }
        };
    let has_conflicts = conflict_mask != 0;

    if packet_masks.is_some() || batch_kind == ActionKind::Down {
        if !packet_masks.is_some_and(|packet| packet.down_mask == 0)
            && !focus_matches(config.focus.require_focus, focus_active, target_hwnd)
        {
            if let Err(error) = suspend_live_input(
                backend,
                coordinator,
                target_hwnd.load(Ordering::Acquire),
            ) {
                return DispatchStep::Terminate(format!("focus suspension failed: {error}"));
            }
            if let Err(error) = clock_state.enter_pause("focus", now_ticks) {
                return DispatchStep::Terminate(format!("playback clock failure: {error}"));
            }
            runtime.focus_restore_started_ticks = None;
            if let Err(error) = telemetry.try_push(|| {
                RtTraceRecord::dispatched(
                    TraceContext {
                        event_index: batch_source_action_index,
                        kind: TRACE_KIND_DOWN,
                        outcome: trace_outcome_code("blocked_unfocused"),
                        polyphony: batch_intent_count,
                        flags: TRACE_FLAG_ANOMALY,
                        win32_error: 0,
                    },
                    TraceTiming {
                        authored_ticks: authored_batch_scheduled_ticks,
                        effective_deadline_ticks: batch_scheduled_ticks,
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
                return DispatchStep::Terminate(format!("native telemetry record overflow: {error}"));
            }
            super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
            let current_us = match qpc_clock.now() {
                Ok(now) => match qpc_clock.duration_to_us(
                    match now.checked_duration_since(clock_state.epoch) {
                        Ok(dur) => dur,
                        Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
                    },
                ) {
                    Ok(us) => us,
                    Err(error) => return DispatchStep::Terminate(format!("QPC us conversion failure: {error:?}")),
                },
                Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
            };
            try_publish_metrics(local_metrics, metrics, current_us, true);
            return DispatchStep::Continue;
        }
        if !packet_masks.is_some_and(|packet| packet.down_mask == 0)
            && focus_loss_fault
            && !runtime.focus_loss_fault_injected
        {
            runtime.focus_loss_fault_injected = true;
            return DispatchStep::Terminate(
                "focus lost after due check before SendInput boundary".to_string(),
            );
        }
        let preflight_target = load_target_stamp(target_hwnd, target_generation);
        if let Err(error) = ensure_preflight_for_target(
            backend,
            preflight_target,
            &mut runtime.verified_target,
        ) {
            runtime.verified_target = None;
            return DispatchStep::Terminate(format!(
                "instrument key preflight failed; release the 15 instrument keys before playback: {error}"
            ));
        }
        if !target_stamp_still_current(target_hwnd, target_generation, preflight_target) {
            runtime.verified_target = None;
            return DispatchStep::Continue;
        }
        if config.timing.strict_timing
            && effective_now_ticks
                .checked_duration_since(authored_batch_scheduled_ticks)
                .is_ok_and(|late| late > timing.hard_late_abort_threshold_ticks)
        {
            return DispatchStep::Terminate(format!(
                "authored Down exceeded hard lateness safety threshold of {}us",
                HARD_LATE_ABORT_THRESHOLD_US
            ));
        }
        if has_conflicts {
            local_metrics.authored_conflict_events =
                local_metrics.authored_conflict_events.saturating_add(1);
            local_metrics.authored_chords_rejected =
                local_metrics.authored_chords_rejected.saturating_add(1);
            local_metrics.authored_keys_rejected = local_metrics
                .authored_keys_rejected
                .saturating_add(batch_intent_count as u64);
            return DispatchStep::Terminate(format!(
                "unexpected blocked authored Down at action {}",
                batch_source_action_index
            ));
        }

        if packet_masks.is_some() || !scan_batch.is_empty() {
            let admission = if packet_masks.is_some_and(|packet| packet.down_mask == 0) {
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
                        Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
                    };
                    if let Err(error) = suspend_live_input(
                        backend,
                        coordinator,
                        target_hwnd.load(Ordering::Acquire),
                    ) {
                        return DispatchStep::Terminate(format!("focus suspension failed: {error}"));
                    }
                    if let Err(error) = clock_state.enter_pause("focus", focus_ticks) {
                        return DispatchStep::Terminate(format!(
                            "playback clock failure after final focus check: {error}"
                        ));
                    }
                    runtime.focus_restore_started_ticks = None;
                    super::publish_backend_metrics(
                        backend,
                        local_metrics,
                        metrics,
                        last_published_error,
                    );
                    let current_us = match qpc_clock.now() {
                        Ok(now) => match qpc_clock.duration_to_us(
                            match now.checked_duration_since(clock_state.epoch) {
                                Ok(dur) => dur,
                                Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
                            },
                        ) {
                            Ok(us) => us,
                            Err(error) => return DispatchStep::Terminate(format!("QPC us conversion failure: {error:?}")),
                        },
                        Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
                    };
                    try_publish_metrics(local_metrics, metrics, current_us, true);
                    return DispatchStep::Continue;
                }
                DownAdmission::TargetChanged
                | DownAdmission::PauseRequested
                | DownAdmission::QuitRequested
                | DownAdmission::SkipRequested
                | DownAdmission::PanicRequested => {
                    runtime.verified_target = None;
                    return DispatchStep::Continue;
                }
            }

            let frozen_budget = build_dispatch_budget(
                estimator,
                dispatch_path,
                latency_class,
                health.options,
                config.timing.strict_timing,
            );
            let result = if let Some(packet) = packet_masks {
                backend.key_down_physical_packet(packet)
            } else {
                backend.key_down(scan_batch.as_slice())
            };
            if let Some(error) = backend.timing_error.take() {
                return DispatchStep::Terminate(format!("QPC failure after note-on: {error:?}"));
            }

            let trace_kind = match prepared_batch.packet_kind {
                Some(sky_dispatch_core::model::PhysicalPacketKind::UpOnly) => TRACE_KIND_UP,
                Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly) => TRACE_KIND_DOWN,
                Some(sky_dispatch_core::model::PhysicalPacketKind::Mixed) => TRACE_KIND_MIXED,
                None => TRACE_KIND_DOWN,
            };

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
                    batch_source_action_index
                ));
            }

            let sender_started_ticks = match result_started_ticks {
                Some(ticks) => ticks,
                None => {
                    return DispatchStep::Terminate(
                        "SendInput note-on succeeded without a QPC start boundary".to_string(),
                    );
                }
            };
            let completed_qpc_ticks = match result_completed_ticks {
                Some(ticks) => ticks,
                None => {
                    return DispatchStep::Terminate(
                        "SendInput note-on completed without a QPC completion boundary".to_string(),
                    );
                }
            };
            let sender_duration_ticks = match completed_qpc_ticks
                .checked_duration_since(sender_started_ticks)
            {
                Ok(duration) => duration,
                Err(error) => {
                    return DispatchStep::Terminate(format!("note-on QPC ordering failure: {error}"));
                }
            };
            let sender_duration_us = match qpc_clock.duration_to_us(sender_duration_ticks) {
                Ok(duration) => duration,
                Err(error) => {
                    return DispatchStep::Terminate(format!(
                        "note-on sender duration conversion failure: {error:?}"
                    ));
                }
            };
            let sender_started_effective_ticks = match clock_state
                .get_elapsed_allow_pre_epoch(
                    sender_started_ticks,
                    runtime.allow_pre_epoch_startup_dispatch,
                ) {
                Ok(ticks) => ticks,
                Err(error) => {
                    return DispatchStep::Terminate(format!("playback clock failure: {error}"));
                }
            };
            let completed_effective_ticks = match clock_state
                .get_elapsed_allow_pre_epoch(
                    completed_qpc_ticks,
                    runtime.allow_pre_epoch_startup_dispatch,
                ) {
                Ok(ticks) => ticks,
                Err(error) => {
                    return DispatchStep::Terminate(format!("playback clock failure: {error}"));
                }
            };
            let completed_effective = match qpc_clock.duration_to_us(
                match completed_effective_ticks.checked_duration_since(TimelineTicks::ZERO) {
                    Ok(dur) => dur,
                    Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
                },
            ) {
                Ok(us) => us,
                Err(error) => return DispatchStep::Terminate(format!("playback clock conversion failure: {error:?}")),
            };
            runtime.last_send_qpc_ticks = Some(completed_qpc_ticks);
            let commit_result = if packet_mode {
                coordinator.commit_packet_success(
                    prepared_batch,
                    sender_started_effective_ticks,
                    completed_effective_ticks,
                )
            } else {
                coordinator.commit_down_success(
                    prepared_batch,
                    &result_sent,
                    sender_started_effective_ticks,
                    completed_effective_ticks,
                )
            };
            if let Err(error) = commit_result {
                return DispatchStep::Terminate(format!("coordinator activation failure: {error}"));
            }
            let completion_lateness_ticks = completed_effective_ticks
                .checked_duration_since(batch_scheduled_ticks)
                .ok();
            let completion_error_ticks_value = match signed_timeline_delta_ticks(
                completed_effective_ticks,
                batch_scheduled_ticks,
            ) {
                Ok(value) => value,
                Err(error) => {
                    return DispatchStep::Terminate(format!("note-on timing conversion failure: {error}"));
                }
            };
            let authored_completion_error_ticks_value = match signed_timeline_delta_ticks(
                completed_effective_ticks,
                authored_batch_scheduled_ticks,
            ) {
                Ok(value) => value,
                Err(error) => {
                    return DispatchStep::Terminate(format!(
                        "note-on authored timing conversion failure: {error}"
                    ));
                }
            };
            let completion_error_us =
                match signed_ticks_to_us(*qpc_clock, completion_error_ticks_value) {
                    Ok(value) => value,
                    Err(error) => {
                        return DispatchStep::Terminate(format!("note-on timing conversion failure: {error}"));
                    }
                };
            let requested_count = dispatch_path.event_count();
            let delivered_count = if packet_mode {
                usize::from(result_success) * requested_count
            } else {
                result_sent.len()
            };
            let estimator_kind = estimator_kind_for_path(dispatch_path);
            let clean_directional_sample = result_success
                && result_skipped_duplicates.is_empty()
                && result_send_attempts == 1
                && !result_chord_integrity_lost
                && !matches!(dispatch_path, DispatchPath::Mixed { .. })
                && estimator_kind.is_some()
                && delivered_count == requested_count;
            let recovered_zero_progress = matches!(
                result_retry_reason,
                sky_dispatch_win32::input::PacketRetryReason::ZeroProgress
            );
            let recovered_partial_up = matches!(
                (dispatch_path, result_retry_reason),
                (
                    DispatchPath::UpOnly { .. },
                    sky_dispatch_win32::input::PacketRetryReason::PartialProgress { .. }
                )
            ) && result_success;
            let recovered_retry_late = recovered_zero_progress
                && result_success
                && completion_lateness_ticks
                    .is_some_and(|late| late > timing.retry_late_threshold_ticks);
            let retry_late_abort = config.timing.strict_timing && recovered_retry_late;
            let strict_completion_late = config.timing.strict_timing
                && clean_directional_sample
                && completion_lateness_ticks.is_some_and(|late| {
                    late > match dispatch_path {
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
            let saturation_abort = match dispatch_path {
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
            let bookkeeping_completed_us = match qpc_clock.now() {
                Ok(now) => match qpc_clock.duration_to_us(
                    match now.checked_duration_since(clock_state.epoch) {
                        Ok(dur) => dur,
                        Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
                    },
                ) {
                    Ok(us) => us,
                    Err(error) => return DispatchStep::Terminate(format!("bookkeeping QPC us conversion failure: {error:?}")),
                },
                Err(error) => return DispatchStep::Terminate(format!("bookkeeping QPC failure: {error:?}")),
            };
            if recovered_zero_progress && result_success {
                local_metrics.recovered_zero_progress_retries = local_metrics
                    .recovered_zero_progress_retries
                    .saturating_add(1);
            }
            if recovered_partial_up {
                local_metrics.recovered_partial_up_retries =
                    local_metrics.recovered_partial_up_retries.saturating_add(1);
            }
            let down_outcome = if recovered_retry_late {
                "recovered_zero_progress_but_late"
            } else if recovered_partial_up {
                "recovered_partial_up_retry"
            } else if strict_completion_late {
                "strict_completion_slo_exceeded"
            } else if result_chord_integrity_lost {
                "chord_integrity_lost"
            } else if packet_masks.is_some_and(|_| result_success)
                || (packet_masks.is_none() && result_sent.len() == scan_batch.len())
            {
                "sent"
            } else {
                "partial_note_on"
            };
            let mut force_dispatch_publish = !result_success
                || !matches!(
                    result_retry_reason,
                    sky_dispatch_win32::input::PacketRetryReason::None
                )
                || result_chord_integrity_lost;
            let mut trace_flags = 0;
            let send_completed_fully = packet_masks.is_some_and(|_| result_success)
                || (packet_masks.is_none() && result_sent.len() == scan_batch.len());
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
                        event_index: batch_source_action_index,
                        kind: trace_kind,
                        outcome: trace_outcome_code(down_outcome),
                        polyphony: batch_intent_count,
                        flags: trace_flags,
                        win32_error: result_last_win32_error.unwrap_or(0),
                    },
                    TraceTiming {
                        authored_ticks: authored_batch_scheduled_ticks,
                        effective_deadline_ticks: batch_scheduled_ticks,
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
                        requested: batch_intent_count,
                        sent: if packet_masks.is_some() && result_success {
                            requested_count
                        } else {
                            result_sent.len()
                        },
                        skipped: result_skipped_duplicates.len(),
                        send_attempts: usize::from(result_send_attempts),
                    },
                )
            }) {
                return DispatchStep::Terminate(format!("native telemetry record overflow: {error}"));
            }
            if config.estimator.enable_adaptive_lead && lead_down_saturated {
                match dispatch_path {
                    DispatchPath::UpOnly { .. } => record_lead_saturation(
                        &mut local_metrics.lead_saturation_count_up,
                        &mut local_metrics.positive_residual_at_cap,
                        batch_intent_count,
                        signed_delta(completed_effective, batch_scheduled_us),
                    ),
                    DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                        record_lead_saturation(
                            &mut local_metrics.lead_saturation_count_down,
                            &mut local_metrics.positive_residual_at_cap,
                            batch_intent_count,
                            signed_delta(completed_effective, batch_scheduled_us),
                        )
                    }
                }
            }
            runtime.pending_pre_send_spin_us = 0;
            let send_warn_threshold_us = frozen_budget.send_warn_us;
            local_metrics.send_warn_threshold_us = frozen_budget.send_warn_us;
            local_metrics.bookkeeping_warn_threshold_us = frozen_budget.bookkeeping_warn_us;
            match dispatch_path {
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
                    batch_intent_count,
                    lead_down,
                    completion_error_us,
                    clean_directional_sample,
                    latency_class,
                )
            {
                return DispatchStep::Terminate(format!("estimator update failure: {error}"));
            }
            record_lateness(
                signed_delta(completed_effective, authored_batch_scheduled_us),
                false,
                false,
                local_metrics,
            );
            let terminal_dispatch =
                result_chord_integrity_lost || retry_late_abort || strict_completion_late || saturation_abort;
            if terminal_dispatch {
                force_dispatch_publish = true;
            }
            super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
            let current_us = match qpc_clock.now() {
                Ok(now) => match qpc_clock.duration_to_us(
                    match now.checked_duration_since(clock_state.epoch) {
                        Ok(dur) => dur,
                        Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
                    },
                ) {
                    Ok(us) => us,
                    Err(error) => return DispatchStep::Terminate(format!("QPC us conversion failure: {error:?}")),
                },
                Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
            };
            try_publish_metrics(local_metrics, metrics, current_us, force_dispatch_publish);
            let iteration_ready_us = match qpc_clock.now() {
                Ok(now) => match qpc_clock.duration_to_us(
                    match now.checked_duration_since(clock_state.epoch) {
                        Ok(dur) => dur,
                        Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
                    },
                ) {
                    Ok(us) => us,
                    Err(error) => return DispatchStep::Terminate(format!("QPC us conversion failure: {error:?}")),
                },
                Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
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
                    batch_source_action_index
                ));
            }
            if retry_late_abort {
                return DispatchStep::Terminate(format!(
                    "strict timing rejected zero-progress retry at action {}: completion was {}us late",
                    batch_source_action_index, completion_error_us
                ));
            }
            if strict_completion_late {
                let timing_label = if matches!(dispatch_path, DispatchPath::UpOnly { .. }) {
                    "note-off"
                } else {
                    "note-on"
                };
                return DispatchStep::Terminate(format!(
                    "strict timing completion SLO exceeded for {timing_label} at action {}: completion was {}us late",
                    batch_source_action_index, completion_error_us
                ));
            }
            if saturation_abort {
                let timing_label = if matches!(dispatch_path, DispatchPath::UpOnly { .. }) {
                    "note-off"
                } else {
                    "note-on"
                };
                return DispatchStep::Terminate(format!(
                    "strict timing SLO exceeded: {timing_label} lead saturated with positive residual for {} consecutive dispatches",
                    STRICT_SATURATION_ABORT_STREAK
                ));
            }
        }
    } else {
        let (_, suppressed) = match coordinator.commit_up_request(prepared_batch) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!("coordinator note-off request failure: {error}"));
            }
        };
        if !suppressed.is_empty()
            && let Err(error) = telemetry.try_push(|| {
                RtTraceRecord::dispatched(
                    TraceContext {
                        event_index: batch_source_action_index,
                        kind: TRACE_KIND_UP,
                        outcome: trace_outcome_code("suppressed_stale_up"),
                        polyphony: suppressed.len(),
                        flags: TRACE_FLAG_ANOMALY,
                        win32_error: 0,
                    },
                    TraceTiming {
                        authored_ticks: authored_batch_scheduled_ticks,
                        effective_deadline_ticks: batch_scheduled_ticks,
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
            Ok(now) => match qpc_clock.duration_to_us(
                match now.checked_duration_since(clock_state.epoch) {
                    Ok(dur) => dur,
                    Err(_) => sky_dispatch_core::time::DurationTicks::ZERO,
                },
            ) {
                Ok(us) => us,
                Err(error) => return DispatchStep::Terminate(format!("QPC us conversion failure: {error:?}")),
            },
            Err(error) => return DispatchStep::Terminate(format!("QPC failure: {error:?}")),
        };
        try_publish_metrics(
            local_metrics,
            metrics,
            current_us,
            !suppressed.is_empty(),
        );
    }

    DispatchStep::Dispatched
}
