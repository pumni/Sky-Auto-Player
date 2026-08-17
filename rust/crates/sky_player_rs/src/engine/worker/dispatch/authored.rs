use super::super::super::{
    ActionKind, DurationTicks, HARD_LATE_ABORT_THRESHOLD_US, PlaybackClockState, QpcClock,
    QpcTicks, RuntimeDispatchCoordinator, TRACE_KIND_DOWN, TRACE_KIND_UP, TimelineTicks,
    TrackedKeyState,
};
use super::super::{
    DispatchPath, DownAdmission, FinalControlAdmission, FinalControlSignals, FinalTargetSignals,
    TargetStamp, WorkerConfig, WorkerHealthState, WorkerMetricsLocal, WorkerResources,
    WorkerRuntime, WorkerTimingState, final_control_admission_at, final_control_precheck,
    final_down_target_admission, focus_matches, load_target_stamp, record_sendinput_entry_lateness,
    signed_ticks_to_us, suspend_live_input, target_stamp_still_current,
};
use super::observation::BlockedUnfocusedObservation;
use super::observer::publisher_down_send_outcome;
use super::timing::interpret_down_send_timing;
use super::{AuthoredBatchView, AuthoredPacketContext, DispatchStep, PendingObservationQueue};
use crate::engine::shared::SharedProgressClock;
use crate::engine::telemetry::TRACE_KIND_MIXED;
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
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    progress_clock: &SharedProgressClock,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let AuthoredPacketContext {
        dispatch_plan,
        effective_now_ticks,
        now_ticks,
        physical_target_qpc,
        startup_target_selected,
        focus_loss_fault,
        interrupt,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
    } = ctx;

    let WorkerResources {
        clock: qpc_clock,
        waiter,
        backend,
        coordinator,
        playback: clock_state,
        ..
    } = resources;
    let qpc_clock = *qpc_clock;

    // The first physical boundary may be reached from the pre-roll wait. The
    // observation consumer must be allowed to interpret that wake as the
    // intentional pre-epoch startup handoff; the authored send itself still
    // cannot occur before its absolute target.
    runtime.allow_pre_epoch_startup_dispatch = now_ticks < clock_state.epoch;

    let Some(physical_plan) = dispatch_plan.physical() else {
        return DispatchStep::NoWork;
    };
    let view = &physical_plan.authored_view;
    commit_down_send_outcome(
        view,
        config,
        health,
        timing,
        runtime,
        local_metrics,
        focus_active,
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        progress_clock,
        qpc_clock,
        waiter,
        backend,
        coordinator,
        clock_state,
        effective_now_ticks,
        now_ticks,
        physical_target_qpc,
        startup_target_selected,
        focus_loss_fault,
        physical_plan.target_proof.verified_target(),
        interrupt,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
        observer,
    )
}

/// Admission gate + SendInput call + telemetry + health
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
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    progress_clock: &SharedProgressClock,
    qpc_clock: QpcClock,
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    physical_target_qpc: QpcTicks,
    _startup_target_selected: bool,
    focus_loss_fault: bool,
    preflight_target: Option<TargetStamp>,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let has_conflicts = view.conflict_mask != 0;
    let admission = match admit_authored_down(
        view,
        config,
        backend,
        coordinator,
        clock_state,
        runtime,
        local_metrics,
        focus_active,
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        progress_clock,
        effective_now_ticks,
        now_ticks,
        timing,
        has_conflicts,
        focus_loss_fault,
        preflight_target,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
        observer,
    ) {
        Ok(admission) => admission,
        Err(step) => return step,
    };
    let admission = match finalize_authored_down_admission(
        view,
        config,
        qpc_clock,
        waiter,
        interrupt,
        backend,
        coordinator,
        clock_state,
        runtime,
        local_metrics,
        focus_active,
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        progress_clock,
        timing,
        physical_target_qpc,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
        admission,
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
        physical_target_qpc,
        &admission,
        observer,
    )
}

/// Outcome returned by `admit_authored_down`.  `Allowed` carries the frozen
/// dispatch budget plus the trace kind for the upcoming send; every other
/// variant is a non-terminal redirect (`Continue`) handled by the worker.
enum AdmissionOutcome {
    Allowed {
        trace_kind: u8,
    },
    Guarded {
        trace_kind: u8,
        preflight_target: Option<TargetStamp>,
    },
    BlockedUnfocused,
    FocusLost,
    TargetChanged,
    ControlRejected,
}

/// Pre-send admission gate: focus, preflight, hard-late abort, conflict
/// detection, and final down-admission.  The unfocused path commits pause
/// state and enqueues fixed observation evidence; the observer materializes
/// telemetry and publication.  Does not call `SendInput`.
#[allow(clippy::too_many_arguments)]
fn admit_authored_down(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    progress_clock: &SharedProgressClock,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    timing: &WorkerTimingState,
    has_conflicts: bool,
    focus_loss_fault: bool,
    preflight_target: Option<TargetStamp>,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    observer: Option<&PendingObservationQueue>,
) -> Result<AdmissionOutcome, DispatchStep> {
    let trace_kind = trace_kind_for_view(view);
    let has_down_events = view.packet_masks.down_mask != 0 || view.batch_kind == ActionKind::Down;
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
        progress_clock.publish(clock_state);
        runtime.focus_restore_started_ticks = None;
        if let Some(observer) = observer {
            observer.push(
                super::observation::DispatchObservation::BlockedUnfocused(
                    BlockedUnfocusedObservation {
                        event_index: view.batch_source_action_index,
                        authored_ticks: view.authored_batch_scheduled_ticks,
                        effective_deadline_ticks: view.batch_scheduled_ticks,
                        effective_now_ticks,
                        polyphony: view.batch_intent_count,
                    },
                ),
                &mut local_metrics.observer_dropped_samples,
                &mut local_metrics.observer_queue_high_watermark,
            );
        }
        return Ok(AdmissionOutcome::BlockedUnfocused);
    }
    if has_down_events && focus_loss_fault && !runtime.focus_loss_fault_injected {
        runtime.focus_loss_fault_injected = true;
        return Err(DispatchStep::Terminate(
            "focus lost after due check before SendInput boundary".to_string(),
        ));
    }
    let preflight_target = if has_down_events {
        let Some(preflight_target) = preflight_target else {
            return Err(DispatchStep::Terminate(
                "down-bearing dispatch reached final admission without preflight proof".to_string(),
            ));
        };
        preflight_target
    } else {
        load_target_stamp(target_hwnd, target_generation)
    };
    if has_down_events
        && !target_stamp_still_current(target_hwnd, target_generation, preflight_target)
    {
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
    let control_signals = FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
    };
    let control_admission = final_control_precheck(FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
    });
    if !matches!(control_admission, FinalControlAdmission::Allowed) {
        runtime.verified_target = None;
        return Ok(AdmissionOutcome::ControlRejected);
    }
    let guard_lease = final_control_admission_at(now_ticks, lease_timeout_ticks, control_signals)
        .map_err(|error| {
        DispatchStep::Terminate(format!("lease admission QPC failure: {error:?}"))
    })?;
    if !matches!(guard_lease, FinalControlAdmission::Allowed) {
        runtime.verified_target = None;
        return Ok(AdmissionOutcome::ControlRejected);
    }
    Ok(AdmissionOutcome::Guarded {
        trace_kind,
        preflight_target: has_down_events.then_some(preflight_target),
    })
}

#[allow(clippy::too_many_arguments)]
fn finalize_authored_down_admission(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    qpc_clock: QpcClock,
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    progress_clock: &SharedProgressClock,
    timing: &WorkerTimingState,
    physical_target_qpc: QpcTicks,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    admission: AdmissionOutcome,
) -> Result<AdmissionOutcome, DispatchStep> {
    let AdmissionOutcome::Guarded {
        trace_kind,
        preflight_target,
    } = admission
    else {
        return Ok(admission);
    };
    let spin_target_qpc = QpcTicks::from_raw(
        physical_target_qpc
            .as_u64()
            .saturating_sub(timing.effective_spin_threshold_ticks.as_u64()),
    );
    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
        qpc_clock,
        spin_target_qpc,
        DurationTicks::ZERO,
        interrupt,
    );
    match wait_result.outcome {
        sky_dispatch_win32::wait::WaitOutcome::Deadline => {}
        sky_dispatch_win32::wait::WaitOutcome::Interrupted => {
            local_metrics.wait_interrupted_count =
                local_metrics.wait_interrupted_count.saturating_add(1);
            return Ok(AdmissionOutcome::ControlRejected);
        }
        sky_dispatch_win32::wait::WaitOutcome::Failed(failure) => {
            if matches!(failure, sky_dispatch_win32::wait::WaitFailure::Clock) {
                local_metrics.wait_clock_failures =
                    local_metrics.wait_clock_failures.saturating_add(1);
            } else {
                local_metrics.wait_backend_failures =
                    local_metrics.wait_backend_failures.saturating_add(1);
            }
            return Err(DispatchStep::Terminate(super::super::wait_failure_message(
                failure,
            )));
        }
    }

    let control_signals = FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
    };
    let control_admission = final_control_precheck(FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
    });
    if !matches!(control_admission, FinalControlAdmission::Allowed) {
        runtime.verified_target = None;
        return Ok(AdmissionOutcome::ControlRejected);
    }
    let view_has_down = view.packet_masks.down_mask != 0 || view.batch_kind == ActionKind::Down;
    if view_has_down {
        let Some(expected) = preflight_target else {
            return Err(DispatchStep::Terminate(
                "down-bearing dispatch reached final admission without frozen target proof"
                    .to_string(),
            ));
        };
        match final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus: config.focus.require_focus,
            focus_active,
            target_hwnd,
            target_generation,
        }) {
            DownAdmission::Allowed => {}
            DownAdmission::FocusLost => {
                return handle_final_focus_loss(
                    qpc_clock,
                    backend,
                    coordinator,
                    clock_state,
                    runtime,
                    target_hwnd,
                    progress_clock,
                );
            }
            DownAdmission::TargetChanged => {
                runtime.verified_target = None;
                return Ok(AdmissionOutcome::TargetChanged);
            }
        }
    }
    let final_admission_qpc = qpc_clock
        .now()
        .map_err(|error| DispatchStep::Terminate(format!("QPC final proof failure: {error:?}")))?;
    let lease_admission =
        final_control_admission_at(final_admission_qpc, lease_timeout_ticks, control_signals)
            .map_err(|error| {
                DispatchStep::Terminate(format!("lease admission QPC failure: {error:?}"))
            })?;
    if !matches!(lease_admission, FinalControlAdmission::Allowed) {
        runtime.verified_target = None;
        return Ok(AdmissionOutcome::ControlRejected);
    }
    if view_has_down && final_admission_qpc >= physical_target_qpc {
        runtime.verified_target = None;
        return Err(DispatchStep::TerminateStatic(
            "down_deadline_missed_before_send",
        ));
    }
    if !view_has_down && final_admission_qpc >= physical_target_qpc {
        if let Err(error) =
            suspend_live_input(backend, coordinator, target_hwnd.load(Ordering::Acquire))
        {
            return Err(DispatchStep::Terminate(format!(
                "late up-only safety release failed: {error}"
            )));
        }
        return Err(DispatchStep::TerminateStatic(
            "up_deadline_missed_before_send",
        ));
    }
    Ok(AdmissionOutcome::Allowed { trace_kind })
}

#[allow(clippy::too_many_arguments)]
fn handle_final_focus_loss(
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    target_hwnd: &AtomicIsize,
    progress_clock: &SharedProgressClock,
) -> Result<AdmissionOutcome, DispatchStep> {
    runtime.verified_target = None;
    let focus_ticks = qpc_clock
        .now()
        .map_err(|error| DispatchStep::Terminate(format!("QPC failure: {error:?}")))?;
    suspend_live_input(backend, coordinator, target_hwnd.load(Ordering::Acquire))
        .map_err(|error| DispatchStep::Terminate(format!("focus suspension failed: {error}")))?;
    clock_state
        .enter_pause("focus", focus_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "playback clock failure after final focus check: {error}"
            ))
        })?;
    progress_clock.publish(clock_state);
    runtime.focus_restore_started_ticks = None;
    Ok(AdmissionOutcome::FocusLost)
}

#[inline]
fn spin_and_send_prepared(
    qpc_clock: QpcClock,
    physical_target_qpc: QpcTicks,
    backend: &mut TrackedKeyState,
    prepared_packet: &sky_dispatch_win32::input::PreparedPhysicalPacket,
) -> Result<sky_dispatch_win32::input::SendTransactionOutcome, sky_dispatch_win32::clock::QpcError>
{
    loop {
        let now_ticks = qpc_clock.now()?;
        if now_ticks >= physical_target_qpc {
            return Ok(backend.send_prepared_physical_packet_with_start(prepared_packet, now_ticks));
        }
        std::hint::spin_loop();
    }
}

/// Contstructs the allowed admission from the frozen dispatch budget and the
/// packet trace kind.  Extracted to keep `admit_authored_down` under the per
/// dispatch-function line limit.
fn trace_kind_for_view(view: &AuthoredBatchView) -> u8 {
    match view.prepared_batch.packet_kind {
        sky_dispatch_core::model::PhysicalPacketKind::UpOnly => TRACE_KIND_UP,
        sky_dispatch_core::model::PhysicalPacketKind::DownOnly => TRACE_KIND_DOWN,
        sky_dispatch_core::model::PhysicalPacketKind::Mixed => TRACE_KIND_MIXED,
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
    physical_target_qpc: QpcTicks,
    admission: &AdmissionOutcome,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let AdmissionOutcome::Allowed { trace_kind } = admission else {
        return DispatchStep::Continue;
    };
    let packet = view.packet_masks;
    let prepared_packet = &view.prepared_packet;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.mark_first_physical_send_started();
    }
    debug_assert_eq!(prepared_packet.packet(), packet);
    let result =
        match spin_and_send_prepared(qpc_clock, physical_target_qpc, backend, prepared_packet) {
            Ok(result) => result,
            Err(error) => return DispatchStep::Terminate(format!("QPC spin failure: {error:?}")),
        };
    if let Some(started_qpc) = result.evidence.started_ticks
        && let Err(error) = record_sendinput_entry_lateness(
            qpc_clock,
            physical_target_qpc,
            started_qpc,
            local_metrics,
        )
    {
        return DispatchStep::Terminate(error);
    }
    if let Some(error) = backend.timing_error.take() {
        return DispatchStep::Terminate(format!("QPC failure after note-on: {error:?}"));
    }
    let result_success = result.is_success();
    let result_started_ticks = result.evidence.started_ticks;
    let result_completed_ticks = result.evidence.completed_ticks;
    let result_confirmed_mask = result.evidence.confirmed_mask;
    let result_skipped_mask = result.evidence.skipped_mask;
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
        physical_target_qpc,
        trace_kind,
        result_success,
        result.status,
        result_started_ticks,
        result_completed_ticks,
        result_confirmed_mask,
        result_skipped_mask,
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
    physical_target_qpc: QpcTicks,
    trace_kind: u8,
    result_success: bool,
    result_status: sky_dispatch_win32::input::SendTransactionStatus,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
    result_confirmed_mask: u16,
    result_skipped_mask: u16,
    result_send_attempts: u8,
    result_retry_reason: sky_dispatch_win32::input::PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let timing_proof = match interpret_down_send_timing(
        view,
        config,
        clock_state,
        runtime,
        qpc_clock,
        physical_target_qpc,
        coordinator,
        health,
        timing,
        result_success,
        result_status,
        result_started_ticks,
        result_completed_ticks,
        result_confirmed_mask,
        result_skipped_mask,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };
    let capture_dispatch_ready_qpc = config.profile.observer_enabled();
    publisher_down_send_outcome(
        view,
        runtime,
        health,
        local_metrics,
        qpc_clock,
        effective_now_ticks,
        physical_target_qpc,
        capture_dispatch_ready_qpc,
        trace_kind,
        result_status,
        result_confirmed_mask,
        result_skipped_mask,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
        observer,
        &timing_proof,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_slo_terminal_step(
    result_chord_integrity_lost: bool,
    strict_completion_late: bool,
    _saturation_abort: bool,
    qpc_clock: QpcClock,
    completion_error_ticks: i64,
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
    if strict_completion_late {
        let completion_error_us = match signed_ticks_to_us(qpc_clock, completion_error_ticks) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "note-on terminal timing conversion failure: {error}"
                ));
            }
        };
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
    DispatchStep::Dispatched
}

#[cfg(test)]
mod tests {
    use super::super::PhysicalCommit;
    use super::*;
    use sky_dispatch_core::coordinator::{PreparedAuthoredCommit, PreparedBatch};
    use sky_dispatch_core::model::PhysicalPacketKind;
    use sky_dispatch_win32::input::{PhysicalPacket, PreparedPhysicalPacket};
    use std::num::NonZeroU64;

    #[test]
    fn healthy_down_terminal_path_does_not_convert_ticks_to_microseconds() {
        let view = AuthoredBatchView {
            prepared_batch: PreparedBatch {
                index: 0,
                effective_scheduled_ticks: TimelineTicks::ZERO,
                packet_kind: PhysicalPacketKind::DownOnly,
                packet_batch_count: 1,
                packet_index: 0,
            },
            batch_source_action_index: 0,
            batch_intent_count: 1,
            batch_kind: ActionKind::Down,
            batch_scheduled_ticks: TimelineTicks::ZERO,
            authored_batch_scheduled_ticks: TimelineTicks::ZERO,
            conflict_mask: 0,
            dispatch_path: DispatchPath::DownOnly { down_count: 1 },
            packet_masks: PhysicalPacket::new(0, 0b001),
            prepared_packet: PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001))
                .unwrap(),
            commit: PhysicalCommit::Authored(PreparedAuthoredCommit {
                frame: sky_dispatch_core::coordinator::PreparedAuthoredFrame {
                    first_batch_index: 0,
                    packet_index: 0,
                    packet_batch_count: 1,
                    authored_ticks: TimelineTicks::ZERO,
                    immediate_up_mask: 0,
                    deferred_up_mask: 0,
                    down_mask: 0b001,
                    stale_up_count: 0,
                },
                immediate_up_intents: smallvec::SmallVec::new(),
                deferred_up_intents: smallvec::SmallVec::new(),
                down_intents: smallvec::SmallVec::new(),
                down_source_action_index: Some(0),
            }),
        };
        let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1).unwrap());
        let mut runtime = WorkerRuntime::default();

        let step = resolve_slo_terminal_step(
            false,
            false,
            false,
            qpc_clock,
            i64::MIN,
            &view,
            &mut runtime,
        );

        assert!(matches!(step, DispatchStep::Dispatched));
    }

    #[test]
    fn anchored_target_math_supports_explicit_offset() {
        let anchor = QpcTicks::from_raw(10_000);
        let lead = DurationTicks::from_raw(500);
        for (scheduled, expected_target) in [
            (0, 9_500),
            (100, 9_600),
            (499, 9_999),
            (500, 10_000),
            (501, 10_001),
        ] {
            let target = super::super::super::anchored_dispatch_target_ticks_typed(
                QpcTicks::from_raw(9_500),
                anchor,
                TimelineTicks::from_raw(scheduled),
                lead,
            )
            .expect("startup target");
            assert_eq!(target, QpcTicks::from_raw(expected_target));
        }
    }
}
