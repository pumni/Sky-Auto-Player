//! Normal prepared-frame precision envelope.

use super::super::{
    DownAdmission, FinalControlAdmission, FinalControlSignals, FinalTargetSignals,
    PreparedDispatchFrame, QpcClock, RuntimeDispatchCoordinator, TargetStamp, TrackedKeyState,
    WorkerConfig, WorkerHealthState, WorkerMetricsLocal, WorkerResources, WorkerRuntime,
    WorkerTimingState, final_control_precheck, final_down_target_admission, focus_matches,
    handle_final_focus_loss, record_final_gate_rejection, record_sendinput_pre_call_lateness,
};
use super::authored::record_prepared_normal_send_outcome;
use super::recovery::{DownMissReason, recover_missed_down_boundary};
use super::{AuthoredBatchView, DispatchStep, PendingObservationQueue};
use crate::engine::shared::{SharedProgressClock, SystemPowerState};
use crate::engine::worker::physical_timing_guard::PhysicalTimingWindow;
use sky_dispatch_core::model::GenerationId;
use sky_dispatch_core::time::{QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::SendTransactionOutcome;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreparedNormalAdmission {
    EarlyControl,
    FocusLost,
    TargetChanged,
    LateControl,
}

enum PreparedNormalPrecisionResult {
    Rejected(PreparedNormalAdmission),
    Sent(sky_dispatch_win32::input::SendTransactionOutcome),
}

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
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    supervisor_expired: &AtomicBool,
    system_power: &SystemPowerState,
    preflight_target: Option<TargetStamp>,
    #[cfg(any(test, feature = "test-support"))] post_focus_race_hook: Option<
        &crate::engine::config::FinalGateRaceHook,
    >,
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    #[cfg(any(test, feature = "test-support"))] test_now_ticks: Option<QpcTicks>,
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
        match final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus,
            focus_active,
            target_hwnd,
            target_generation,
            #[cfg(any(test, feature = "test-support"))]
            post_focus_race_hook,
            #[cfg(any(test, feature = "test-support"))]
            post_focus_control_signals: Some(control_signals),
        }) {
            DownAdmission::Allowed => {}
            DownAdmission::FocusLost => {
                return Ok(PreparedNormalPrecisionResult::Rejected(
                    PreparedNormalAdmission::FocusLost,
                ));
            }
            DownAdmission::TargetChanged => {
                return Ok(PreparedNormalPrecisionResult::Rejected(
                    PreparedNormalAdmission::TargetChanged,
                ));
            }
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

    // The prepared payload and Win32 call metadata are fully resolved by the
    // sender before its authoritative pre-call QPC boundary.
    let result = backend.send_prepared_physical_packet_at_final_boundary(
        &frame.view.prepared_packet,
        None,
        #[cfg(any(test, feature = "test-support"))]
        test_now_ticks,
        #[cfg(not(any(test, feature = "test-support")))]
        None,
    );
    Ok(PreparedNormalPrecisionResult::Sent(result))
}

/// Normal playback precision envelope for one startup-prepared frame.
///
/// All authored planning, packet construction, and pending-release policy has
/// already completed before the waiter is entered. The healthy suffix below
/// only performs atomic control/target gates and hands the immutable packet to
/// the prepared sender; coordinator accounting is reached only after the
/// sender returns.
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
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    supervisor_expired: &AtomicBool,
    system_power: &SystemPowerState,
    progress_clock: &SharedProgressClock,
    observer: Option<&PendingObservationQueue>,
    preflight_target: Option<TargetStamp>,
    physical_target_qpc: QpcTicks,
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
    // Keep this cheap outer read for the distinct preroll/focus-pause path.
    // The precision helper still performs the authoritative final atomic
    // admission; removing this read would merge the preroll fault path with
    // the post-start revalidation path and the test-only focus fault seam.
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
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
    );
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.mark_first_physical_send_started();
    }
    let precision_result = send_prepared_normal_precision_frame(
        frame,
        config.focus.require_focus,
        focus_active,
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_expired,
        system_power,
        preflight_target,
        #[cfg(any(test, feature = "test-support"))]
        runtime.final_gate_post_focus_race_hook.as_ref(),
        &mut resources.backend,
        #[cfg(any(test, feature = "test-support"))]
        test_inject_sender_start.then_some(now_ticks),
    );
    let precision_result = match precision_result {
        Ok(result) => result,
        Err(error) => return DispatchStep::TerminateStatic(error),
    };
    let result = match precision_result {
        PreparedNormalPrecisionResult::Sent(result) => result,
        PreparedNormalPrecisionResult::Rejected(PreparedNormalAdmission::EarlyControl) => {
            runtime.verified_target = None;
            return DispatchStep::Continue;
        }
        PreparedNormalPrecisionResult::Rejected(PreparedNormalAdmission::FocusLost) => {
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
        PreparedNormalPrecisionResult::Rejected(PreparedNormalAdmission::TargetChanged) => {
            runtime.verified_target = None;
            return DispatchStep::Continue;
        }
        PreparedNormalPrecisionResult::Rejected(PreparedNormalAdmission::LateControl) => {
            runtime.verified_target = None;
            record_final_gate_rejection(local_metrics, super::super::FinalGateRejection::Control);
            return DispatchStep::Continue;
        }
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
        boundary_crossing_qpc,
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
    physical_timing_window: Option<PhysicalTimingWindow>,
    target_crossing_qpc: Option<QpcTicks>,
    trace_kind: u8,
    prepared_final_policy_qpc: Option<QpcTicks>,
    result: SendTransactionOutcome,
    explicitly_cancelled_by_suspension: &[GenerationId],
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    let packet = view.packet_masks;
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
    if matches!(
        result.status,
        sky_dispatch_win32::input::SendTransactionStatus::DownExpiredBeforeSend
    ) && view.packet_masks.down_mask != 0
        && timing.strict_timing
    {
        let Some(observed_qpc) = result.evidence.started_ticks else {
            return DispatchStep::TerminateStatic(
                "DownExpiredBeforeSend missing authoritative start boundary",
            );
        };
        let Some(physical_timing_window) = physical_timing_window else {
            return DispatchStep::TerminateStatic(
                "strict Down recovery is missing physical timing evidence",
            );
        };
        return recover_missed_down_boundary(
            view,
            config,
            runtime,
            local_metrics,
            backend,
            coordinator,
            clock_state,
            physical_timing_window,
            observed_qpc,
            effective_now_ticks,
            DownMissReason::DownExpiredBeforeSend,
            false,
            observer,
        );
    }
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
        super::authored::observe_strict_completion(timing, runtime, completed_qpc, packet)
    {
        return step;
    }
    let final_policy_qpc = prepared_final_policy_qpc
        .or(result_started_ticks)
        .unwrap_or(physical_target_qpc);
    let physical_timing_window = physical_timing_window
        .unwrap_or_else(|| PhysicalTimingWindow::authored_only(physical_target_qpc));
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
mod tests {
    use crate::engine::worker::dispatch::PhysicalCommit;
    use crate::engine::worker::{
        DispatchPreparationProbe, PreparedDispatchEntry, PreparedDispatchStream,
    };
    use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    use sky_dispatch_win32::clock::QpcClock;
    use sky_dispatch_win32::input::MaterializedInstrumentKeyProfile;
    use std::num::NonZeroU64;

    #[test]
    fn normal_precision_envelope_has_no_dynamic_dispatch_references() {
        let source = include_str!("prepared.rs");
        let outer = source
            .split("pub(crate) fn dispatch_prepared_normal_frame")
            .nth(1)
            .expect("normal precision helper")
            .split("#[cfg(test)]")
            .next()
            .expect("normal precision helper body");
        let helper = source
            .split("fn send_prepared_normal_precision_frame")
            .nth(1)
            .expect("prepared precision suffix")
            .split("/// Normal playback precision envelope")
            .next()
            .expect("prepared precision suffix body");
        for forbidden in [
            "RuntimeDispatchCoordinator",
            "plan_next_dispatch_projected",
            "prepare_current_authored_packet",
            "pending_release",
            "DownBoundaryState",
            "recover_missed_down_boundary",
            "focus_matches_hwnd",
            "supervisor_lease_expired",
            "format!(",
            "String",
            "Vec",
            "Box",
            "Mutex",
            "lock(",
        ] {
            assert!(
                !helper.contains(forbidden),
                "normal precision suffix contains forbidden reference {forbidden}"
            );
        }
        let final_control = helper
            .find("final_control_precheck")
            .expect("final control gate");
        assert_eq!(
            helper.matches("final_control_precheck").count(),
            2,
            "Down keeps early+late control gates; UpOnly has only the shared late gate"
        );
        let sender = helper
            .find("let result = backend.send_prepared_physical_packet_at_final_boundary")
            .expect("normal sender handoff");
        assert!(final_control < sender, "final atomics must precede sender");
        assert!(helper.contains("final_down_target_admission"));
        assert!(helper.contains("test_now_ticks"));
        assert!(!helper.contains("inline(never)"));
        assert!(!helper.contains("record_prepared_normal_send_outcome"));

        let precision_call = outer
            .find("let precision_result = send_prepared_normal_precision_frame")
            .expect("precision suffix call");
        let post_send = outer
            .find("record_prepared_normal_send_outcome")
            .expect("normal post-send accounting");
        assert!(
            precision_call < post_send,
            "normal precision suffix must precede post-send work"
        );
        let normal_body = source
            .split("pub(crate) fn dispatch_prepared_normal_frame")
            .nth(1)
            .expect("normal dispatch body")
            .split("pub(super) fn record_down_send_result")
            .next()
            .expect("normal dispatch body before shared outcome accounting");
        assert!(!normal_body.contains("PhysicalTimingWindow"));
        assert!(!normal_body.contains("physical_timing_window"));
        for forbidden in [
            "plan_next_dispatch_projected",
            "prepare_current_authored_packet",
            "pending_release",
            "DownBoundaryState",
            "recover_missed_down_boundary",
            "focus_matches_hwnd",
            "supervisor_lease_expired",
            "lease_timeout",
        ] {
            assert!(
                !outer[..precision_call].contains(forbidden),
                "normal dispatch pre-suffix contains forbidden reference {forbidden}"
            );
        }
    }

    #[test]
    fn prepared_sender_handoff_is_transitively_auditable() {
        let source = include_str!("prepared.rs");
        let helper = source
            .split("fn send_prepared_normal_precision_frame")
            .nth(1)
            .expect("prepared precision suffix")
            .split("/// Normal playback precision envelope")
            .next()
            .expect("prepared precision suffix body");
        for forbidden in [
            "RuntimeDispatchCoordinator",
            "plan_next_dispatch_projected",
            "prepare_current_authored_packet",
            "pending_release",
            "DownBoundaryState",
            "recover_missed_down_boundary",
            "focus_matches_hwnd",
            "supervisor_lease_expired",
            "format!(",
            "String",
            "Vec",
            "Box",
            "Mutex",
            "lock(",
            "coordinator",
            "planner",
            "recovery",
        ] {
            assert!(
                !helper.contains(forbidden),
                "prepared precision suffix contains forbidden reference {forbidden}"
            );
        }
        assert!(helper.contains("backend.send_prepared_physical_packet_at_final_boundary"));
        assert!(helper.contains("None"), "normal sender must pass no cutoff");

        let tracked = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sky_dispatch_win32/src/input/tracked/packet_send.rs"
        ));
        let final_sender = tracked
            .split("pub fn send_prepared_physical_packet_at_final_boundary")
            .nth(1)
            .expect("tracked final prepared sender")
            .split("pub fn send_prepared_physical_packet_view_with_cutoff")
            .next()
            .expect("tracked final sender body");
        assert!(final_sender.contains("send_prepared_physical_packet_with_cutoff"));
        assert!(!final_sender.contains("RuntimeDispatchCoordinator"));

        let packet = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sky_dispatch_win32/src/input/packet.rs"
        ));
        let syscall = packet
            .split("fn send_input_view_at_target")
            .nth(1)
            .expect("prepared Win32 input envelope")
            .split("fn ")
            .next()
            .expect("prepared Win32 input envelope body");
        let last_error = syscall
            .find("SetLastError(0)")
            .expect("last-error preparation");
        let qpc = syscall
            .find("clock.now()")
            .expect("authoritative QPC sample");
        let send_input = syscall.find("SendInput(").expect("single SendInput call");
        assert!(last_error < qpc && qpc < send_input);
    }

    #[test]
    fn normal_stream_materializes_only_physical_boundaries_and_keeps_authored_order() {
        let actions = [
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 0,
                kind: sky_dispatch_core::model::ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stream-down-1".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 1,
                kind: sky_dispatch_core::model::ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stream-up-1".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 2,
                kind: sky_dispatch_core::model::ActionKind::Down,
                scheduled_us: 40_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stream-down-2".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 3,
                kind: sky_dispatch_core::model::ActionKind::Up,
                scheduled_us: 60_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stream-up-2".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 4,
                kind: sky_dispatch_core::model::ActionKind::Up,
                scheduled_us: 80_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stream-stale-up".into(),
            },
        ];
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
            .expect("valid same-key stream schedule");
        let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let profile = MaterializedInstrumentKeyProfile::canonical();
        let probe = DispatchPreparationProbe::default();
        let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            500,
            DurationTicks::from_raw(500),
            |microseconds| {
                qpc_clock
                    .timeline_from_us(microseconds)
                    .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
            },
        )
        .expect("prepared stream test coordinator");
        let (stream, _) = PreparedDispatchStream::build(coordinator, qpc_clock, &probe, &profile)
            .expect("normal stream preparation");

        assert_eq!(stream.physical_count(), 4);
        assert_eq!(stream.len(), 5);
        let mut offsets = Vec::new();
        let mut metadata_count = 0;
        for entry in stream.entries() {
            match entry {
                PreparedDispatchEntry::Physical(frame) => {
                    assert_eq!(
                        frame.view.packet_masks.up_mask & frame.view.packet_masks.down_mask,
                        0
                    );
                    let PhysicalCommit::Authored(commit) = &frame.view.commit else {
                        panic!("healthy same-key schedule must use authored commits");
                    };
                    assert_eq!(commit.frame.deferred_up_mask, 0);
                    offsets.push(frame.offset_ticks);
                }
                PreparedDispatchEntry::Metadata {
                    offset_ticks,
                    commit,
                } => {
                    metadata_count += 1;
                    assert_eq!(offset_ticks.as_u64(), 80_000);
                    assert!(commit.up_intents.is_empty());
                }
            }
        }
        assert_eq!(metadata_count, 1);
        assert!(offsets.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(offsets[0].as_u64(), 0);
        assert_eq!(offsets[1].as_u64(), 20_000);
        assert_eq!(offsets[2].as_u64(), 40_000);
        assert_eq!(offsets[3].as_u64(), 60_000);
    }

    #[test]
    fn prepared_stream_payload_and_offsets_are_immutable_across_builds() {
        let actions = [
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 0,
                kind: sky_dispatch_core::model::ActionKind::Down,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "immutable-down".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 1,
                kind: sky_dispatch_core::model::ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "immutable-up".into(),
            },
        ];
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
            .expect("immutable stream schedule");
        let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let profile = MaterializedInstrumentKeyProfile::canonical();
        let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            500,
            DurationTicks::from_raw(500),
            |microseconds| {
                qpc_clock
                    .timeline_from_us(microseconds)
                    .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
            },
        )
        .expect("immutable stream coordinator");
        let probe = DispatchPreparationProbe::default();
        let mut stream = PreparedDispatchStream::build(coordinator, qpc_clock, &probe, &profile)
            .expect("immutable stream")
            .0;
        let entries_before = stream.entries().iter().map(|entry| match entry {
            PreparedDispatchEntry::Physical(frame) => (
                frame.offset_ticks,
                frame.view.packet_masks,
                frame.view.prepared_packet.packet(),
                frame.view.prepared_packet.event_count(),
            ),
            PreparedDispatchEntry::Metadata { .. } => (
                TimelineTicks::ZERO,
                sky_dispatch_win32::input::PhysicalPacket::new(0, 0),
                sky_dispatch_win32::input::PhysicalPacket::new(0, 0),
                0,
            ),
        });
        let entries_before: Vec<_> = entries_before.collect();
        assert_eq!(entries_before.len(), 2);
        stream.advance().expect("immutable stream cursor advance");
        let entries_after: Vec<_> = stream
            .entries()
            .iter()
            .map(|entry| match entry {
                PreparedDispatchEntry::Physical(frame) => (
                    frame.offset_ticks,
                    frame.view.packet_masks,
                    frame.view.prepared_packet.packet(),
                    frame.view.prepared_packet.event_count(),
                ),
                PreparedDispatchEntry::Metadata { .. } => (
                    TimelineTicks::ZERO,
                    sky_dispatch_win32::input::PhysicalPacket::new(0, 0),
                    sky_dispatch_win32::input::PhysicalPacket::new(0, 0),
                    0,
                ),
            })
            .collect();
        assert_eq!(entries_before, entries_after);
    }
}
