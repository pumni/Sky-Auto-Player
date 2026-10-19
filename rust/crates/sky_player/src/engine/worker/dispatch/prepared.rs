//! Normal prepared-frame precision envelope.

use super::super::{
    FinalControlSignals, FinalDownVerdict, FinalTargetSignals, PreparedDispatchFrame, QpcClock,
    RuntimeDispatchCoordinator, TargetStamp, TrackedKeyState, WorkerConfig, WorkerHealthState,
    WorkerMetricsLocal, WorkerResources, WorkerRuntime, WorkerTimingState,
    apply_final_down_rejection, final_down_control_verdict, final_down_modifier_verdict,
    final_down_post_modifier_verdict, final_down_target_admission, focus_matches,
    handle_final_focus_loss, modifier_guard_rejection_step, owner_rejection_reason,
    record_final_gate_rejection, record_sendinput_pre_call_lateness,
};
use super::authored::record_prepared_normal_send_outcome;
use super::{AuthoredBatchView, DispatchStep, PendingObservationQueue};
use crate::engine::shared::SessionTarget;
use crate::engine::shared::{SharedProgressClock, SupervisorLeaseState, SystemPowerState};
use crate::engine::telemetry::metrics::PrecisionPacketTimingObservation;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::SendTransactionOutcome;
use std::sync::atomic::AtomicBool;

enum PreparedNormalPrecisionResult {
    Rejected(FinalDownVerdict),
    Sent {
        result: sky_dispatch_win32::input::SendTransactionOutcome,
        final_policy_qpc: Option<QpcTicks>,
    },
}

use PreparedNormalPrecisionResult::Rejected;

/// The complete normal precision suffix.  The payload has already been
/// materialized by the startup-owned stream.  This helper owns only the final
/// atomic gates and the one prepared sender transaction; all coordinator,
/// planner, recovery, telemetry, and cursor work remains in its caller after
/// the sender returns.
#[allow(clippy::too_many_arguments)]
fn send_prepared_normal_precision_frame(
    frame: &PreparedDispatchFrame,
    require_focus: bool,
    focus_active: &AtomicBool,
    target: &SessionTarget,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    supervisor_expired: &SupervisorLeaseState,
    system_power: &SystemPowerState,
    qpc_clock: QpcClock,
    lease_timeout_ticks: DurationTicks,
    preflight_target: Option<TargetStamp>,
    #[cfg(any(test, feature = "test-support"))] post_focus_race_hook: Option<
        &crate::engine::config::FinalGateRaceHook,
    >,
    #[cfg(any(test, feature = "test-support"))] post_modifier_race_hook: Option<
        &crate::engine::config::FinalGateRaceHook,
    >,
    #[cfg(any(test, feature = "test-support"))] post_lease_race_hook: Option<
        &crate::engine::config::FinalGateRaceHook,
    >,
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    #[cfg(any(test, feature = "test-support"))] test_now_ticks: Option<QpcTicks>,
    #[cfg(test)] final_lease_qpc_sample_count: &mut u64,
    #[cfg(test)] final_lease_qpc_sample_override: &mut Option<
        Result<QpcTicks, sky_dispatch_win32::clock::QpcError>,
    >,
    #[cfg(test)] final_lease_age_hook: Option<&(dyn Fn(&SupervisorLeaseState) + Send + Sync)>,
) -> Result<PreparedNormalPrecisionResult, &'static str> {
    let has_down_events = frame.view.packet_masks.down_mask != 0;
    if has_down_events {
        let control_signals = FinalControlSignals {
            quit_requested,
            skip_requested,
            panic_requested,
            desired_pause,
            supervisor_expired,
            system_power: Some(system_power),
        };
        if final_down_control_verdict(control_signals) != FinalDownVerdict::Allowed {
            return Ok(PreparedNormalPrecisionResult::Rejected(
                FinalDownVerdict::ControlRejected,
            ));
        }

        let Some(expected) = preflight_target else {
            return Err("prepared Down reached final admission without target proof");
        };
        let down_admission = final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus,
            focus_active,
            target,
            #[cfg(any(test, feature = "test-support"))]
            post_focus_race_hook,
            #[cfg(any(test, feature = "test-support"))]
            post_focus_control_signals: Some(control_signals),
        });
        if down_admission != FinalDownVerdict::Allowed {
            return Ok(PreparedNormalPrecisionResult::Rejected(down_admission));
        }
    }

    let control_signals = FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_expired,
        system_power: Some(system_power),
    };
    if final_down_control_verdict(control_signals) != FinalDownVerdict::Allowed {
        return Ok(PreparedNormalPrecisionResult::Rejected(
            FinalDownVerdict::ControlRejected,
        ));
    }

    if has_down_events {
        let modifier_verdict = final_down_modifier_verdict(backend);
        if modifier_verdict != FinalDownVerdict::Allowed {
            return Ok(PreparedNormalPrecisionResult::Rejected(modifier_verdict));
        }
        #[cfg(any(test, feature = "test-support"))]
        super::super::invoke_final_gate_race_hook(
            post_modifier_race_hook,
            focus_active,
            target,
            quit_requested,
            skip_requested,
            panic_requested,
            desired_pause,
            system_power,
        );
        let expected = preflight_target.expect("prepared Down target was checked above");
        #[cfg(test)]
        if let Some(hook) = final_lease_age_hook {
            hook(supervisor_expired);
        }
        let verdict = final_down_post_modifier_verdict(
            target,
            expected,
            require_focus,
            focus_active,
            control_signals,
            lease_timeout_ticks,
            qpc_clock,
            #[cfg(any(test, feature = "test-support"))]
            post_lease_race_hook,
            #[cfg(test)]
            final_lease_qpc_sample_count,
            #[cfg(test)]
            final_lease_qpc_sample_override,
        )
        .map_err(|_| "prepared_down_qpc_failure")?;
        if verdict != FinalDownVerdict::Allowed {
            return Ok(PreparedNormalPrecisionResult::Rejected(verdict));
        }
    }

    // The prepared payload and Win32 call metadata are fully resolved by the
    // sender before its authoritative pre-call QPC boundary.
    // physical study's test-support benchmark records the final admission-to-SendInput
    // suffix without adding a QPC read to the production path.
    #[cfg(any(test, feature = "test-support"))]
    let final_policy_qpc = backend.physical_study_final_policy_qpc_for_test();
    #[cfg(not(any(test, feature = "test-support")))]
    let final_policy_qpc = None;
    let result = backend.send_prepared_physical_packet_at_final_boundary(
        &frame.view.prepared_packet,
        #[cfg(any(test, feature = "test-support"))]
        test_now_ticks,
        #[cfg(not(any(test, feature = "test-support")))]
        None,
    );
    Ok(PreparedNormalPrecisionResult::Sent {
        result,
        final_policy_qpc,
    })
}

fn finish_prepared_normal_precision_result(
    precision_result: PreparedNormalPrecisionResult,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
) -> Result<
    Option<(
        sky_dispatch_win32::input::SendTransactionOutcome,
        Option<QpcTicks>,
    )>,
    DispatchStep,
> {
    match precision_result {
        PreparedNormalPrecisionResult::Sent {
            result,
            final_policy_qpc,
        } => Ok(Some((result, final_policy_qpc))),
        Rejected(verdict) => {
            apply_final_down_rejection(verdict, runtime, local_metrics);
            match verdict {
                FinalDownVerdict::ControlRejected | FinalDownVerdict::TargetChanged => {
                    Err(DispatchStep::Continue)
                }
                FinalDownVerdict::FocusLost => Ok(None),
                FinalDownVerdict::OwnerRejected(failure) => Err(DispatchStep::TerminateStatic(
                    owner_rejection_reason(failure, true),
                )),
                FinalDownVerdict::ModifierHeld(mask) => Err(modifier_guard_rejection_step(mask)),
                FinalDownVerdict::LeaseExpired => {
                    Err(DispatchStep::TerminateStatic("supervisor_lease_expired"))
                }
                FinalDownVerdict::LeaseUnproven => {
                    Err(DispatchStep::TerminateStatic("supervisor_lease_unproven"))
                }
                FinalDownVerdict::Allowed => {
                    unreachable!("allowed final Down verdict cannot be rejected")
                }
            }
        }
    }
}

/// Normal playback precision envelope for one startup-prepared frame.
///
/// All authored planning, packet construction, and pending-release policy has
/// already completed before the waiter is entered. The healthy suffix below
/// performs the bounded final control/target/focus admission and hands the
/// immutable packet to the prepared sender; coordinator accounting is reached
/// only after the sender returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_prepared_normal_frame(
    frame: &PreparedDispatchFrame,
    config: &WorkerConfig,
    resources: &mut WorkerResources,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    focus_active: &AtomicBool,
    target: &SessionTarget,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    supervisor_expired: &SupervisorLeaseState,
    system_power: &SystemPowerState,
    progress_clock: &SharedProgressClock,
    observer: Option<&PendingObservationQueue>,
    preflight_target: Option<TargetStamp>,
    physical_target_qpc: QpcTicks,
    physical_timing_window: super::super::physical_timing_guard::PhysicalTimingWindow,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    focus_loss_fault: bool,
    boundary_crossing_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))] test_inject_sender_start: bool,
) -> DispatchStep {
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = now_ticks;
    let view = &frame.view;
    let has_down_events = view.packet_masks.down_mask != 0;
    #[cfg(any(test, feature = "test-support"))]
    if has_down_events && let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.record_down_window(
            physical_timing_window.authored_target_qpc.as_u64(),
            physical_timing_window.packet_not_before_qpc.as_u64(),
        );
    }
    // Keep the cheap outer read for the preroll/focus-pause fault path; the
    // precision helper still owns the authoritative final atomic gate.
    if has_down_events && !focus_matches(config.focus.require_focus, focus_active) {
        record_final_gate_rejection(local_metrics, super::super::FinalGateRejection::Focus);
        if !runtime.musical_physical_commit_started {
            return DispatchStep::TerminateStatic("focus_lost_during_preroll");
        }
        if let Err(step) = handle_final_focus_loss(
            resources.clock,
            &mut resources.playback,
            runtime,
            progress_clock,
        ) {
            return step;
        }
        return DispatchStep::Continue;
    }
    if has_down_events && focus_loss_fault && !runtime.focus_loss_fault_injected {
        runtime.focus_loss_fault_injected = true;
        return DispatchStep::TerminateStatic(
            "focus lost after due check before SendInput boundary",
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    super::super::invoke_final_gate_race_hook(
        runtime.final_gate_race_hook.as_ref(),
        focus_active,
        target,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        system_power,
    );
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.mark_first_physical_send_started();
    }
    let precision_result = send_prepared_normal_precision_frame(
        frame,
        config.focus.require_focus,
        focus_active,
        target,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_expired,
        system_power,
        resources.clock,
        timing.lease_timeout_ticks,
        preflight_target,
        #[cfg(any(test, feature = "test-support"))]
        runtime.final_gate_post_focus_race_hook.as_ref(),
        #[cfg(any(test, feature = "test-support"))]
        runtime.final_gate_post_modifier_race_hook.as_ref(),
        #[cfg(any(test, feature = "test-support"))]
        runtime.final_gate_post_lease_race_hook.as_ref(),
        &mut resources.backend,
        #[cfg(any(test, feature = "test-support"))]
        test_inject_sender_start.then_some(now_ticks),
        #[cfg(test)]
        &mut runtime.final_lease_qpc_sample_count_for_test,
        #[cfg(test)]
        &mut runtime.final_lease_qpc_sample_override_for_test,
        #[cfg(test)]
        runtime.final_lease_age_hook_for_test.as_deref(),
    );
    let precision_result = match precision_result {
        Ok(result) => result,
        Err(error) => return DispatchStep::TerminateStatic(error),
    };
    #[cfg(any(test, feature = "test-support"))]
    if frame.view.packet_masks.down_mask == 0
        && frame.view.packet_masks.up_mask != 0
        && let PreparedNormalPrecisionResult::Sent { result, .. } = &precision_result
        && result.is_success()
        && let Some(hook) = runtime.startup_ordering_hook.as_ref()
    {
        hook.wait_after_successful_up();
    }
    let (result, prepared_final_policy_qpc) =
        match finish_prepared_normal_precision_result(precision_result, runtime, local_metrics) {
            Ok(Some(result)) => result,
            Ok(None) => {
                if let Err(step) = handle_final_focus_loss(
                    resources.clock,
                    &mut resources.playback,
                    runtime,
                    progress_clock,
                ) {
                    return step;
                }
                return DispatchStep::Continue;
            }
            Err(step) => return step,
        };
    debug_assert_eq!(view.prepared_packet.packet(), view.packet_masks);
    record_prepared_normal_send_outcome(
        view,
        config,
        health,
        timing,
        runtime,
        local_metrics,
        resources.clock,
        &mut resources.backend,
        &mut resources.coordinator,
        &mut resources.playback,
        effective_now_ticks,
        physical_target_qpc,
        physical_timing_window,
        boundary_crossing_qpc,
        prepared_final_policy_qpc,
        result,
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_down_send_result(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut super::super::PlaybackClockState,
    effective_now_ticks: TimelineTicks,
    physical_target_qpc: QpcTicks,
    physical_timing_window: super::super::physical_timing_guard::PhysicalTimingWindow,
    target_crossing_qpc: Option<QpcTicks>,
    trace_kind: u8,
    prepared_final_policy_qpc: Option<QpcTicks>,
    result: SendTransactionOutcome,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let packet = view.packet_masks;
    #[cfg(any(test, feature = "test-support"))]
    if packet.down_mask != 0
        && result.evidence.attempts != 0
        && let Some(hook) = runtime.startup_ordering_hook.as_ref()
    {
        hook.record_down_sender_attempt(result.evidence.started_ticks.map(|ticks| ticks.as_u64()));
    }
    if result.evidence.attempts != 0 {
        local_metrics.observe_precision_packet_timing(PrecisionPacketTimingObservation {
            authored_target_qpc: physical_timing_window.authored_target_qpc.as_u64(),
            packet_not_before_qpc: physical_timing_window.packet_not_before_qpc.as_u64(),
            pre_call_qpc: result.evidence.started_ticks.map(|ticks| ticks.as_u64()),
            sendinput_completion_qpc: result.evidence.completed_ticks.map(|ticks| ticks.as_u64()),
            previous_sendinput_completion_qpc: runtime
                .last_physical_send_completion_qpc
                .map(|ticks| ticks.as_u64()),
            previous_sender_completion_known: runtime.last_physical_send_completion_qpc.is_some(),
            transport_anomaly: result.status
                != sky_dispatch_win32::input::SendTransactionStatus::Complete
                || result.evidence.last_win32_error.is_some()
                || result.evidence.skipped_mask != 0
                || result.evidence.attempts != 1,
        });
    } else if result.status != sky_dispatch_win32::input::SendTransactionStatus::Complete {
        local_metrics.transport_anomalies = local_metrics.transport_anomalies.saturating_add(1);
    }
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
    runtime.previous_sendinput_completion_qpc = runtime.last_physical_send_completion_qpc;
    runtime.previous_sender_completion_known = runtime.last_physical_send_completion_qpc.is_some();
    if let Some(error) = backend.timing_error.take() {
        if result.evidence.attempts != 0
            && let Some(guard) = runtime.physical_timing_guard.as_mut()
        {
            guard.invalidate();
        }
        return DispatchStep::Terminate(format!("QPC failure after note-on: {error:?}"));
    }
    let result_success = result.is_success();
    let result_started_ticks = result.evidence.started_ticks;
    let result_completed_ticks = result.evidence.completed_ticks;
    if result_success {
        runtime.last_physical_send_completion_qpc = result_completed_ticks;
    }
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
        if result.evidence.attempts != 0
            && let Some(guard) = runtime.physical_timing_guard.as_mut()
        {
            guard.invalidate();
        }
        return DispatchStep::Terminate(format!(
            "authored Down send integrity failure at action {}",
            view.batch_source_action_index
        ));
    }
    let Some(completed_qpc) = result_completed_ticks else {
        if let Some(guard) = runtime.physical_timing_guard.as_mut() {
            guard.invalidate();
        }
        return DispatchStep::TerminateStatic("successful Down missing completion QPC");
    };
    if let Err(step) =
        super::authored::observe_successful_completion(runtime, completed_qpc, packet)
    {
        return step;
    }
    let final_policy_qpc = prepared_final_policy_qpc;
    super::authored::finalize_down_send_outcome(
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
        physical_timing_window,
        target_crossing_qpc,
        final_policy_qpc,
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

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;
