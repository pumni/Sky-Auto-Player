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
fn exact_hold_and_release_floors_are_measured_from_sender_completion() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(DurationTicks::from_raw(20), DurationTicks::from_raw(10));
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
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        139,
        140,
        142,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    assert_eq!(metrics.production_forensics_version, 4);
    assert_eq!(metrics.production_hold_pair_samples, 1);
    assert_eq!(
        metrics.production_min_hold_start_after_down_completion_ticks,
        20
    );
    assert_eq!(metrics.production_hold_floor_violation_count, 0);
    assert_eq!(metrics.production_release_floor_samples, 1);
    assert_eq!(
        metrics.production_min_down_start_after_up_completion_ticks,
        10
    );
    assert_eq!(metrics.production_release_floor_violation_count, 0);
    assert_eq!(metrics.production_forensics_anomaly_count, 0);
}

#[test]
fn effective_min_hold_includes_timing_margin_once_in_hold_oracle() {
    let mut forensics = ProductionHoldForensics::default();
    // 20 ticks base hold plus a 5-tick timing margin is materialized as 25
    // before the forensics boundary, exactly as it is for the production guard.
    forensics.set_frame_policies(DurationTicks::from_raw(25), DurationTicks::from_raw(10));
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
fn successful_recovery_up_records_hold_and_next_down_release_samples() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(DurationTicks::from_raw(20), DurationTicks::from_raw(10));
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

    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        139,
        140,
        142,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    assert_eq!(metrics.production_release_floor_samples, 1);
    assert_eq!(
        metrics.production_min_down_start_after_up_completion_ticks,
        10
    );
    assert_eq!(metrics.production_release_floor_violation_count, 0);
}

#[test]
fn uncertain_recovery_transport_does_not_create_hold_or_release_evidence() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(DurationTicks::from_raw(20), DurationTicks::from_raw(10));
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
        false,
        &mut metrics,
    );

    assert_eq!(metrics.production_hold_pair_samples, 0);
    assert_eq!(metrics.production_release_floor_samples, 0);
    assert_eq!(metrics.production_forensics_anomaly_count, 0);
}

#[test]
fn up_start_below_down_completion_hold_floor_is_a_forensics_anomaly() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(DurationTicks::from_raw(20), DurationTicks::from_raw(10));
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
    assert_eq!(anomaly.mask, 1);
    assert_eq!(anomaly.target_ticks, 120);
    assert_eq!(anomaly.observed_ticks, 124);
    assert_eq!(anomaly.aux_ticks, 105);
    assert_eq!(anomaly.delta_ticks, 19);
}

#[test]
fn next_down_start_below_up_completion_frame_floor_is_an_anomaly() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(DurationTicks::from_raw(20), DurationTicks::from_raw(10));
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
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        138,
        139,
        142,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    assert_eq!(metrics.production_release_floor_samples, 1);
    assert_eq!(
        metrics.production_min_down_start_after_up_completion_ticks,
        9
    );
    assert_eq!(metrics.production_release_floor_violation_count, 1);
    let anomaly = forensics
        .latest_anomaly_for_test()
        .expect("release anomaly");
    assert_eq!(anomaly.kind, 7);
    assert_eq!(anomaly.observed_ticks, 139);
    assert_eq!(anomaly.aux_ticks, 130);
    assert_eq!(anomaly.delta_ticks, 9);
}

#[test]
fn safety_reset_discards_musical_anchors_without_recording_a_floor_violation() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(DurationTicks::from_raw(20), DurationTicks::from_raw(10));
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

    // Safety/emergency Up is represented by ResetAll and bypasses musical
    // forensics; it is not fed back as an authored packet observation.
    forensics.observe_lifecycle(ObserverLifecycle::ResetAll);

    assert_eq!(metrics.production_hold_floor_violation_count, 0);
    assert_eq!(metrics.production_release_floor_violation_count, 0);
    assert_eq!(metrics.production_forensics_anomaly_count, 0);
}

#[test]
fn incomplete_transport_does_not_create_optimistic_floor_evidence() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(DurationTicks::from_raw(20), DurationTicks::from_raw(10));
    let mut metrics = WorkerMetricsLocal::default();
    observe(
        &mut forensics,
        PhysicalPacket::new(0, 1),
        100,
        101,
        105,
        SendTransactionStatus::PartialProgress,
        &mut metrics,
    );

    assert!(!metrics.production_forensics_available);
    assert_eq!(metrics.production_hold_pair_samples, 0);
    assert_eq!(metrics.production_release_floor_samples, 0);
}
