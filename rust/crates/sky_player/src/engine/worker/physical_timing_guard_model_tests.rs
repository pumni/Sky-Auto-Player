use super::{PhysicalTimingGuard, VALID_KEY_MASK};
use crate::timing_model::{
    ModelPacket, ModelPacketKind, WholeStreamModelConfig, WholeStreamModelReport,
    analyze_whole_stream,
};
use sky_dispatch_core::time::QpcTicks;
use sky_dispatch_win32::clock::QpcClock;
use std::num::NonZeroU64;

fn qpc(value: u64) -> QpcTicks {
    QpcTicks::from_raw(value)
}

fn bit(slot: usize) -> u16 {
    1_u16 << slot
}

fn assert_model_matches_production_guard(
    packets: &[ModelPacket],
    config: WholeStreamModelConfig,
) -> WholeStreamModelReport {
    let frequency = NonZeroU64::new(config.qpc_frequency_hz).expect("nonzero test frequency");
    let clock = QpcClock::from_frequency_hz(frequency);
    let mut guard = PhysicalTimingGuard::new(
        clock
            .duration_from_us(config.effective_min_hold_us)
            .expect("test hold conversion"),
        clock
            .duration_from_us(config.physical_release_gap_us)
            .expect("test release conversion"),
    );
    if let Some(barrier) = config.resume_down_barrier_ticks {
        let frame = clock
            .duration_from_us(config.physical_release_gap_us)
            .expect("test frame conversion")
            .as_u64();
        let cleanup_completed = barrier
            .checked_sub(frame)
            .expect("test barrier includes one frame");
        guard
            .establish_resume_down_barrier(qpc(cleanup_completed))
            .expect("test resume barrier");
    }

    let report = analyze_whole_stream(packets, config).expect("offline timing model");
    assert_eq!(report.packet_count, packets.len());
    assert_eq!(report.packets.len(), packets.len());
    let mut previous_worker_free = None;
    for (packet, estimate) in packets.iter().zip(&report.packets) {
        if packet.kind == ModelPacketKind::Physical {
            let window = guard
                .query(
                    qpc(packet.authored_target_ticks),
                    packet.up_mask,
                    packet.down_mask,
                )
                .expect("production guard query");
            let production_floor_displacement = window
                .packet_not_before_qpc
                .as_u64()
                .checked_sub(packet.authored_target_ticks)
                .expect("production floor is no earlier than authored target");
            assert_eq!(
                estimate.total_floor_displacement_ticks, production_floor_displacement,
                "model and production guard floor differ at packet {}",
                estimate.packet_index
            );

            let production_ready = packet
                .authored_target_ticks
                .max(window.packet_not_before_qpc.as_u64())
                .max(previous_worker_free.unwrap_or(0));
            assert_eq!(
                estimate.modeled_ready_ticks, production_ready,
                "model and production guard readiness differ at packet {}",
                estimate.packet_index
            );

            guard
                .observe_successful_packet(
                    qpc(estimate
                        .modeled_sender_completion_ticks
                        .expect("physical packet completion")),
                    packet.up_mask,
                    packet.down_mask,
                )
                .expect("production completion observation");
        } else {
            assert_eq!(estimate.total_floor_displacement_ticks, 0);
            assert_eq!(
                estimate.modeled_ready_ticks,
                packet
                    .authored_target_ticks
                    .max(previous_worker_free.unwrap_or(0)),
                "metadata readiness retains sequential worker occupancy"
            );
            assert!(estimate.modeled_sender_completion_ticks.is_none());
        }
        previous_worker_free = Some(estimate.modeled_worker_free_ticks);
    }
    report
}

fn model_config() -> WholeStreamModelConfig {
    WholeStreamModelConfig {
        qpc_frequency_hz: 1_000_000,
        effective_min_hold_us: 10,
        physical_release_gap_us: 4,
        estimated_sender_setup_us: 0,
        estimated_per_input_us: 10,
        estimated_post_send_us: 0,
        estimated_metadata_cost_us: 0,
        resume_down_barrier_ticks: None,
    }
}

fn physical(target: u64, up_mask: u16, down_mask: u16) -> ModelPacket {
    ModelPacket {
        authored_target_ticks: target,
        up_mask,
        down_mask,
        kind: ModelPacketKind::Physical,
    }
}

fn metadata(target: u64) -> ModelPacket {
    ModelPacket {
        authored_target_ticks: target,
        up_mask: 0,
        down_mask: 0,
        kind: ModelPacketKind::Metadata,
    }
}

#[test]
fn chord_floor_counterexample_uses_packet_completion_for_every_key() {
    let packets = [physical(0, 0, bit(0) | bit(1)), physical(20, bit(0), 0)];
    let report = assert_model_matches_production_guard(&packets, model_config());

    assert_eq!(report.packets[0].modeled_sender_completion_ticks, Some(20));
    assert_eq!(report.packets[1].modeled_ready_ticks, 30);
}

#[test]
fn differential_model_matches_guard_for_chains_chords_mixed_packets_and_all_slots() {
    let packets = [
        physical(1_000, 0, VALID_KEY_MASK),
        physical(1_200, VALID_KEY_MASK, 0),
        physical(1_300, bit(0) | bit(1), bit(2) | bit(3)),
        physical(1_400, bit(2) | bit(3), bit(0) | bit(1)),
        physical(1_500, 0, bit(0)),
        physical(1_600, bit(0), 0),
        metadata(1_700),
        physical(1_700, 0, bit(14)),
        physical(1_800, bit(14), 0),
    ];
    let config = WholeStreamModelConfig {
        qpc_frequency_hz: 10_000_003,
        effective_min_hold_us: 17_167,
        physical_release_gap_us: 16_667,
        estimated_sender_setup_us: 101,
        estimated_per_input_us: 37,
        estimated_post_send_us: 80,
        estimated_metadata_cost_us: 10,
        resume_down_barrier_ticks: None,
    };

    let report = assert_model_matches_production_guard(&packets, config);
    assert!(report.packets[0].modeled_sender_completion_ticks.is_some());
    assert!(report.packets[6].modeled_sender_completion_ticks.is_none());
    assert!(report.packets[7].modeled_ready_ticks >= report.packets[6].modeled_worker_free_ticks);
}

#[test]
fn differential_model_matches_guard_across_resume_floor_and_zero_cost_packets() {
    let mut resumed = model_config();
    resumed.resume_down_barrier_ticks = Some(2_000_000);
    resumed.estimated_sender_setup_us = 3;
    resumed.estimated_post_send_us = 5;
    resumed.estimated_metadata_cost_us = 7;
    let packets = [
        physical(100, bit(0), 0),
        metadata(200),
        physical(300, 0, bit(0)),
        physical(400, 0, bit(1)),
        physical(2_000_000, 0, bit(2)),
        physical(2_000_100, bit(2), 0),
    ];
    assert_model_matches_production_guard(&packets, resumed);

    let zero_cost = WholeStreamModelConfig {
        qpc_frequency_hz: 10_000_003,
        effective_min_hold_us: 0,
        physical_release_gap_us: 0,
        estimated_sender_setup_us: 0,
        estimated_per_input_us: 0,
        estimated_post_send_us: 0,
        estimated_metadata_cost_us: 0,
        resume_down_barrier_ticks: None,
    };
    let zero_cost_packets = [
        physical(40, 0, bit(0)),
        physical(40, bit(0), 0),
        physical(40, 0, bit(0)),
        metadata(40),
    ];
    assert_model_matches_production_guard(&zero_cost_packets, zero_cost);
}

#[test]
fn differential_model_preserves_worker_hol_after_physical_and_metadata_work() {
    let config = WholeStreamModelConfig {
        qpc_frequency_hz: 3,
        effective_min_hold_us: 0,
        physical_release_gap_us: 0,
        estimated_sender_setup_us: 333_334,
        estimated_per_input_us: 333_334,
        estimated_post_send_us: 333_334,
        estimated_metadata_cost_us: 1_000_000,
        resume_down_barrier_ticks: None,
    };
    let packets = [physical(0, 0, bit(0)), metadata(1), physical(2, 0, bit(1))];
    let report = assert_model_matches_production_guard(&packets, config);
    assert_eq!(report.packets[0].modeled_sender_completion_ticks, Some(4));
    assert_eq!(report.packets[0].modeled_worker_free_ticks, 6);
    assert_eq!(report.packets[1].modeled_worker_free_ticks, 9);
    assert_eq!(report.packets[2].hol_delay_lower_bound_ticks, 7);
    assert_eq!(report.packets[2].modeled_ready_ticks, 9);
}
