use super::super::hold_forensics::ProductionHoldForensics;
use super::super::observation::ObserverLifecycle;
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::input::{PhysicalPacket, SendTransactionStatus};

fn observe(
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
        17,
        QpcTicks::from_raw(target_qpc),
        QpcTicks::from_raw(pre_call_qpc),
        QpcTicks::from_raw(completion_qpc),
        status,
        metrics,
    );
}

#[test]
fn hold_floor_is_measured_from_successful_down_completion() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_timing_policies(DurationTicks::from_raw(20));
    let mut metrics = WorkerMetricsLocal::default();

    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        100,
        101,
        105,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        120,
        125,
        130,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    assert_eq!(metrics.production_forensics_version, 5);
    assert_eq!(metrics.production_hold_pair_samples, 1);
    assert_eq!(
        metrics.production_min_hold_start_after_down_completion_ticks,
        20
    );
    assert_eq!(metrics.production_hold_floor_violation_count, 0);
    assert_eq!(metrics.production_forensics_anomaly_count, 0);
}

#[test]
fn same_key_down_may_follow_a_committed_up_without_a_gap_floor() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_timing_policies(DurationTicks::from_raw(20));
    let mut metrics = WorkerMetricsLocal::default();

    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        100,
        100,
        105,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        125,
        130,
        130,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        130,
        130,
        132,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        152,
        152,
        155,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    assert_eq!(metrics.production_hold_pair_samples, 2);
    assert_eq!(metrics.production_hold_floor_violation_count, 0);
    assert_eq!(metrics.production_forensics_anomaly_count, 0);
}

#[test]
fn effective_min_hold_includes_timing_margin_once_in_hold_oracle() {
    let mut forensics = ProductionHoldForensics::default();
    // 20 ticks base hold plus a 5-tick margin is materialized as 25 before
    // the forensics boundary, exactly as it is for the production guard.
    forensics.set_timing_policies(DurationTicks::from_raw(25));
    let mut metrics = WorkerMetricsLocal::default();

    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        100,
        101,
        105,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        125,
        130,
        135,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    assert_eq!(metrics.production_hold_floor_ticks, 25);
    assert_eq!(
        metrics.production_min_hold_start_after_down_completion_ticks,
        25
    );
    assert_eq!(metrics.production_hold_floor_violation_count, 0);
}

#[test]
fn successful_recovery_up_records_hold_evidence() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_timing_policies(DurationTicks::from_raw(20));
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        100,
        101,
        105,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    forensics.observe_recovery_up(
        1,
        18,
        QpcTicks::from_raw(120),
        QpcTicks::from_raw(125),
        QpcTicks::from_raw(130),
        true,
        &mut metrics,
    );
    assert_eq!(metrics.production_hold_pair_samples, 1);
    assert_eq!(
        metrics.production_min_hold_start_after_down_completion_ticks,
        20
    );
    assert_eq!(metrics.production_hold_floor_violation_count, 0);

    // Resume scheduling has its own verified-cleanup barrier. Forensics no
    // longer imposes or measures the old musical post-Up gap floor.
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        130,
        130,
        132,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    assert_eq!(metrics.production_forensics_anomaly_count, 0);
}

#[test]
fn up_start_below_down_completion_hold_floor_is_a_forensics_anomaly() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_timing_policies(DurationTicks::from_raw(20));
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        100,
        101,
        105,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    observe(
        &mut forensics,
        PhysicalPacket::new(1, 0),
        120,
        124,
        127,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    assert_eq!(metrics.production_hold_floor_violation_count, 1);
    assert_eq!(metrics.production_forensics_anomaly_count, 1);
    let anomaly = forensics.latest_anomaly_for_test().expect("hold anomaly");
    assert_eq!(anomaly.kind, 9);
    assert_eq!(anomaly.slot, 0);
    assert_eq!(anomaly.source_action_index, 17);
    assert_eq!(anomaly.mask, 1);
    assert_eq!(anomaly.target_ticks, 120);
    assert_eq!(anomaly.observed_ticks, 124);
    assert_eq!(anomaly.aux_ticks, 105);
    assert_eq!(anomaly.delta_ticks, 19);
}

#[test]
fn anomaly_ring_overwrite_scalar_counts_test_only_detail_eviction() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_timing_policies(DurationTicks::from_raw(20));
    let mut metrics = WorkerMetricsLocal::default();

    for index in 0..33u64 {
        let target = 100 + index * 10;
        observe(
            &mut forensics,
            PhysicalPacket::new(1, 0),
            target,
            target + 1,
            target + 2,
            SendTransactionStatus::Complete,
            &mut metrics,
        );
    }

    assert_eq!(metrics.production_forensics_anomaly_count, 33);
    assert_eq!(metrics.production_anomaly_ring_overwrite_count, 1);
    let anomaly = forensics.latest_anomaly_for_test().expect("latest anomaly");
    assert_eq!(anomaly.kind, 4);
    assert_eq!(anomaly.source_action_index, 17);
}

#[test]
fn safety_reset_and_incomplete_transport_do_not_create_hold_evidence() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_timing_policies(DurationTicks::from_raw(20));
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        100,
        101,
        105,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    forensics.observe_lifecycle(ObserverLifecycle::ResetAll);
    assert_eq!(metrics.production_forensics_anomaly_count, 0);

    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        200,
        201,
        205,
        SendTransactionStatus::PartialProgress,
        &mut metrics,
    );
    assert_eq!(metrics.production_hold_pair_samples, 0);
    assert_eq!(metrics.production_hold_floor_violation_count, 0);
}
