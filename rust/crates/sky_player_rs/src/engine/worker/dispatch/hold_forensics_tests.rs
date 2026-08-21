use super::super::hold_forensics::{HoldForensics, ProductionHoldForensics};
use super::super::observation::ObserverLifecycle;
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::input::PhysicalPacket;
use sky_dispatch_win32::input::SendTransactionStatus;
use std::num::NonZeroU64;

fn observe(
    forensics: &mut HoldForensics,
    packet: PhysicalPacket,
    target_qpc: u64,
    pre_call_qpc: u64,
    completion_qpc: u64,
    full_transport_success: bool,
    metrics: &mut WorkerMetricsLocal,
) {
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    forensics
        .observe_packet(
            packet,
            QpcTicks::from_raw(target_qpc),
            QpcTicks::from_raw(pre_call_qpc),
            QpcTicks::from_raw(completion_qpc),
            full_transport_success,
            metrics,
            qpc_clock,
            qpc_clock.duration_from_us(500).unwrap(),
        )
        .unwrap();
}

#[test]
fn ordinary_pair_reports_sender_boundary_holds_and_shrink() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_100,
        1_150,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        18_000,
        18_000,
        18_020,
        true,
        &mut metrics,
    );

    assert_eq!(metrics.hold_pair_samples, 1);
    assert_eq!(metrics.min_pre_call_hold_us, 16_900);
    assert_eq!(metrics.min_completion_hold_us, 16_870);
    assert_eq!(metrics.max_pre_call_hold_shrink_us, 100);
    assert_eq!(metrics.max_completion_hold_shrink_us, 130);
    assert_eq!(metrics.pre_call_hold_shrink_over_grace_count, 0);
}

#[test]
fn chord_and_staggered_release_pairs_are_counted_per_slot() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 0b111),
        1_000,
        1_000,
        1_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(0b001, 0),
        18_000,
        18_000,
        18_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(0b110, 0),
        19_000,
        19_000,
        19_000,
        true,
        &mut metrics,
    );

    assert_eq!(metrics.hold_pair_samples, 3);
    assert_eq!(metrics.hold_unmatched_up_count, 0);
}

#[test]
fn same_call_retrigger_closes_old_generation_before_opening_new_one() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_000,
        1_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 1),
        18_000,
        18_000,
        18_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        35_000,
        35_000,
        35_000,
        true,
        &mut metrics,
    );

    assert_eq!(metrics.same_call_retrigger_boundaries, 1);
    assert_eq!(metrics.same_call_retrigger_keys, 1);
    assert_eq!(metrics.hold_pair_samples, 2);
    assert_eq!(metrics.hold_anchor_overwrite_count, 0);
}

#[test]
fn mixed_unrelated_and_partial_overlap_track_only_retriggers() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 0b001),
        1_000,
        1_000,
        1_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(0b001, 0b010),
        18_000,
        18_000,
        18_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(0b010, 0),
        19_000,
        19_000,
        19_000,
        true,
        &mut metrics,
    );
    assert_eq!(metrics.same_call_retrigger_boundaries, 0);
    assert_eq!(metrics.same_call_retrigger_keys, 0);

    let mut overlap_forensics = HoldForensics::default();
    let mut overlap_metrics = WorkerMetricsLocal::default();
    observe(
        &mut overlap_forensics,
        PhysicalPacket::new(0, 0b011),
        1_000,
        1_000,
        1_000,
        true,
        &mut overlap_metrics,
    );
    observe(
        &mut overlap_forensics,
        PhysicalPacket::new(0b010, 0b110),
        18_000,
        18_000,
        18_000,
        true,
        &mut overlap_metrics,
    );
    assert_eq!(overlap_metrics.same_call_retrigger_boundaries, 1);
    assert_eq!(overlap_metrics.same_call_retrigger_keys, 1);
}

#[test]
fn unmatched_up_and_anchor_overwrite_are_counted_without_panicking() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        1_000,
        1_000,
        1_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        2_000,
        2_000,
        2_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        3_000,
        3_000,
        3_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        4_000,
        4_000,
        4_000,
        true,
        &mut metrics,
    );
    assert_eq!(metrics.hold_unmatched_up_count, 1);
    assert_eq!(metrics.hold_anchor_overwrite_count, 1);
    assert_eq!(metrics.hold_pair_samples, 1);
}

#[test]
fn incomplete_transport_does_not_mutate_pairing_state() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_000,
        1_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        18_000,
        18_000,
        18_000,
        false,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        19_000,
        19_000,
        19_000,
        true,
        &mut metrics,
    );
    assert_eq!(metrics.hold_pair_samples, 1);
    assert_eq!(metrics.hold_unmatched_up_count, 0);
}

#[test]
fn synthetic_pre_call_grace_violation_is_counted() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_000,
        1_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        18_000,
        16_500,
        18_000,
        true,
        &mut metrics,
    );
    assert_eq!(metrics.max_pre_call_hold_shrink_us, 1_500);
    assert_eq!(metrics.pre_call_hold_shrink_over_grace_count, 1);
}

#[test]
fn recovery_safety_up_clears_released_generation_before_retrigger() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_000,
        1_000,
        true,
        &mut metrics,
    );
    forensics.observe_lifecycle(ObserverLifecycle::RecoveryUp { up_mask: 1 });
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        18_000,
        18_000,
        18_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        35_000,
        35_000,
        35_000,
        true,
        &mut metrics,
    );

    assert_eq!(metrics.hold_pair_samples, 1);
    assert_eq!(metrics.hold_anchor_overwrite_count, 0);
    assert_eq!(metrics.hold_unmatched_up_count, 0);
}

#[test]
fn global_reset_clears_all_released_generations() {
    let mut forensics = HoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 0b11),
        1_000,
        1_000,
        1_000,
        true,
        &mut metrics,
    );
    forensics.observe_lifecycle(ObserverLifecycle::ResetAll);
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 0b11),
        18_000,
        18_000,
        18_000,
        true,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(0b11, 0),
        35_000,
        35_000,
        35_000,
        true,
        &mut metrics,
    );

    assert_eq!(metrics.hold_pair_samples, 2);
    assert_eq!(metrics.hold_anchor_overwrite_count, 0);
    assert_eq!(metrics.hold_unmatched_up_count, 0);
}

fn observe_production(
    forensics: &mut ProductionHoldForensics,
    packet: PhysicalPacket,
    target_qpc: u64,
    pre_call_qpc: u64,
    completion_qpc: u64,
    status: SendTransactionStatus,
    metrics: &mut WorkerMetricsLocal,
) {
    forensics.observe_packet_result(
        packet,
        0,
        QpcTicks::from_raw(target_qpc),
        QpcTicks::from_raw(pre_call_qpc),
        QpcTicks::from_raw(completion_qpc),
        status,
        metrics,
    );
}

#[test]
fn production_forensics_pairs_fixed_sender_evidence_and_release_gap() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(
        DurationTicks::from_raw(16_667),
        DurationTicks::from_raw(16_667),
    );
    let mut metrics = WorkerMetricsLocal::default();
    observe_production(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_100,
        1_150,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe_production(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        18_000,
        18_100,
        18_150,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe_production(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        35_000,
        35_100,
        35_150,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe_production(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        52_000,
        52_100,
        52_150,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    assert!(metrics.production_forensics_available);
    assert_eq!(metrics.production_forensics_version, 1);
    assert_eq!(metrics.production_hold_pair_samples, 2);
    assert_eq!(metrics.production_release_gap_samples, 1);
    // Release-gap forensic evidence is measured at the actual Down pre-call
    // boundary: 35_100 - 18_150, not the authored target 35_000 - 18_150.
    assert_eq!(metrics.production_min_release_gap_ticks, 16_950);
    assert_eq!(metrics.production_forensics_anomaly_count, 0);
}

#[test]
fn production_forensics_anomaly_ring_retains_boundary_payload() {
    let mut forensics = ProductionHoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe_production(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_100,
        1_150,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    // Deliberately exercise the invalid same-call retrigger path. The ring
    // must retain the action and timestamp context, not only kind/slot.
    forensics.observe_packet_result(
        PhysicalPacket::new(1, 1),
        42,
        QpcTicks::from_raw(30_000),
        QpcTicks::from_raw(30_100),
        QpcTicks::from_raw(30_100),
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    let anomaly = forensics
        .latest_anomaly_for_test()
        .expect("same-call retrigger must leave an anomaly payload");
    assert_eq!(anomaly.kind, 6);
    assert_eq!(anomaly.slot, 0);
    assert_eq!(anomaly.source_action_index, 42);
    assert_eq!(anomaly.mask, 1);
    assert_eq!(anomaly.target_ticks, 30_000);
    assert_eq!(anomaly.observed_ticks, 30_100);
    assert_eq!(anomaly.aux_ticks, 1_000);
    assert_eq!(anomaly.delta_ticks, 29_000);
    assert_eq!(metrics.production_forensics_anomaly_count, 1);
}

#[test]
fn production_forensics_preserves_negative_release_ordering_as_anomaly() {
    let mut forensics = ProductionHoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe_production(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_000,
        1_000,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe_production(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        20_000,
        20_000,
        20_000,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    forensics.observe_packet_result(
        PhysicalPacket::new(0, 1),
        91,
        QpcTicks::from_raw(30_000),
        QpcTicks::from_raw(19_000),
        QpcTicks::from_raw(30_000),
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    let anomaly = forensics
        .latest_anomaly_for_test()
        .expect("ordering fault must be retained");
    assert_eq!(anomaly.kind, 8);
    assert_eq!(anomaly.source_action_index, 91);
    assert_eq!(anomaly.target_ticks, 30_000);
    assert_eq!(anomaly.observed_ticks, 19_000);
    assert_eq!(anomaly.aux_ticks, 20_000);
    assert_eq!(anomaly.delta_ticks, 1_000);
    assert_eq!(metrics.production_release_gap_samples, 0);
    assert_eq!(metrics.production_release_gap_below_policy_count, 1);
}

#[test]
fn production_forensics_dropped_down_and_recovery_do_not_fabricate_pairs() {
    let mut forensics = ProductionHoldForensics::default();
    let mut metrics = WorkerMetricsLocal::default();
    observe_production(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        1_000,
        1_000,
        1_000,
        SendTransactionStatus::ZeroProgress,
        &mut metrics,
    );
    observe_production(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        18_000,
        18_000,
        18_000,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    assert!(metrics.production_forensics_available);
    assert_eq!(metrics.production_hold_pair_samples, 0);
    assert_eq!(metrics.production_unmatched_up_count, 1);

    forensics.observe_lifecycle(ObserverLifecycle::ResetAll);
    observe_production(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        35_000,
        35_000,
        35_000,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    forensics.observe_lifecycle(ObserverLifecycle::RecoveryUp { up_mask: 1 });
    observe_production(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        52_000,
        52_000,
        52_000,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    assert_eq!(metrics.production_hold_pair_samples, 0);
    assert_eq!(metrics.production_unmatched_up_count, 2);
}
