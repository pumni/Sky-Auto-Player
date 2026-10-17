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
        .split("pub(super) fn record_down_send_result")
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
    assert_eq!(
        helper
            .matches("DispatchStep::TerminateStatic(\"supervisor_lease_expired\")")
            .count(),
        1,
        "expired lease must terminate through its fixed static reason"
    );
    assert_eq!(
        helper
            .matches("DispatchStep::TerminateStatic(\"supervisor_lease_unproven\")")
            .count(),
        1,
        "unproven lease must terminate through its fixed static reason"
    );
    let final_control = helper
        .find("final_control_precheck")
        .expect("final control gate");
    assert_eq!(
        helper.matches("final_control_precheck").count(),
        3,
        "Down keeps early, post-focus, and post-modifier control gates; UpOnly has only the shared late gate"
    );
    let control_gates = helper
        .match_indices("final_control_precheck")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let target = helper
        .find("final_down_target_admission")
        .expect("modifier guard foreground and owner proof");
    let modifier = helper
        .find("observe_modifiers_before_down")
        .expect("fixed modifier guard");
    let final_authority = helper
        .find("final_down_atomic_revalidation")
        .expect("post-modifier guard revalidation");
    let lease_age_proof = helper
        .find("final_down_lease_age_proof")
        .expect("bounded final Down lease-age proof");
    let sender = helper
        .find("let result = backend.send_prepared_physical_packet_at_final_boundary")
        .expect("normal sender handoff");
    assert!(final_control < target);
    assert!(target < control_gates[1]);
    assert!(control_gates[1] < modifier);
    assert!(modifier < final_authority);
    assert!(final_authority < lease_age_proof);
    assert!(lease_age_proof < control_gates[2]);
    assert!(helper.contains("lease_timeout_ticks"));
    assert!(
        control_gates[2] < sender,
        "final atomics must precede sender"
    );
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
    assert!(
        outer.contains("PhysicalTimingWindow"),
        "normal prepared dispatch carries floor evidence into post-send accounting"
    );
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
    assert!(helper.contains("backend.send_prepared_physical_packet_at_final_boundary("));

    let tracked = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../sky_dispatch_win32/src/input/tracked/packet_send.rs"
    ));
    let final_sender = tracked
        .split("pub fn send_prepared_physical_packet_at_final_boundary")
        .nth(1)
        .expect("tracked final prepared sender")
        .split("pub(crate) fn apply_packet_outcome")
        .next()
        .expect("tracked final sender body");
    assert!(final_sender.contains("send_prepared_physical_packet(prepared)"));
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
