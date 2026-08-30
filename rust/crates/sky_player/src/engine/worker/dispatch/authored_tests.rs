use super::super::{PhysicalCommit, RecoveryDescriptor};
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
        prepared_packet: PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001)).unwrap(),
        recovery: RecoveryDescriptor::None,
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
            up_intents: smallvec::SmallVec::new(),
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
fn final_gate_precedes_the_authoritative_pre_call_boundary() {
    let source = include_str!("authored.rs");
    let finalizer = source
        .split("fn finalize_authored_down_admission")
        .nth(1)
        .expect("authored finalizer");
    let crossing = finalizer
        .find("let target_crossing_qpc")
        .expect("target crossing handoff");
    assert!(!finalizer.contains("wait_to_precision_boundary"));
    let control = finalizer
        .find("let control_admission")
        .expect("final control gate");
    let target = finalizer
        .find("final_down_target_admission")
        .expect("final target/focus gate");
    let pre_call = finalizer
        .find("let final_policy_qpc")
        .expect("final policy timestamp");
    assert!(crossing < control);
    assert!(control < target);
    assert!(target < pre_call);

    let sender = source
        .split("fn record_down_send_outcome")
        .nth(1)
        .expect("authored sender handoff");
    assert!(sender.contains("send_prepared_physical_packet_at_final_boundary"));
    assert!(!sender.contains("send_prepared_physical_packet_at_target_with_cutoff"));
}

#[test]
fn final_gate_rejection_counters_are_worker_local_and_reason_specific() {
    let mut metrics = WorkerMetricsLocal::default();
    for reason in [
        FinalGateRejection::Control,
        FinalGateRejection::Target,
        FinalGateRejection::Focus,
        FinalGateRejection::Lease,
    ] {
        super::record_final_gate_rejection(&mut metrics, reason);
    }
    assert_eq!(metrics.final_gate_control_rejections, 1);
    assert_eq!(metrics.final_gate_target_changes, 1);
    assert_eq!(metrics.final_gate_focus_losses, 1);
    assert_eq!(metrics.final_gate_lease_expirations, 1);
    assert_eq!(metrics.final_gate_cutoff_misses, 0);
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
