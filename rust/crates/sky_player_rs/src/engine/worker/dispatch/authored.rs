use super::super::super::{
    ActionKind, DurationTicks, PlaybackClockState, QpcClock, QpcTicks, RuntimeDispatchCoordinator,
    TimelineTicks, TrackedKeyState,
};
#[cfg(any(test, feature = "test-support"))]
use super::super::invoke_final_gate_race_hook;
use super::super::{
    DispatchPath, DownAdmission, FinalControlAdmission, FinalControlSignals, FinalGateRejection,
    FinalTargetSignals, TargetStamp, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    WorkerResources, WorkerRuntime, WorkerTimingState, final_control_admission_at,
    final_control_precheck, final_down_target_admission, focus_matches, handle_final_focus_loss,
    load_target_stamp, record_final_gate_rejection, record_sendinput_pre_call_lateness,
    signed_ticks_to_us, suspend_live_input, target_stamp_still_current, trace_kind_for_packet_kind,
    wait_to_precision_boundary,
};
use super::DownBoundaryAdmission;
use super::observation::{BlockedUnfocusedObservation, ObserverLifecycle};
use super::observer::publisher_down_send_outcome;
use super::recovery::{
    DownMissReason, record_missed_down_classification, record_rescue_admission, record_rescue_send,
    recover_missed_down_boundary,
};
use super::timing::interpret_down_send_timing;
use super::{AuthoredBatchView, AuthoredPacketContext, DispatchStep, PendingObservationQueue};
use crate::engine::shared::SharedProgressClock;
use sky_dispatch_core::clock::PauseReason;
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
        down_admission,
        focus_loss_fault,
        interrupt,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
        boundary_crossing_qpc,
        #[cfg(any(test, feature = "test-support"))]
        test_direct_boundary,
        #[cfg(any(test, feature = "test-support"))]
        test_inject_sender_start,
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
        down_admission,
        focus_loss_fault,
        physical_plan.target_proof.verified_target(),
        interrupt,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
        boundary_crossing_qpc,
        #[cfg(any(test, feature = "test-support"))]
        test_direct_boundary,
        #[cfg(any(test, feature = "test-support"))]
        test_inject_sender_start,
        observer,
    )
}
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
    down_admission: DownBoundaryAdmission,
    focus_loss_fault: bool,
    preflight_target: Option<TargetStamp>,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    boundary_crossing_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))] test_direct_boundary: bool,
    #[cfg(any(test, feature = "test-support"))] test_inject_sender_start: bool,
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
        physical_target_qpc,
        down_admission,
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
    record_rescue_admission(down_admission, &admission, local_metrics);
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
        down_admission,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
        boundary_crossing_qpc,
        #[cfg(any(test, feature = "test-support"))]
        test_direct_boundary,
        admission,
        observer,
    ) {
        Ok(admission) => admission,
        Err(step) => return step,
    };
    if down_admission.is_missed() {
        return recover_missed_down_boundary(
            view,
            config,
            runtime,
            local_metrics,
            backend,
            coordinator,
            clock_state,
            physical_target_qpc,
            now_ticks,
            DownMissReason::Backlog,
            observer,
        );
    }
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
        now_ticks,
        physical_target_qpc,
        down_admission,
        &admission,
        #[cfg(any(test, feature = "test-support"))]
        test_inject_sender_start.then_some(now_ticks),
        #[cfg(not(any(test, feature = "test-support")))]
        None,
        observer,
    )
}
pub(crate) enum AdmissionOutcome {
    Allowed {
        trace_kind: u8,
        target_crossing_qpc: Option<QpcTicks>,
        final_policy_qpc: QpcTicks,
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

#[allow(clippy::too_many_arguments)]
fn resolve_target_crossing_qpc(
    down_admission: DownBoundaryAdmission,
    boundary_crossing_qpc: Option<QpcTicks>,
    physical_target_qpc: QpcTicks,
    qpc_clock: QpcClock,
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    timing: &WorkerTimingState,
    local_metrics: &mut WorkerMetricsLocal,
    #[cfg(any(test, feature = "test-support"))] test_direct_boundary: bool,
) -> Result<Option<QpcTicks>, DispatchStep> {
    if down_admission.is_missed() || down_admission.is_late_rescue() {
        return Ok(None);
    }
    if let Some(boundary_crossing_qpc) = boundary_crossing_qpc {
        return Ok(Some(boundary_crossing_qpc));
    }
    #[cfg(any(test, feature = "test-support"))]
    if test_direct_boundary {
        return Ok(Some(physical_target_qpc));
    }
    let result = wait_to_precision_boundary(
        qpc_clock,
        waiter,
        interrupt,
        physical_target_qpc,
        timing,
        local_metrics,
    );
    result.map(|result| Some(result.target_crossing_qpc))
}

/// Pre-send admission gate for focus, preflight, late-grace, conflict, and final Down authorization.
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
    physical_target_qpc: QpcTicks,
    down_admission: DownBoundaryAdmission,
    timing: &WorkerTimingState,
    has_conflicts: bool,
    focus_loss_fault: bool,
    preflight_target: Option<TargetStamp>,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    observer: Option<&PendingObservationQueue>,
) -> Result<AdmissionOutcome, DispatchStep> {
    let trace_kind = trace_kind_for_packet_kind(view.prepared_batch.packet_kind);
    let has_down_events = view.packet_masks.down_mask != 0 || view.batch_kind == ActionKind::Down;
    if has_down_events && !focus_matches(config.focus.require_focus, focus_active) {
        if !runtime.musical_physical_commit_started {
            return Err(DispatchStep::TerminateStatic("focus_lost_during_preroll"));
        }
        if let Err(error) =
            suspend_live_input(backend, coordinator, target_hwnd.load(Ordering::Acquire))
        {
            return Err(DispatchStep::Terminate(format!(
                "focus suspension failed: {error}"
            )));
        }
        runtime
            .production_forensics
            .observe_lifecycle(ObserverLifecycle::ResetAll);
        super::observation::enqueue_lifecycle(observer, ObserverLifecycle::ResetAll, local_metrics);
        if let Err(error) = clock_state.enter_pause(PauseReason::Focus, now_ticks) {
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
        runtime.invalidate_down_authorization();
        return Ok(AdmissionOutcome::TargetChanged);
    }
    if has_down_events
        && config.timing.strict_timing
        && !down_admission.is_missed()
        && effective_now_ticks
            .checked_duration_since(view.authored_batch_scheduled_ticks)
            .is_ok_and(|late| late > timing.down_late_grace_ticks)
    {
        record_missed_down_classification(
            local_metrics,
            view.batch_source_action_index,
            view.packet_masks.down_mask,
            physical_target_qpc,
            now_ticks,
            DownMissReason::HardLate,
        );
        return Err(DispatchStep::Terminate(
            "authored Down exceeded the session down late-grace window".to_string(),
        ));
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
    let control_admission = final_control_precheck(control_signals);
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
    down_admission: DownBoundaryAdmission,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    boundary_crossing_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))] test_direct_boundary: bool,
    admission: AdmissionOutcome,
    observer: Option<&PendingObservationQueue>,
) -> Result<AdmissionOutcome, DispatchStep> {
    let AdmissionOutcome::Guarded {
        trace_kind,
        preflight_target,
    } = admission
    else {
        return Ok(admission);
    };
    let target_crossing_qpc = match resolve_target_crossing_qpc(
        down_admission,
        boundary_crossing_qpc,
        physical_target_qpc,
        qpc_clock,
        waiter,
        interrupt,
        timing,
        local_metrics,
        #[cfg(any(test, feature = "test-support"))]
        test_direct_boundary,
    ) {
        Ok(value) => value,
        Err(DispatchStep::Continue) => return Ok(AdmissionOutcome::ControlRejected),
        Err(step) => return Err(step),
    };
    #[cfg(any(test, feature = "test-support"))]
    invoke_final_gate_race_hook(
        runtime.final_gate_race_hook.as_ref(),
        focus_active,
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
    );
    let control_signals = FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
    };
    let control_admission = final_control_precheck(control_signals);
    if !matches!(control_admission, FinalControlAdmission::Allowed) {
        runtime.verified_target = None;
        record_final_gate_rejection(local_metrics, FinalGateRejection::Control);
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
            #[cfg(any(test, feature = "test-support"))]
            post_focus_race_hook: runtime.final_gate_post_focus_race_hook.as_ref(),
            #[cfg(any(test, feature = "test-support"))]
            post_focus_control_signals: Some(control_signals),
        }) {
            DownAdmission::Allowed => {}
            DownAdmission::FocusLost => {
                record_final_gate_rejection(local_metrics, FinalGateRejection::Focus);
                handle_final_focus_loss(
                    qpc_clock,
                    backend,
                    coordinator,
                    clock_state,
                    runtime,
                    target_hwnd,
                    progress_clock,
                )?;
                runtime
                    .production_forensics
                    .observe_lifecycle(ObserverLifecycle::ResetAll);
                super::observation::enqueue_lifecycle(
                    observer,
                    ObserverLifecycle::ResetAll,
                    local_metrics,
                );
                return Ok(AdmissionOutcome::FocusLost);
            }
            DownAdmission::TargetChanged => {
                runtime.verified_target = None;
                runtime.invalidate_down_authorization();
                record_final_gate_rejection(local_metrics, FinalGateRejection::Target);
                return Ok(AdmissionOutcome::TargetChanged);
            }
        }
    }
    if !final_atomic_revalidation(control_signals, runtime, local_metrics) {
        return Ok(AdmissionOutcome::ControlRejected);
    }
    #[cfg(any(test, feature = "test-support"))]
    let final_policy_qpc = if test_direct_boundary {
        physical_target_qpc
    } else {
        qpc_clock.now().map_err(|error| {
            DispatchStep::Terminate(format!("QPC final policy boundary failure: {error:?}"))
        })?
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let final_policy_qpc = qpc_clock.now().map_err(|error| {
        DispatchStep::Terminate(format!("QPC final policy boundary failure: {error:?}"))
    })?;
    let lease_admission =
        final_control_admission_at(final_policy_qpc, lease_timeout_ticks, control_signals)
            .map_err(|error| {
                DispatchStep::Terminate(format!("lease admission QPC failure: {error:?}"))
            })?;
    if !matches!(lease_admission, FinalControlAdmission::Allowed) {
        runtime.verified_target = None;
        record_final_gate_rejection(local_metrics, FinalGateRejection::Lease);
        return Ok(AdmissionOutcome::ControlRejected);
    }
    Ok(AdmissionOutcome::Allowed {
        trace_kind,
        target_crossing_qpc,
        final_policy_qpc,
    })
}

fn final_atomic_revalidation(
    control_signals: FinalControlSignals<'_>,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
) -> bool {
    if matches!(
        final_control_precheck(control_signals),
        FinalControlAdmission::Allowed
    ) {
        return true;
    }
    runtime.verified_target = None;
    record_final_gate_rejection(local_metrics, FinalGateRejection::Control);
    false
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
    _now_ticks: QpcTicks,
    physical_target_qpc: QpcTicks,
    down_admission: DownBoundaryAdmission,
    admission: &AdmissionOutcome,
    test_now_ticks: Option<QpcTicks>,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let AdmissionOutcome::Allowed {
        trace_kind,
        target_crossing_qpc,
        final_policy_qpc,
    } = admission
    else {
        return DispatchStep::Continue;
    };
    let packet = view.packet_masks;
    let prepared_packet = &view.prepared_packet;
    let latest_allowed_down_qpc = if packet.down_mask != 0 {
        match physical_target_qpc.checked_add_duration(timing.down_late_grace_ticks) {
            Ok(latest) => Some(latest),
            Err(_) => {
                return DispatchStep::TerminateStatic("down_late_grace_boundary_overflow");
            }
        }
    } else {
        None
    };
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.mark_first_physical_send_started();
    }
    debug_assert_eq!(prepared_packet.packet(), packet);
    let result = backend.send_prepared_physical_packet_at_final_boundary(
        prepared_packet,
        latest_allowed_down_qpc,
        test_now_ticks,
    );
    if let Some(started_qpc) = result.evidence.started_ticks
        && let Err(error) = record_sendinput_pre_call_lateness(
            physical_target_qpc,
            started_qpc,
            timing,
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
    if matches!(
        result.status,
        sky_dispatch_win32::input::SendTransactionStatus::DeadlineMissedBeforeSend
    ) && view.packet_masks.down_mask != 0
    {
        local_metrics.final_gate_cutoff_misses =
            local_metrics.final_gate_cutoff_misses.saturating_add(1);
        record_rescue_send(local_metrics, down_admission, true);
        let Some(observed_qpc) = result.evidence.started_ticks else {
            return DispatchStep::TerminateStatic(
                "DeadlineMissedBeforeSend missing authoritative start boundary",
            );
        };
        if !runtime.musical_physical_commit_started {
            record_missed_down_classification(
                local_metrics,
                view.batch_source_action_index,
                view.packet_masks.down_mask,
                physical_target_qpc,
                observed_qpc,
                DownMissReason::HardLate,
            );
            return DispatchStep::TerminateStatic("down_deadline_missed_before_send");
        }
        return recover_missed_down_boundary(
            view,
            config,
            runtime,
            local_metrics,
            backend,
            coordinator,
            clock_state,
            physical_target_qpc,
            observed_qpc,
            DownMissReason::HardLate,
            observer,
        );
    }
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
    record_rescue_send(local_metrics, down_admission, false);
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
        *target_crossing_qpc,
        *final_policy_qpc,
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
    target_crossing_qpc: Option<QpcTicks>,
    final_policy_qpc: QpcTicks,
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
        target_crossing_qpc,
        final_policy_qpc,
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
    if view.packet_masks.down_mask != 0 {
        runtime.musical_physical_commit_started = true;
    }
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
#[path = "authored_tests.rs"]
mod tests;
