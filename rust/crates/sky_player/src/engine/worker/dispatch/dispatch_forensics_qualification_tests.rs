use super::super::hold_forensics::{ProductionForensicsAnomaly, ProductionHoldForensics};
use super::super::observation::{DownObservation, ObserverLifecycle};
use super::super::observer_wake::{publish_pending_wait_observation, record_wait_attempt};
use super::PendingObservationQueue;
use crate::engine::telemetry::{
    SharedMetrics, TelemetryCollector, TelemetryMode, WorkerMetricsLocal,
};
use crate::engine::worker::WorkerRuntime;
use crate::engine::worker::dispatch::observation::record_down_send_telemetry;
use crate::engine::worker::dispatch_loop::publish_live_metrics_after_dispatch;
use crate::engine::worker::health::publish_backend_counters;
use crate::engine::worker::physical_timing_guard::PhysicalTimingGuard;
use crate::timing_model::{
    ModelPacket, ModelPacketKind, WholeStreamModelConfig, analyze_whole_stream,
};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::input::{PhysicalPacket, SendTransactionStatus, TrackedKeyState};
use sky_dispatch_win32::wait::{WaitOutcome, WaitResult};
use std::num::NonZeroU64;
use std::time::Instant;

const HOLD_TICKS: u64 = 20;
const RELEASE_TICKS: u64 = 10;
const SEND_ENVELOPE_TICKS: u64 = 5;

#[derive(Clone, Copy, Debug)]
struct RawPacket {
    packet: PhysicalPacket,
    target_ticks: u64,
}

#[derive(Clone, Copy, Debug)]
struct PacketSample {
    packet: PhysicalPacket,
    target_ticks: u64,
    pre_call_ticks: u64,
    completion_ticks: u64,
    timing_window: crate::engine::worker::physical_timing_guard::PhysicalTimingWindow,
}

#[derive(Clone, Copy, Debug)]
enum QualificationWorkload {
    DenseAlternating,
    SparseSingleKey,
    MixedUpDown,
    FifteenKeyChordSweep,
    SameKeyRetrigger,
}

impl QualificationWorkload {
    const ALL: [Self; 5] = [
        Self::DenseAlternating,
        Self::SparseSingleKey,
        Self::MixedUpDown,
        Self::FifteenKeyChordSweep,
        Self::SameKeyRetrigger,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::DenseAlternating => "dense-alternating",
            Self::SparseSingleKey => "sparse-single-key",
            Self::MixedUpDown => "mixed-up-down",
            Self::FifteenKeyChordSweep => "fifteen-key-chord-sweep",
            Self::SameKeyRetrigger => "same-key-retrigger",
        }
    }

    fn raw_cycle(self) -> Vec<RawPacket> {
        self.raw_cycle_with(HOLD_TICKS, RELEASE_TICKS, 100_000)
    }

    fn raw_cycle_with(
        self,
        hold_ticks: u64,
        release_ticks: u64,
        sparse_gap_ticks: u64,
    ) -> Vec<RawPacket> {
        let mut packets = Vec::new();
        match self {
            Self::DenseAlternating => {
                for pair in 0..8_u64 {
                    let slot = (pair % 2) as usize;
                    let target = pair * (hold_ticks + release_ticks);
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(0, 1 << slot),
                        target_ticks: target,
                    });
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(1 << slot, 0),
                        target_ticks: target + hold_ticks,
                    });
                }
            }
            Self::SparseSingleKey => {
                for pair in 0..24_u64 {
                    let target = pair * sparse_gap_ticks;
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(0, 1),
                        target_ticks: target,
                    });
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(1, 0),
                        target_ticks: target + hold_ticks,
                    });
                }
            }
            Self::MixedUpDown => {
                packets.push(RawPacket {
                    packet: PhysicalPacket::new(0, 1),
                    target_ticks: 0,
                });
                let mut active_slot = 0_usize;
                for index in 0..8_u64 {
                    let next_slot = 1 - active_slot;
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(1 << active_slot, 1 << next_slot),
                        target_ticks: hold_ticks + index * hold_ticks,
                    });
                    active_slot = next_slot;
                }
                packets.push(RawPacket {
                    packet: PhysicalPacket::new(1 << active_slot, 0),
                    target_ticks: hold_ticks * 9,
                });
            }
            Self::FifteenKeyChordSweep => {
                let mut target = 0_u64;
                for chord_size in 2..=15_u32 {
                    let mask = ((1_u32 << chord_size) - 1) as u16;
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(0, mask),
                        target_ticks: target,
                    });
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(mask, 0),
                        target_ticks: target + hold_ticks,
                    });
                    target += hold_ticks + sparse_gap_ticks;
                }
            }
            Self::SameKeyRetrigger => {
                for pair in 0..4_u64 {
                    let target = pair * (hold_ticks + release_ticks);
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(0, 1),
                        target_ticks: target,
                    });
                    packets.push(RawPacket {
                        packet: PhysicalPacket::new(1, 0),
                        target_ticks: target + hold_ticks,
                    });
                }
            }
        }
        packets
    }
}

fn make_guard() -> PhysicalTimingGuard {
    PhysicalTimingGuard::new(
        DurationTicks::from_raw(HOLD_TICKS),
        DurationTicks::from_raw(RELEASE_TICKS),
    )
}

fn make_samples(
    workload: QualificationWorkload,
    guard: &mut PhysicalTimingGuard,
    batch_index: usize,
    cycles: usize,
) -> Vec<PacketSample> {
    let raw_packets = workload.raw_cycle();
    let last_target = raw_packets
        .last()
        .expect("qualification cycle has packets")
        .target_ticks;
    let cycle_span = last_target + 200_000;
    let batch_base = 1_000_000 + (batch_index as u64) * (cycles as u64) * cycle_span;
    let mut samples = Vec::with_capacity(raw_packets.len() * cycles);

    for cycle in 0..cycles {
        let cycle_base = batch_base + cycle as u64 * cycle_span;
        for raw in &raw_packets {
            let target_ticks = cycle_base + raw.target_ticks;
            let authored_target = QpcTicks::from_raw(target_ticks);
            let timing_window = guard
                .query(authored_target, raw.packet.up_mask, raw.packet.down_mask)
                .expect("qualified packet masks");
            let pre_call_ticks = timing_window.packet_not_before_qpc.as_u64();
            let completion_ticks = pre_call_ticks + SEND_ENVELOPE_TICKS;
            guard
                .observe_successful_packet(
                    QpcTicks::from_raw(completion_ticks),
                    raw.packet.up_mask,
                    raw.packet.down_mask,
                )
                .expect("qualified completion");
            samples.push(PacketSample {
                packet: raw.packet,
                target_ticks,
                pre_call_ticks,
                completion_ticks,
                timing_window,
            });
        }
    }
    samples
}

#[test]
fn physical_guard_and_production_forensics_agree_on_successful_workload_matrix() {
    for workload in QualificationWorkload::ALL {
        let raw = workload.raw_cycle();
        let cycles = (512_usize + raw.len() - 1) / raw.len();
        let mut builder_guard = make_guard();
        let samples = make_samples(workload, &mut builder_guard, 0, cycles);
        let mut measured_guard = make_guard();
        let mut forensics = ProductionHoldForensics::default();
        forensics.set_frame_policies(
            DurationTicks::from_raw(HOLD_TICKS),
            DurationTicks::from_raw(RELEASE_TICKS),
        );
        let mut metrics = WorkerMetricsLocal::default();
        let mut expected_hold_samples = 0_u64;
        let mut expected_release_samples = 0_u64;
        let mut previously_down = 0_u16;

        for (index, sample) in samples.iter().enumerate() {
            let target = QpcTicks::from_raw(sample.target_ticks);
            let window = measured_guard
                .query(target, sample.packet.up_mask, sample.packet.down_mask)
                .expect("physical floor query succeeds");
            assert_eq!(
                window,
                sample.timing_window,
                "{} packet {index}",
                workload.name()
            );
            assert!(sample.pre_call_ticks >= sample.target_ticks);
            assert_eq!(
                sample.pre_call_ticks,
                window.packet_not_before_qpc.as_u64(),
                "{} packet {index} is sent at its production guard floor",
                workload.name()
            );
            measured_guard
                .observe_successful_packet(
                    QpcTicks::from_raw(sample.completion_ticks),
                    sample.packet.up_mask,
                    sample.packet.down_mask,
                )
                .expect("physical completion update succeeds");

            expected_hold_samples += u64::from(sample.packet.up_mask.count_ones());
            expected_release_samples +=
                u64::from((sample.packet.down_mask & previously_down).count_ones());
            previously_down |= sample.packet.down_mask;
            forensics.observe_packet_result(
                sample.packet,
                index as u32,
                target,
                QpcTicks::from_raw(sample.pre_call_ticks),
                QpcTicks::from_raw(sample.completion_ticks),
                SendTransactionStatus::Complete,
                &mut metrics,
            );
        }

        assert_eq!(
            metrics.production_hold_pair_samples,
            expected_hold_samples,
            "{}",
            workload.name()
        );
        assert_eq!(
            metrics.production_release_floor_samples,
            expected_release_samples,
            "{}",
            workload.name()
        );
        assert_eq!(
            metrics.production_hold_floor_violation_count,
            0,
            "{}",
            workload.name()
        );
        assert_eq!(
            metrics.production_release_floor_violation_count,
            0,
            "{}",
            workload.name()
        );
        assert_eq!(
            metrics.production_forensics_anomaly_count,
            0,
            "{}",
            workload.name()
        );
    }
}

#[test]
fn offline_whole_stream_model_characterizes_hold_gap_and_hol_for_workload_matrix() {
    const H_US: u64 = 17_167;
    const G_US: u64 = 17_167;
    let config = WholeStreamModelConfig {
        qpc_frequency_hz: 1_000_000,
        effective_min_hold_us: H_US,
        physical_release_gap_us: G_US,
        // Explicit illustrative costs; these are not inferred from the
        // SendInput envelope and do not describe Windows or game reception.
        estimated_sender_setup_us: 10,
        estimated_per_input_us: 3,
        estimated_post_send_us: 5,
        estimated_metadata_cost_us: 0,
        resume_down_barrier_ticks: None,
    };

    for workload in QualificationWorkload::ALL {
        let packets = workload
            .raw_cycle_with(H_US, G_US, 100_000)
            .into_iter()
            .map(|raw| ModelPacket {
                authored_target_ticks: raw.target_ticks,
                up_mask: raw.packet.up_mask,
                down_mask: raw.packet.down_mask,
                kind: ModelPacketKind::Physical,
            })
            .collect::<Vec<_>>();
        let report = analyze_whole_stream(&packets, config).expect("offline model input");
        let max_hold_displacement = report
            .packets
            .iter()
            .map(|packet| packet.h_floor_displacement_ticks)
            .max()
            .unwrap_or_default();
        let max_release_displacement = report
            .packets
            .iter()
            .map(|packet| packet.t_floor_displacement_ticks)
            .max()
            .unwrap_or_default();
        let max_hol_lower_bound = report
            .packets
            .iter()
            .map(|packet| packet.hol_delay_lower_bound_ticks)
            .max()
            .unwrap_or_default();
        let max_modeled_lateness = report
            .packets
            .iter()
            .map(|packet| packet.modeled_lateness_ticks)
            .max()
            .unwrap_or_default();
        println!(
            "#476 offline-model workload={} guarantee={} packets={} negative_slack={} max_hold_displacement_us={} max_release_displacement_us={} max_hol_lower_bound_us={} max_modeled_lateness_us={}",
            workload.name(),
            report.guarantee,
            report.packet_count,
            report.negative_slack_count,
            max_hold_displacement,
            max_release_displacement,
            max_hol_lower_bound,
            max_modeled_lateness,
        );
        assert!(!report.guarantee);
        assert_eq!(report.packet_count, packets.len());
        assert_eq!(report.packets.len(), packets.len());
    }

    let chord_mask = (1_u16 << 14) - 1;
    let last_slot = 1_u16 << 14;
    let hol_stress = [
        ModelPacket {
            authored_target_ticks: 0,
            up_mask: 0,
            down_mask: chord_mask,
            kind: ModelPacketKind::Physical,
        },
        ModelPacket {
            authored_target_ticks: 1,
            up_mask: 0,
            down_mask: last_slot,
            kind: ModelPacketKind::Physical,
        },
        ModelPacket {
            authored_target_ticks: H_US,
            up_mask: chord_mask,
            down_mask: 0,
            kind: ModelPacketKind::Physical,
        },
        ModelPacket {
            authored_target_ticks: H_US + 1,
            up_mask: last_slot,
            down_mask: 0,
            kind: ModelPacketKind::Physical,
        },
    ];
    let report = analyze_whole_stream(&hol_stress, config).expect("offline HOL stress");
    let maximum_hol = report
        .packets
        .iter()
        .map(|packet| packet.hol_delay_lower_bound_ticks)
        .max()
        .unwrap_or_default();
    println!(
        "#476 offline-model workload=14-key-hol-stress guarantee={} packets={} max_hol_lower_bound_us={maximum_hol}",
        report.guarantee, report.packet_count,
    );
    assert!(!report.guarantee);
    assert_eq!(maximum_hol, 56);
}

#[test]
fn lifecycle_reset_and_uncertain_transport_do_not_create_independent_floor_proof() {
    let mut guard = make_guard();
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(
        DurationTicks::from_raw(HOLD_TICKS),
        DurationTicks::from_raw(RELEASE_TICKS),
    );
    let mut metrics = WorkerMetricsLocal::default();
    let down = PhysicalPacket::new(0, 1);
    let down_target = QpcTicks::from_raw(100);
    let down_window = guard.query(down_target, 0, 1).unwrap();
    let down_completion = down_window.packet_not_before_qpc.as_u64() + SEND_ENVELOPE_TICKS;
    guard
        .observe_successful_packet(QpcTicks::from_raw(down_completion), 0, 1)
        .unwrap();
    forensics.observe_packet_result(
        down,
        0,
        down_target,
        down_window.packet_not_before_qpc,
        QpcTicks::from_raw(down_completion),
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    guard.invalidate();
    forensics.observe_lifecycle(ObserverLifecycle::ResetAll);
    let cleanup_completed = QpcTicks::from_raw(1_000);
    guard
        .establish_resume_down_barrier(cleanup_completed)
        .unwrap();
    let resumed_down_target = QpcTicks::from_raw(1_000);
    let resumed_down_window = guard.query(resumed_down_target, 0, 1).unwrap();
    assert_eq!(resumed_down_window.packet_not_before_qpc.as_u64(), 1_010);
    let resumed_down_completion = resumed_down_window.packet_not_before_qpc.as_u64() + 5;
    guard
        .observe_successful_packet(QpcTicks::from_raw(resumed_down_completion), 0, 1)
        .unwrap();
    forensics.observe_packet_result(
        down,
        1,
        resumed_down_target,
        resumed_down_window.packet_not_before_qpc,
        QpcTicks::from_raw(resumed_down_completion),
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    let up = PhysicalPacket::new(1, 0);
    let up_target = QpcTicks::from_raw(resumed_down_completion + HOLD_TICKS);
    let up_window = guard.query(up_target, 1, 0).unwrap();
    let up_pre_call = up_window.packet_not_before_qpc;
    let up_completion = QpcTicks::from_raw(up_pre_call.as_u64() + SEND_ENVELOPE_TICKS);
    guard
        .observe_successful_packet(up_completion, 1, 0)
        .unwrap();
    forensics.observe_packet_result(
        up,
        2,
        up_target,
        up_pre_call,
        up_completion,
        SendTransactionStatus::Complete,
        &mut metrics,
    );
    assert_eq!(metrics.production_hold_pair_samples, 1);
    assert_eq!(metrics.production_hold_floor_violation_count, 0);

    guard.invalidate();
    forensics.observe_lifecycle(ObserverLifecycle::ResetAll);
    let uncertain_up = PhysicalPacket::new(1, 0);
    forensics.observe_packet_result(
        uncertain_up,
        3,
        QpcTicks::from_raw(1_100),
        QpcTicks::from_raw(1_100),
        QpcTicks::from_raw(1_105),
        SendTransactionStatus::PartialProgress,
        &mut metrics,
    );
    assert!(guard.query(QpcTicks::from_raw(1_200), 1, 0).is_err());
    assert_eq!(metrics.production_release_floor_samples, 0);
    assert_eq!(metrics.production_release_floor_violation_count, 0);
}

#[test]
fn interrupted_wait_replan_preserves_sender_completion_anchors() {
    let mut guard = make_guard();
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(
        DurationTicks::from_raw(HOLD_TICKS),
        DurationTicks::from_raw(RELEASE_TICKS),
    );
    let mut metrics = WorkerMetricsLocal::default();
    let down = PhysicalPacket::new(0, 1);
    let down_target = QpcTicks::from_raw(100);
    let down_window = guard.query(down_target, 0, 1).unwrap();
    let down_completion = QpcTicks::from_raw(105);
    guard
        .observe_successful_packet(down_completion, 0, 1)
        .unwrap();
    forensics.observe_packet_result(
        down,
        0,
        down_target,
        down_window.packet_not_before_qpc,
        down_completion,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    let queue = PendingObservationQueue::default();
    let interrupted = record_wait_attempt(
        Some(&queue),
        Some(WaitResult::interrupted()),
        QpcTicks::from_raw(200),
        DurationTicks::from_raw(100),
        TimelineTicks::from_raw(200),
        QpcTicks::ZERO,
    )
    .expect("interrupted replan observation");
    let mut runtime = WorkerRuntime {
        pending_wait_observation: Some(interrupted),
        ..WorkerRuntime::default()
    };
    publish_pending_wait_observation(&mut runtime, Some(&queue), &mut metrics);
    assert!(queue.pop_front().is_some());

    let up = PhysicalPacket::new(1, 0);
    let up_target = QpcTicks::from_raw(125);
    let up_window = guard.query(up_target, 1, 0).unwrap();
    assert_eq!(up_window.packet_not_before_qpc.as_u64(), 125);
    let up_completion = QpcTicks::from_raw(130);
    guard
        .observe_successful_packet(up_completion, 1, 0)
        .unwrap();
    forensics.observe_packet_result(
        up,
        1,
        up_target,
        up_window.packet_not_before_qpc,
        up_completion,
        SendTransactionStatus::Complete,
        &mut metrics,
    );

    assert_eq!(metrics.production_hold_pair_samples, 1);
    assert_eq!(
        metrics.production_min_hold_start_after_down_completion_ticks,
        HOLD_TICKS
    );
    assert_eq!(metrics.production_hold_floor_violation_count, 0);
    assert_eq!(metrics.production_forensics_anomaly_count, 0);
}

#[test]
fn anomaly_ring_remains_bounded_and_reports_overwrites() {
    let mut forensics = ProductionHoldForensics::default();
    forensics.set_frame_policies(
        DurationTicks::from_raw(HOLD_TICKS),
        DurationTicks::from_raw(RELEASE_TICKS),
    );
    let mut metrics = WorkerMetricsLocal::default();
    for index in 0..40_u64 {
        forensics.observe_packet_result(
            PhysicalPacket::new(1, 0),
            index as u32,
            QpcTicks::from_raw(index * 10),
            QpcTicks::from_raw(index * 10),
            QpcTicks::from_raw(index * 10 + 5),
            SendTransactionStatus::Complete,
            &mut metrics,
        );
    }
    assert_eq!(metrics.production_forensics_anomaly_count, 40);
    assert_eq!(metrics.production_anomaly_ring_overwrite_count, 8);
    let latest = forensics
        .latest_anomaly_for_test()
        .expect("latest ring entry");
    assert_eq!(latest.kind, 4);
    assert_eq!(latest.source_action_index, 39);
}

#[cfg(windows)]
#[test]
#[ignore = "manual Windows component-cost qualification; see docs/dispatch-forensics-cost-qualification.md"]
fn dispatch_cost_qualification() {
    const BATCH_SAMPLES: usize = 512;
    const TARGET_PACKET_OPERATIONS: usize = 512;
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let anomaly_ring_storage_bytes = std::mem::size_of::<[ProductionForensicsAnomaly; 32]>()
        + std::mem::size_of::<[bool; 32]>()
        + std::mem::size_of::<usize>()
        + std::mem::size_of::<u64>();
    println!(
        "#476 forensics-storage anomaly_record_bytes={} ring_capacity=32 production_ring_storage_bytes={anomaly_ring_storage_bytes}",
        std::mem::size_of::<ProductionForensicsAnomaly>(),
    );

    for workload in QualificationWorkload::ALL {
        let raw = workload.raw_cycle();
        let cycles_per_batch = TARGET_PACKET_OPERATIONS.div_ceil(raw.len());
        let mut builder_guard = make_guard();
        let mut measured_guard = make_guard();
        let mut forensics = ProductionHoldForensics::default();
        forensics.set_frame_policies(
            DurationTicks::from_raw(HOLD_TICKS),
            DurationTicks::from_raw(RELEASE_TICKS),
        );
        let mut forensics_metrics = WorkerMetricsLocal::default();
        let backend = TrackedKeyState::default();
        let mut backend_metrics = WorkerMetricsLocal::default();
        let shared = SharedMetrics::default();
        let mut wait_runtime = WorkerRuntime::default();
        let wait_queue = PendingObservationQueue::default();
        let mut wait_metrics = WorkerMetricsLocal::default();
        let mut costs = QualificationCosts::default();
        let mut actual_operations_per_batch = 0;

        for batch_index in 0..BATCH_SAMPLES {
            let samples = make_samples(workload, &mut builder_guard, batch_index, cycles_per_batch);
            let operations = samples.len() as f64;
            actual_operations_per_batch = samples.len();

            let started = Instant::now();
            for (index, sample) in samples.iter().enumerate() {
                forensics.observe_packet_result(
                    sample.packet,
                    index as u32,
                    QpcTicks::from_raw(sample.target_ticks),
                    QpcTicks::from_raw(sample.pre_call_ticks),
                    QpcTicks::from_raw(sample.completion_ticks),
                    SendTransactionStatus::Complete,
                    &mut forensics_metrics,
                );
            }
            costs
                .forensics
                .push(started.elapsed().as_nanos() as f64 / operations);

            let started = Instant::now();
            for sample in &samples {
                let window = measured_guard
                    .query(
                        QpcTicks::from_raw(sample.target_ticks),
                        sample.packet.up_mask,
                        sample.packet.down_mask,
                    )
                    .expect("guard qualification query");
                measured_guard
                    .observe_successful_packet(
                        QpcTicks::from_raw(sample.completion_ticks),
                        sample.packet.up_mask,
                        sample.packet.down_mask,
                    )
                    .expect("guard qualification completion");
                std::hint::black_box(window);
            }
            costs
                .guard_query_and_update
                .push(started.elapsed().as_nanos() as f64 / operations);

            let started = Instant::now();
            for _ in &samples {
                publish_backend_counters(
                    std::hint::black_box(&backend),
                    std::hint::black_box(&mut backend_metrics),
                );
            }
            costs
                .backend_counter_refresh
                .push(started.elapsed().as_nanos() as f64 / operations);
            std::hint::black_box(&backend_metrics);

            let started = Instant::now();
            for sample in &samples {
                publish_live_metrics_after_dispatch(
                    &backend_metrics,
                    &shared,
                    qpc_clock,
                    QpcTicks::from_raw(sample.completion_ticks),
                );
            }
            costs
                .best_effort_snapshot_publication
                .push(started.elapsed().as_nanos() as f64 / operations);

            let started = Instant::now();
            for sample in &samples {
                let wait = record_wait_attempt(
                    Some(&wait_queue),
                    Some(WaitResult {
                        outcome: WaitOutcome::Deadline,
                        wake_qpc: Some(QpcTicks::from_raw(sample.pre_call_ticks)),
                        spin_ticks: DurationTicks::ZERO,
                    }),
                    QpcTicks::from_raw(sample.target_ticks),
                    DurationTicks::from_raw(
                        sample.pre_call_ticks.saturating_sub(sample.target_ticks),
                    ),
                    TimelineTicks::from_raw(sample.target_ticks),
                    QpcTicks::ZERO,
                );
                wait_runtime.pending_wait_observation = wait;
                publish_pending_wait_observation(
                    &mut wait_runtime,
                    Some(&wait_queue),
                    &mut wait_metrics,
                );
                std::hint::black_box(wait_queue.pop_front());
            }
            costs
                .pending_wait_capture_and_queue
                .push(started.elapsed().as_nanos() as f64 / operations);
            assert_eq!(wait_metrics.observer_queue_high_watermark, 1);

            let mut telemetry = TelemetryCollector::new(TelemetryMode::Ring, samples.len());
            let observations = samples
                .iter()
                .enumerate()
                .map(|(index, sample)| make_down_observation(*sample, index as u32))
                .collect::<Vec<_>>();
            let started = Instant::now();
            for observation in &observations {
                record_down_send_telemetry(
                    observation,
                    &mut telemetry,
                    2,
                    5,
                    true,
                    0,
                    0,
                    Some(TimelineTicks::from_raw(observation.pre_call_qpc.as_u64())),
                    Some(TimelineTicks::from_raw(
                        observation.sendinput_completion_qpc.as_u64(),
                    )),
                    Some(TimelineTicks::from_raw(observation.pre_call_qpc.as_u64())),
                    QpcTicks::from_raw(observation.sendinput_completion_qpc.as_u64() + 2),
                    false,
                    false,
                )
                .expect("telemetry qualification record");
            }
            costs
                .telemetry_record_and_ring
                .push(started.elapsed().as_nanos() as f64 / operations);
            assert_eq!(telemetry.output.accepted, samples.len() as u64);
            assert_eq!(telemetry.output.dropped, 0);
        }

        costs.report(workload.name(), BATCH_SAMPLES, actual_operations_per_batch);
        assert_eq!(forensics_metrics.production_hold_floor_violation_count, 0);
        assert_eq!(
            forensics_metrics.production_release_floor_violation_count,
            0
        );
        assert_eq!(wait_metrics.observer_dropped_samples, 0);
    }
}

#[cfg(windows)]
fn make_down_observation(sample: PacketSample, index: u32) -> DownObservation {
    let mut observation = DownObservation::test_fixture(u64::from(index));
    observation.allow_pre_epoch_startup_dispatch = true;
    observation.physical_target_qpc = QpcTicks::from_raw(sample.target_ticks);
    observation.physical_timing_window = sample.timing_window;
    observation.final_policy_qpc = Some(QpcTicks::from_raw(sample.pre_call_ticks));
    observation.pre_call_qpc = QpcTicks::from_raw(sample.pre_call_ticks);
    observation.sendinput_completion_qpc = QpcTicks::from_raw(sample.completion_ticks);
    observation.dispatch_ready_qpc = Some(QpcTicks::from_raw(sample.completion_ticks + 2));
    observation.wake_qpc = Some(QpcTicks::from_raw(sample.pre_call_ticks));
    observation.requested_packet = sample.packet;
    observation.confirmed_mask = sample.packet.up_mask | sample.packet.down_mask;
    observation.skipped_mask = 0;
    observation.trace.event_index = index;
    observation.trace.compiled_packet_index = Some(u64::from(index));
    observation.trace.authored_ticks = TimelineTicks::from_raw(sample.target_ticks);
    observation.trace.effective_deadline_ticks = TimelineTicks::from_raw(sample.target_ticks);
    observation
}

#[cfg(windows)]
#[derive(Default)]
struct QualificationCosts {
    forensics: Vec<f64>,
    guard_query_and_update: Vec<f64>,
    backend_counter_refresh: Vec<f64>,
    best_effort_snapshot_publication: Vec<f64>,
    pending_wait_capture_and_queue: Vec<f64>,
    telemetry_record_and_ring: Vec<f64>,
}

#[cfg(windows)]
impl QualificationCosts {
    fn report(&mut self, workload: &str, batch_samples: usize, target_operations: usize) {
        Self::report_one(
            workload,
            "production-forensics",
            batch_samples,
            target_operations,
            &mut self.forensics,
        );
        Self::report_one(
            workload,
            "guard-query-update",
            batch_samples,
            target_operations,
            &mut self.guard_query_and_update,
        );
        Self::report_one(
            workload,
            "backend-counter-refresh",
            batch_samples,
            target_operations,
            &mut self.backend_counter_refresh,
        );
        Self::report_one(
            workload,
            "best-effort-snapshot-publication",
            batch_samples,
            target_operations,
            &mut self.best_effort_snapshot_publication,
        );
        Self::report_one(
            workload,
            "pending-wait-capture-queue",
            batch_samples,
            target_operations,
            &mut self.pending_wait_capture_and_queue,
        );
        Self::report_one(
            workload,
            "telemetry-record-ring",
            batch_samples,
            target_operations,
            &mut self.telemetry_record_and_ring,
        );
    }

    fn report_one(
        workload: &str,
        component: &str,
        batch_samples: usize,
        target_operations: usize,
        values: &mut Vec<f64>,
    ) {
        values.sort_by(f64::total_cmp);
        let percentile = |rank: usize| -> f64 {
            let index = (values.len() * rank).div_ceil(100).saturating_sub(1);
            values[index.min(values.len() - 1)]
        };
        println!(
            "#476 component-cost workload={workload} component={component} batch_samples={batch_samples} target_packet_ops_per_batch={target_operations} batch_mean_ns_per_packet_p50={:.2} p95={:.2} p99={:.2} max={:.2}",
            percentile(50),
            percentile(95),
            percentile(99),
            values[values.len() - 1],
        );
    }
}
