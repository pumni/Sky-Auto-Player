//! Normal prepared-frame precision envelope.

use super::super::{
    DownAdmission, FinalControlAdmission, FinalControlSignals, FinalTargetSignals,
    PreparedDispatchFrame, TargetStamp, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    WorkerResources, WorkerRuntime, WorkerTimingState, final_control_precheck,
    final_down_target_admission, focus_matches, handle_final_focus_loss,
    record_final_gate_rejection, trace_kind_for_packet_kind,
};
use super::authored::{AdmissionOutcome, record_down_send_outcome};
use super::{DispatchStep, PendingObservationQueue};
use crate::engine::shared::{SharedProgressClock, SystemPowerState};
use crate::engine::worker::physical_timing_guard::PhysicalTimingWindow;
use sky_dispatch_core::time::{QpcTicks, TimelineTicks};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64};

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
    physical_timing_window: PhysicalTimingWindow,
    effective_now_ticks: TimelineTicks,
    now_ticks: QpcTicks,
    focus_loss_fault: bool,
    boundary_crossing_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))] test_inject_sender_start: bool,
) -> DispatchStep {
    let view = &frame.view;
    let has_down_events = view.packet_masks.down_mask != 0;
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
        runtime.verified_target = None;
        return DispatchStep::Continue;
    }

    if has_down_events {
        let Some(expected) = preflight_target else {
            return DispatchStep::TerminateStatic(
                "prepared Down reached final admission without target proof",
            );
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
            DownAdmission::TargetChanged => {
                runtime.verified_target = None;
                return DispatchStep::Continue;
            }
        }
    }
    if !matches!(
        final_control_precheck(control_signals),
        FinalControlAdmission::Allowed
    ) {
        runtime.verified_target = None;
        record_final_gate_rejection(local_metrics, super::super::FinalGateRejection::Control);
        return DispatchStep::Continue;
    }

    record_down_send_outcome(
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
        now_ticks,
        physical_target_qpc,
        physical_timing_window,
        physical_timing_window.latest_down_start_qpc,
        &AdmissionOutcome::PreparedAllowed {
            trace_kind: trace_kind_for_packet_kind(view.prepared_batch.packet_kind),
            target_crossing_qpc: boundary_crossing_qpc,
        },
        #[cfg(any(test, feature = "test-support"))]
        test_inject_sender_start.then_some(now_ticks),
        #[cfg(not(any(test, feature = "test-support")))]
        None,
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
        let body = source
            .split("pub(crate) fn dispatch_prepared_normal_frame")
            .nth(1)
            .expect("normal precision helper")
            .split("#[cfg(test)]")
            .next()
            .expect("normal precision helper body");
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
                !body.contains(forbidden),
                "normal precision helper contains forbidden reference {forbidden}"
            );
        }
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
