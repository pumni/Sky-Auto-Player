//! Normal prepared-frame precision envelope.

use super::super::{
    DownAdmission, FinalControlAdmission, FinalControlSignals, FinalTargetSignals,
    PreparedDispatchFrame, QpcClock, RuntimeDispatchCoordinator, TargetStamp, TrackedKeyState,
    WorkerConfig, WorkerHealthState, WorkerMetricsLocal, WorkerResources, WorkerRuntime,
    WorkerTimingState, final_control_precheck, final_down_atomic_revalidation,
    final_down_target_admission, focus_matches, handle_final_focus_loss,
    modifier_guard_rejection_step, record_final_gate_rejection, record_sendinput_pre_call_lateness,
};
use super::authored::record_prepared_normal_send_outcome;
use super::{AuthoredBatchView, DispatchStep, PendingObservationQueue};
use crate::engine::shared::SessionTarget;
use crate::engine::shared::{
    SharedProgressClock, SupervisorLeaseAdmission, SupervisorLeaseState, SystemPowerState,
};
use sky_dispatch_core::model::GenerationId;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{ModifierKeyObservation, ModifierMask, SendTransactionOutcome};
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreparedNormalAdmission {
    EarlyControl,
    FocusLost,
    TargetChanged,
    LateControl,
    OwnerMismatch,
    OwnerQueryUnavailable,
    OwnerIdentityAbsent,
    OwnerIdentityStale,
    OwnerProcessTerminated,
    OwnerIdentityDrift,
    ModifierHeld(ModifierMask),
    SupervisorLeaseExpired,
    SupervisorLeaseUnproven,
}

impl PreparedNormalAdmission {
    fn from_down_admission(admission: DownAdmission) -> Option<Self> {
        match admission {
            DownAdmission::Allowed => None,
            DownAdmission::FocusLost => Some(Self::FocusLost),
            DownAdmission::TargetChanged => Some(Self::TargetChanged),
            DownAdmission::OwnerMismatch => Some(Self::OwnerMismatch),
            DownAdmission::OwnerQueryUnavailable => Some(Self::OwnerQueryUnavailable),
            DownAdmission::OwnerIdentityAbsent => Some(Self::OwnerIdentityAbsent),
            DownAdmission::OwnerIdentityStale => Some(Self::OwnerIdentityStale),
            DownAdmission::OwnerProcessTerminated => Some(Self::OwnerProcessTerminated),
            DownAdmission::OwnerIdentityDrift => Some(Self::OwnerIdentityDrift),
        }
    }
}

enum PreparedNormalPrecisionResult {
    Rejected(PreparedNormalAdmission),
    Sent {
        result: sky_dispatch_win32::input::SendTransactionOutcome,
        final_policy_qpc: Option<QpcTicks>,
    },
}

use PreparedNormalAdmission::ModifierHeld;
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
        if !matches!(
            final_control_precheck(control_signals),
            FinalControlAdmission::Allowed
        ) {
            return Ok(PreparedNormalPrecisionResult::Rejected(
                PreparedNormalAdmission::EarlyControl,
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
        if let Some(admission) = PreparedNormalAdmission::from_down_admission(down_admission) {
            return Ok(PreparedNormalPrecisionResult::Rejected(admission));
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
    if !matches!(
        final_control_precheck(control_signals),
        FinalControlAdmission::Allowed
    ) {
        return Ok(PreparedNormalPrecisionResult::Rejected(
            PreparedNormalAdmission::LateControl,
        ));
    }

    if has_down_events {
        match backend.observe_modifiers_before_down() {
            ModifierKeyObservation::NoHeldObserved => {}
            ModifierKeyObservation::HeldObserved(mask) => {
                return Ok(PreparedNormalPrecisionResult::Rejected(
                    PreparedNormalAdmission::ModifierHeld(mask),
                ));
            }
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
        let authority =
            final_down_atomic_revalidation(target, expected, require_focus, focus_active);
        if let Some(admission) = PreparedNormalAdmission::from_down_admission(authority) {
            return Ok(PreparedNormalPrecisionResult::Rejected(admission));
        }
        #[cfg(test)]
        if let Some(hook) = final_lease_age_hook {
            hook(supervisor_expired);
        }
        let lease_proof = super::super::final_down_lease_age_proof(
            supervisor_expired,
            lease_timeout_ticks,
            qpc_clock,
            #[cfg(test)]
            final_lease_qpc_sample_count,
            #[cfg(test)]
            final_lease_qpc_sample_override,
        )
        .map_err(|_| "prepared_down_qpc_failure")?;
        match lease_proof.admission {
            SupervisorLeaseAdmission::Fresh => {}
            SupervisorLeaseAdmission::Expired => {
                return Ok(PreparedNormalPrecisionResult::Rejected(
                    PreparedNormalAdmission::SupervisorLeaseExpired,
                ));
            }
            SupervisorLeaseAdmission::Unproven => {
                return Ok(PreparedNormalPrecisionResult::Rejected(
                    PreparedNormalAdmission::SupervisorLeaseUnproven,
                ));
            }
        }
        if !matches!(
            final_control_precheck(control_signals),
            FinalControlAdmission::Allowed
        ) {
            return Ok(PreparedNormalPrecisionResult::Rejected(
                PreparedNormalAdmission::LateControl,
            ));
        }
    }

    // The prepared payload and Win32 call metadata are fully resolved by the
    // sender before its authoritative pre-call QPC boundary.
    // P7a's test-support benchmark records the final admission-to-SendInput
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
        Rejected(PreparedNormalAdmission::EarlyControl) => {
            runtime.verified_target = None;
            Err(DispatchStep::Continue)
        }
        Rejected(PreparedNormalAdmission::FocusLost) => Ok(None),
        Rejected(PreparedNormalAdmission::TargetChanged) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            Err(DispatchStep::Continue)
        }
        Rejected(PreparedNormalAdmission::LateControl) => {
            runtime.verified_target = None;
            record_final_gate_rejection(local_metrics, super::super::FinalGateRejection::Control);
            Err(DispatchStep::Continue)
        }
        Rejected(PreparedNormalAdmission::OwnerMismatch) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            Err(DispatchStep::TerminateStatic(
                "prepared_down_owner_mismatch",
            ))
        }
        Rejected(PreparedNormalAdmission::OwnerQueryUnavailable) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            Err(DispatchStep::TerminateStatic(
                "prepared_down_owner_query_unavailable",
            ))
        }
        Rejected(PreparedNormalAdmission::OwnerIdentityAbsent) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            Err(DispatchStep::TerminateStatic(
                "prepared_down_owner_identity_absent",
            ))
        }
        Rejected(PreparedNormalAdmission::OwnerIdentityStale) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            Err(DispatchStep::TerminateStatic(
                "prepared_down_owner_identity_stale",
            ))
        }
        Rejected(PreparedNormalAdmission::OwnerProcessTerminated) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            Err(DispatchStep::TerminateStatic(
                "prepared_down_owner_process_terminated",
            ))
        }
        Rejected(PreparedNormalAdmission::OwnerIdentityDrift) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            Err(DispatchStep::TerminateStatic(
                "prepared_down_owner_identity_drift",
            ))
        }
        Rejected(ModifierHeld(mask)) => Err(modifier_guard_rejection_step(mask)),
        Rejected(PreparedNormalAdmission::SupervisorLeaseExpired) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            record_final_gate_rejection(
                local_metrics,
                super::super::FinalGateRejection::LeaseExpired,
            );
            Err(DispatchStep::TerminateStatic("supervisor_lease_expired"))
        }
        Rejected(PreparedNormalAdmission::SupervisorLeaseUnproven) => {
            runtime.verified_target = None;
            runtime.invalidate_down_authorization();
            Err(DispatchStep::TerminateStatic("supervisor_lease_unproven"))
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
    explicitly_cancelled_by_suspension: &[GenerationId],
    #[cfg(any(test, feature = "test-support"))] test_inject_sender_start: bool,
) -> DispatchStep {
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = now_ticks;
    let view = &frame.view;
    let has_down_events = view.packet_masks.down_mask != 0;
    #[cfg(any(test, feature = "test-support"))]
    if has_down_events && let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.record_r3_down_window(
            physical_timing_window.authored_target_qpc.as_u64(),
            physical_timing_window.packet_not_before_qpc.as_u64(),
        );
    }
    // Keep the cheap outer read for the preroll/focus-pause fault path; the
    // precision helper still owns the authoritative final atomic gate.
    if has_down_events && !focus_matches(config.focus.require_focus, focus_active) {
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
        hook.wait_after_r3_successful_up();
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
        explicitly_cancelled_by_suspension,
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
    explicitly_cancelled_by_suspension: &[GenerationId],
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let packet = view.packet_masks;
    #[cfg(any(test, feature = "test-support"))]
    if packet.down_mask != 0
        && result.evidence.attempts != 0
        && let Some(hook) = runtime.startup_ordering_hook.as_ref()
    {
        hook.record_r3_down_sender_attempt(
            result.evidence.started_ticks.map(|ticks| ticks.as_u64()),
        );
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
    let final_policy_qpc = prepared_final_policy_qpc
        .or(result_started_ticks)
        .unwrap_or(physical_target_qpc);
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
        explicitly_cancelled_by_suspension,
        observer,
    )
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;
