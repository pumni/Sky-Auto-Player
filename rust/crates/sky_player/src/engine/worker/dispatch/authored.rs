use super::super::super::{
    ActionKind, PlaybackClockState, QpcClock, QpcTicks, RuntimeDispatchCoordinator, TimelineTicks,
    TrackedKeyState,
};
#[cfg(any(test, feature = "test-support"))]
use super::super::invoke_final_gate_race_hook;
use super::super::physical_timing_guard::PhysicalTimingWindow;
use super::super::{
    DispatchPath, DownAdmission, FinalControlAdmission, FinalControlSignals, FinalGateRejection,
    FinalTargetSignals, TargetStamp, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    WorkerResources, WorkerRuntime, WorkerTimingState, enter_focus_pause, final_control_precheck,
    final_down_target_admission, focus_matches, handle_final_focus_loss, load_target_stamp,
    record_final_gate_rejection, signed_ticks_to_us, target_stamp_still_current,
    trace_kind_for_packet_kind,
};
use super::DownBoundaryAdmission;
use super::observation::BlockedUnfocusedObservation;
use super::observer::publisher_down_send_outcome;
use super::recovery::{DownMissReason, effective_down_sender_cutoff, recover_missed_down_boundary};
use super::timing::interpret_down_send_timing;
use super::{AuthoredBatchView, AuthoredPacketContext, DispatchStep, PendingObservationQueue};
use crate::engine::shared::{SharedProgressClock, SystemPowerState};
use sky_dispatch_core::model::GenerationId;
use sky_dispatch_win32::input::SendTransactionOutcome;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64};
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
    system_power: &SystemPowerState,
    progress_clock: &SharedProgressClock,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let AuthoredPacketContext {
        dispatch_plan,
        effective_now_ticks,
        now_ticks,
        physical_timing_window,
        down_admission,
        focus_loss_fault,
        supervisor_expired,
        boundary_crossing_qpc,
        #[cfg(any(test, feature = "test-support"))]
        test_direct_boundary,
        #[cfg(any(test, feature = "test-support"))]
        test_inject_sender_start,
    } = ctx;
    let physical_target_qpc = physical_timing_window.authored_target_qpc;
    let WorkerResources {
        clock: qpc_clock,
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
        system_power,
        progress_clock,
        qpc_clock,
        backend,
        coordinator,
        clock_state,
        effective_now_ticks,
        now_ticks,
        physical_target_qpc,
        physical_timing_window,
        down_admission,
        focus_loss_fault,
        physical_plan.target_proof.verified_target(),
        supervisor_expired,
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
    system_power: &SystemPowerState,
    progress_clock: &SharedProgressClock,
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    physical_target_qpc: QpcTicks,
    physical_timing_window: PhysicalTimingWindow,
    down_admission: DownBoundaryAdmission,
    focus_loss_fault: bool,
    preflight_target: Option<TargetStamp>,
    supervisor_expired: &AtomicBool,
    boundary_crossing_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))] test_direct_boundary: bool,
    #[cfg(any(test, feature = "test-support"))] test_inject_sender_start: bool,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let has_conflicts = view.conflict_mask != 0;
    let admission = match admit_authored_down(
        view,
        config,
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
        system_power,
        progress_clock,
        effective_now_ticks,
        now_ticks,
        physical_target_qpc,
        has_conflicts,
        focus_loss_fault,
        preflight_target,
        supervisor_expired,
        observer,
    ) {
        Ok(admission) => admission,
        Err(step) => return step,
    };
    let admission = match finalize_authored_down_admission(
        view,
        config,
        qpc_clock,
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
        system_power,
        progress_clock,
        physical_target_qpc,
        down_admission,
        supervisor_expired,
        boundary_crossing_qpc,
        #[cfg(any(test, feature = "test-support"))]
        test_direct_boundary,
        admission,
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
            physical_timing_window,
            now_ticks,
            effective_now_ticks,
            match down_admission {
                DownBoundaryAdmission::UnobservedBacklog => DownMissReason::UnobservedBacklog,
                DownBoundaryAdmission::PhysicalWindowExpired => {
                    DownMissReason::PhysicalWindowExpired
                }
                DownBoundaryAdmission::Authorized => {
                    return DispatchStep::TerminateStatic("authorized Down classified as missed");
                }
            },
            runtime.pending_up_recovery.is_some(),
            &[],
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
        physical_timing_window,
        &admission,
        #[cfg(any(test, feature = "test-support"))]
        test_inject_sender_start.then_some(now_ticks),
        #[cfg(not(any(test, feature = "test-support")))]
        None,
        &[],
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
    #[cfg(any(test, feature = "test-support"))] test_direct_boundary: bool,
) -> Result<Option<QpcTicks>, DispatchStep> {
    if down_admission.is_missed() {
        return Ok(None);
    }
    if let Some(boundary_crossing_qpc) = boundary_crossing_qpc {
        return Ok(Some(boundary_crossing_qpc));
    }
    #[cfg(any(test, feature = "test-support"))]
    if test_direct_boundary {
        return Ok(Some(physical_target_qpc));
    }
    let _ = physical_target_qpc;
    Err(DispatchStep::TerminateStatic(
        "missing physical boundary crossing evidence",
    ))
}

/// Pre-send gate for focus, preflight, conflicts, and final Down authorization.
#[allow(clippy::too_many_arguments)]
fn admit_authored_down(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
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
    system_power: &SystemPowerState,
    progress_clock: &SharedProgressClock,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    physical_target_qpc: QpcTicks,
    has_conflicts: bool,
    focus_loss_fault: bool,
    preflight_target: Option<TargetStamp>,
    supervisor_expired: &AtomicBool,
    observer: Option<&PendingObservationQueue>,
) -> Result<AdmissionOutcome, DispatchStep> {
    let trace_kind = trace_kind_for_packet_kind(view.prepared_batch.packet_kind);
    let has_down_events = view.packet_masks.down_mask != 0 || view.batch_kind == ActionKind::Down;
    if has_down_events && !focus_matches(config.focus.require_focus, focus_active) {
        if !runtime.musical_physical_commit_started {
            return Err(DispatchStep::TerminateStatic("focus_lost_during_preroll"));
        }
        enter_focus_pause(clock_state, runtime, now_ticks, progress_clock)
            .map_err(DispatchStep::Terminate)?;
        if let Some(observer) = observer {
            observer.push(
                super::observation::DispatchObservation::BlockedUnfocused(
                    BlockedUnfocusedObservation {
                        event_index: view.batch_source_action_index,
                        compiled_packet_index: u64::try_from(view.prepared_batch.packet_index).ok(),
                        authored_ticks: view.authored_batch_scheduled_ticks,
                        effective_deadline_ticks: view.batch_scheduled_ticks,
                        effective_now_ticks,
                        physical_target_qpc,
                        observed_qpc: now_ticks,
                        polyphony: view.batch_intent_count,
                        up_mask: view.packet_masks.up_mask,
                        down_mask: view.packet_masks.down_mask,
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
        supervisor_expired,
        system_power: Some(system_power),
    };
    let control_admission = final_control_precheck(control_signals);
    if !matches!(control_admission, FinalControlAdmission::Allowed) {
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
    system_power: &SystemPowerState,
    progress_clock: &SharedProgressClock,
    physical_target_qpc: QpcTicks,
    down_admission: DownBoundaryAdmission,
    supervisor_expired: &AtomicBool,
    boundary_crossing_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))] test_direct_boundary: bool,
    admission: AdmissionOutcome,
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
        supervisor_expired,
        system_power: Some(system_power),
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
                handle_final_focus_loss(qpc_clock, clock_state, runtime, progress_clock)?;
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

pub(super) fn observe_strict_completion(
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    completed_qpc: QpcTicks,
    packet: sky_dispatch_win32::input::PhysicalPacket,
) -> Result<(), DispatchStep> {
    if !timing.strict_timing {
        return Ok(());
    }
    let Some(guard) = runtime.physical_timing_guard.as_mut() else {
        return Err(DispatchStep::TerminateStatic(
            "physical timing guard is not initialized",
        ));
    };
    guard
        .observe_successful_packet(completed_qpc, packet.up_mask, packet.down_mask)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "physical timing guard completion update failed: {error:?}"
            ))
        })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_down_send_outcome(
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
    physical_timing_window: PhysicalTimingWindow,
    admission: &AdmissionOutcome,
    test_now_ticks: Option<QpcTicks>,
    explicitly_cancelled_by_suspension: &[GenerationId],
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let (trace_kind, target_crossing_qpc, prepared_final_policy_qpc) = match admission {
        AdmissionOutcome::Allowed {
            trace_kind,
            target_crossing_qpc,
            final_policy_qpc,
        } => (*trace_kind, *target_crossing_qpc, Some(*final_policy_qpc)),
        _ => return DispatchStep::Continue,
    };
    let packet = view.packet_masks;
    let prepared_packet = &view.prepared_packet;
    let sender_cutoff_qpc = match effective_down_sender_cutoff(physical_timing_window, timing) {
        Ok(sender_cutoff_qpc) => sender_cutoff_qpc,
        Err(error) => return DispatchStep::TerminateStatic(error),
    };
    if timing.strict_timing && packet.down_mask != 0 && sender_cutoff_qpc.is_none() {
        return DispatchStep::TerminateStatic("strict Down packet is missing sender cutoff");
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.mark_first_physical_send_started();
    }
    debug_assert_eq!(prepared_packet.packet(), packet);
    let result = backend.send_prepared_physical_packet_at_final_boundary(
        prepared_packet,
        sender_cutoff_qpc,
        test_now_ticks,
    );
    super::prepared::record_down_send_result(
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
        physical_timing_window,
        target_crossing_qpc,
        trace_kind,
        prepared_final_policy_qpc,
        result,
        explicitly_cancelled_by_suspension,
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_prepared_normal_send_outcome(
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
    sender_cutoff_qpc: Option<sky_dispatch_win32::clock::QpcTicks>,
    target_crossing_qpc: Option<QpcTicks>,
    result: SendTransactionOutcome,
    explicitly_cancelled_by_suspension: &[GenerationId],
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    debug_assert!(!timing.strict_timing);
    if matches!(
        result.status,
        sky_dispatch_win32::input::SendTransactionStatus::DownExpiredBeforeSend
    ) && view.packet_masks.down_mask != 0
    {
        let Some(observed_qpc) = result.evidence.started_ticks else {
            return DispatchStep::TerminateStatic(
                "DownExpiredBeforeSend missing authoritative start boundary",
            );
        };
        if let Some(started_qpc) = result.evidence.started_ticks
            && let Err(error) = super::super::record_sendinput_pre_call_lateness(
                physical_target_qpc,
                started_qpc,
                timing,
                local_metrics,
            )
        {
            return DispatchStep::Terminate(error);
        }
        return super::recovery::resolve_normal_prepared_deadline_miss(
            view,
            runtime,
            local_metrics,
            backend,
            coordinator,
            clock_state,
            effective_now_ticks,
            physical_target_qpc,
            sender_cutoff_qpc,
            observed_qpc,
            DownMissReason::DownExpiredBeforeSend,
            explicitly_cancelled_by_suspension,
            observer,
        );
    }
    let mut normal_observation_window = PhysicalTimingWindow::authored_only(physical_target_qpc);
    normal_observation_window.latest_down_start_qpc = sender_cutoff_qpc;
    super::prepared::record_down_send_result(
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
        normal_observation_window,
        target_crossing_qpc,
        trace_kind_for_packet_kind(view.prepared_batch.packet_kind),
        None,
        result,
        explicitly_cancelled_by_suspension,
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finalize_down_send_outcome(
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
    physical_timing_window: PhysicalTimingWindow,
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
    explicitly_cancelled_by_suspension: &[GenerationId],
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
        explicitly_cancelled_by_suspension,
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
        physical_timing_window,
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
