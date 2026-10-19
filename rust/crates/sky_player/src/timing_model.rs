//! Offline whole-stream timing and headroom estimates.
//!
//! This module is deliberately disconnected from worker admission and packet
//! execution. Its estimates cannot delay, authorize, reorder, retry, or drop a
//! production packet.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

pub const INSTRUMENT_SLOT_COUNT: usize = 15;
const INSTRUMENT_MASK: u16 = (1 << INSTRUMENT_SLOT_COUNT) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPacketKind {
    Physical,
    Metadata,
}

/// One immutable, ordered schedule boundary. Times are relative authored QPC
/// ticks; masks use the canonical fifteen instrument slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ModelPacket {
    pub authored_target_ticks: u64,
    pub up_mask: u16,
    pub down_mask: u16,
    pub kind: ModelPacketKind,
}

/// Explicit assumptions used only by [`analyze_whole_stream`]. Durations are
/// estimates in microseconds and are rounded up into QPC ticks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct WholeStreamModelConfig {
    pub qpc_frequency_hz: u64,
    pub effective_min_hold_us: u64,
    /// Physical same-key Up-completion to next-Down floor (one frame).
    pub physical_release_gap_us: u64,
    pub estimated_sender_setup_us: u64,
    pub estimated_per_input_us: u64,
    pub estimated_post_send_us: u64,
    pub estimated_metadata_cost_us: u64,
    /// Known absolute relative-to-epoch barrier. It applies to Down packets.
    pub resume_down_barrier_ticks: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlackClass {
    Positive,
    Zero,
    Negative,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WholeStreamPacketEstimate {
    pub packet_index: usize,
    pub authored_target_ticks: u64,
    pub modeled_ready_ticks: u64,
    pub modeled_sender_completion_ticks: Option<u64>,
    pub modeled_worker_free_ticks: u64,
    pub h_floor_displacement_ticks: u64,
    pub t_floor_displacement_ticks: u64,
    pub resume_floor_displacement_ticks: u64,
    pub total_floor_displacement_ticks: u64,
    pub hol_delay_lower_bound_ticks: u64,
    pub modeled_lateness_ticks: u64,
    /// `authored_target - max(H/T/resume/previous-worker constraints)`.
    /// Negative values predict a late boundary under these assumptions.
    pub signed_slack_ticks: i64,
    pub slack_class: SlackClass,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TickPercentiles {
    pub p50: i64,
    pub p95: i64,
    pub p99: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WholeStreamModelReport {
    pub model_kind: &'static str,
    pub guarantee: bool,
    pub packet_count: usize,
    pub positive_slack_count: u64,
    pub zero_slack_count: u64,
    pub negative_slack_count: u64,
    pub slack_ticks: TickPercentiles,
    pub packets: Vec<WholeStreamPacketEstimate>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WholeStreamModelError {
    Cancelled,
    ZeroQpcFrequency,
    InvalidPacketMask,
    InvalidPacketKind,
    NonChronologicalTargets {
        previous_index: usize,
        current_index: usize,
    },
    ArithmeticOverflow,
}

impl std::fmt::Display for WholeStreamModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("whole-stream model analysis was cancelled"),
            Self::ZeroQpcFrequency => f.write_str("QPC frequency must be nonzero"),
            Self::InvalidPacketMask => {
                f.write_str("packet masks exceed or overlap the instrument slots")
            }
            Self::InvalidPacketKind => {
                f.write_str("metadata packets must have empty masks and physical packets must not")
            }
            Self::NonChronologicalTargets {
                previous_index,
                current_index,
            } => write!(
                f,
                "authored target at packet {current_index} precedes packet {previous_index}"
            ),
            Self::ArithmeticOverflow => f.write_str("whole-stream model arithmetic overflow"),
        }
    }
}

impl std::error::Error for WholeStreamModelError {}

/// Simulate the complete sequential schedule using explicit cost assumptions.
/// Up events in a mixed packet precede every Down event, matching the physical
/// packet contract. Every key in a successful physical packet receives floors
/// from that packet's common sender-completion estimate. Every operation uses
/// checked arithmetic. This is a risk estimate, never a production timing
/// guarantee.
pub fn analyze_whole_stream(
    packets: &[ModelPacket],
    config: WholeStreamModelConfig,
) -> Result<WholeStreamModelReport, WholeStreamModelError> {
    analyze_whole_stream_inner(packets, config, None)
}

/// Cancellable form used by bounded Prepare-time analysis. The caller must
/// bound `packets` before calling; cancellation is checked before allocation,
/// during the fixed-work packet loop, and before percentile sorting.
pub fn analyze_whole_stream_cancellable(
    packets: &[ModelPacket],
    config: WholeStreamModelConfig,
    cancelled: &AtomicBool,
) -> Result<WholeStreamModelReport, WholeStreamModelError> {
    analyze_whole_stream_inner(packets, config, Some(cancelled))
}

fn analyze_whole_stream_inner(
    packets: &[ModelPacket],
    config: WholeStreamModelConfig,
    cancelled: Option<&AtomicBool>,
) -> Result<WholeStreamModelReport, WholeStreamModelError> {
    check_cancelled(cancelled)?;
    if config.qpc_frequency_hz == 0 {
        return Err(WholeStreamModelError::ZeroQpcFrequency);
    }
    let h_ticks = us_to_ticks_ceil(config.effective_min_hold_us, config.qpc_frequency_hz)?;
    let t_ticks = us_to_ticks_ceil(config.physical_release_gap_us, config.qpc_frequency_hz)?;
    let setup_ticks = us_to_ticks_ceil(config.estimated_sender_setup_us, config.qpc_frequency_hz)?;
    let input_ticks = us_to_ticks_ceil(config.estimated_per_input_us, config.qpc_frequency_hz)?;
    let post_send_ticks = us_to_ticks_ceil(config.estimated_post_send_us, config.qpc_frequency_hz)?;
    let metadata_ticks =
        us_to_ticks_ceil(config.estimated_metadata_cost_us, config.qpc_frequency_hz)?;

    let mut h_not_before = [None; INSTRUMENT_SLOT_COUNT];
    let mut t_not_before = [None; INSTRUMENT_SLOT_COUNT];
    let mut previous_worker_free: Option<u64> = None;
    let mut rows = Vec::with_capacity(packets.len());
    let mut positive_slack_count = 0u64;
    let mut zero_slack_count = 0u64;
    let mut negative_slack_count = 0u64;
    let mut slack_values = Vec::with_capacity(packets.len());
    let mut previous_target: Option<(usize, u64)> = None;

    for (packet_index, packet) in packets.iter().copied().enumerate() {
        if packet_index % 256 == 0 {
            check_cancelled(cancelled)?;
        }
        if let Some((previous_index, previous_ticks)) = previous_target
            && packet.authored_target_ticks < previous_ticks
        {
            return Err(WholeStreamModelError::NonChronologicalTargets {
                previous_index,
                current_index: packet_index,
            });
        }
        previous_target = Some((packet_index, packet.authored_target_ticks));

        if (packet.up_mask | packet.down_mask) & !INSTRUMENT_MASK != 0
            || packet.up_mask & packet.down_mask != 0
        {
            return Err(WholeStreamModelError::InvalidPacketMask);
        }
        let physical = packet.kind == ModelPacketKind::Physical;
        if (physical && packet.up_mask | packet.down_mask == 0)
            || (!physical && (packet.up_mask != 0 || packet.down_mask != 0))
        {
            return Err(WholeStreamModelError::InvalidPacketKind);
        }

        let mut h_floor: Option<u64> = None;
        let mut t_floor: Option<u64> = None;
        if packet.up_mask != 0 {
            let mut mask = packet.up_mask;
            while mask != 0 {
                let slot = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                h_floor = Some(h_floor.unwrap_or(0).max(h_not_before[slot].unwrap_or(0)));
            }
        }
        if packet.down_mask != 0 {
            let mut mask = packet.down_mask;
            while mask != 0 {
                let slot = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                t_floor = Some(t_floor.unwrap_or(0).max(t_not_before[slot].unwrap_or(0)));
            }
        }
        let resume_floor = if packet.down_mask != 0 {
            config.resume_down_barrier_ticks
        } else {
            None
        };
        let floor_ready = packet
            .authored_target_ticks
            .max(h_floor.unwrap_or(packet.authored_target_ticks))
            .max(t_floor.unwrap_or(packet.authored_target_ticks))
            .max(resume_floor.unwrap_or(packet.authored_target_ticks));
        let constraint_ready = h_floor
            .unwrap_or(0)
            .max(t_floor.unwrap_or(0))
            .max(resume_floor.unwrap_or(0))
            .max(previous_worker_free.unwrap_or(0));
        let ready = packet.authored_target_ticks.max(constraint_ready);
        let slack = signed_difference(packet.authored_target_ticks, constraint_ready)?;
        slack_values.push(slack);
        match slack.cmp(&0) {
            std::cmp::Ordering::Greater => {
                positive_slack_count = positive_slack_count
                    .checked_add(1)
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
            }
            std::cmp::Ordering::Equal => {
                zero_slack_count = zero_slack_count
                    .checked_add(1)
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
            }
            std::cmp::Ordering::Less => {
                negative_slack_count = negative_slack_count
                    .checked_add(1)
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
            }
        }

        let h_displacement = h_floor
            .map(|floor| floor.max(packet.authored_target_ticks) - packet.authored_target_ticks)
            .unwrap_or(0);
        let t_displacement = t_floor
            .map(|floor| floor.max(packet.authored_target_ticks) - packet.authored_target_ticks)
            .unwrap_or(0);
        let resume_displacement = resume_floor
            .map(|floor| floor.max(packet.authored_target_ticks) - packet.authored_target_ticks)
            .unwrap_or(0);
        let total_floor_displacement = floor_ready
            .checked_sub(packet.authored_target_ticks)
            .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
        // HOL is a nonnegative lower bound by definition; overlap between the
        // prior worker and a later floor contributes zero rather than a
        // negative delay.
        let hol_delay = previous_worker_free
            .map(|free| free.saturating_sub(floor_ready))
            .unwrap_or(0);
        let modeled_lateness = ready
            .checked_sub(packet.authored_target_ticks)
            .ok_or(WholeStreamModelError::ArithmeticOverflow)?;

        let (sender_completion, worker_free) = if physical {
            let event_count =
                u64::from(packet.up_mask.count_ones() + packet.down_mask.count_ones());
            let event_cost = input_ticks
                .checked_mul(event_count)
                .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
            let envelope = setup_ticks
                .checked_add(event_cost)
                .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
            let sender_completion = ready
                .checked_add(envelope)
                .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
            let worker_free = sender_completion
                .checked_add(post_send_ticks)
                .ok_or(WholeStreamModelError::ArithmeticOverflow)?;

            // SendInput observes one packet completion, not per-key event
            // completions. The production guard applies that same completion
            // to every member and replaces the opposite-direction floor.
            let next_t_floor = if packet.up_mask != 0 {
                Some(
                    sender_completion
                        .checked_add(t_ticks)
                        .ok_or(WholeStreamModelError::ArithmeticOverflow)?,
                )
            } else {
                None
            };
            let next_h_floor = if packet.down_mask != 0 {
                Some(
                    sender_completion
                        .checked_add(h_ticks)
                        .ok_or(WholeStreamModelError::ArithmeticOverflow)?,
                )
            } else {
                None
            };

            let mut up_mask = packet.up_mask;
            while up_mask != 0 {
                let slot = up_mask.trailing_zeros() as usize;
                up_mask &= up_mask - 1;
                h_not_before[slot] = None;
                t_not_before[slot] = next_t_floor;
            }
            let mut down_mask = packet.down_mask;
            while down_mask != 0 {
                let slot = down_mask.trailing_zeros() as usize;
                down_mask &= down_mask - 1;
                t_not_before[slot] = None;
                h_not_before[slot] = next_h_floor;
            }
            (Some(sender_completion), worker_free)
        } else {
            let worker_free = ready
                .checked_add(metadata_ticks)
                .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
            (None, worker_free)
        };
        previous_worker_free = Some(worker_free);

        rows.push(WholeStreamPacketEstimate {
            packet_index,
            authored_target_ticks: packet.authored_target_ticks,
            modeled_ready_ticks: ready,
            modeled_sender_completion_ticks: sender_completion,
            modeled_worker_free_ticks: worker_free,
            h_floor_displacement_ticks: h_displacement,
            t_floor_displacement_ticks: t_displacement,
            resume_floor_displacement_ticks: resume_displacement,
            total_floor_displacement_ticks: total_floor_displacement,
            hol_delay_lower_bound_ticks: hol_delay,
            modeled_lateness_ticks: modeled_lateness,
            signed_slack_ticks: slack,
            slack_class: if slack > 0 {
                SlackClass::Positive
            } else if slack == 0 {
                SlackClass::Zero
            } else {
                SlackClass::Negative
            },
        });
    }

    check_cancelled(cancelled)?;
    let slack_ticks = percentiles(&mut slack_values);
    Ok(WholeStreamModelReport {
        model_kind: "offline_whole_stream_risk_estimate",
        guarantee: false,
        packet_count: rows.len(),
        positive_slack_count,
        zero_slack_count,
        negative_slack_count,
        slack_ticks,
        packets: rows,
    })
}

fn check_cancelled(cancelled: Option<&AtomicBool>) -> Result<(), WholeStreamModelError> {
    if cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Acquire)) {
        Err(WholeStreamModelError::Cancelled)
    } else {
        Ok(())
    }
}

fn us_to_ticks_ceil(us: u64, frequency_hz: u64) -> Result<u64, WholeStreamModelError> {
    let product = u128::from(us)
        .checked_mul(u128::from(frequency_hz))
        .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
    let ticks = product
        .checked_add(999_999)
        .ok_or(WholeStreamModelError::ArithmeticOverflow)?
        / 1_000_000;
    u64::try_from(ticks).map_err(|_| WholeStreamModelError::ArithmeticOverflow)
}

fn signed_difference(left: u64, right: u64) -> Result<i64, WholeStreamModelError> {
    let difference = i128::from(left) - i128::from(right);
    i64::try_from(difference).map_err(|_| WholeStreamModelError::ArithmeticOverflow)
}

fn percentiles(values: &mut [i64]) -> TickPercentiles {
    if values.is_empty() {
        return TickPercentiles::default();
    }
    values.sort_unstable();
    let at = |percent: usize| {
        let rank = values.len().saturating_mul(percent).div_ceil(100).max(1);
        values[rank.saturating_sub(1).min(values.len() - 1)]
    };
    TickPercentiles {
        p50: at(50),
        p95: at(95),
        p99: at(99),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        INSTRUMENT_SLOT_COUNT, ModelPacket, ModelPacketKind, SlackClass, TickPercentiles,
        WholeStreamModelConfig, WholeStreamModelError, analyze_whole_stream,
        analyze_whole_stream_cancellable,
    };
    use std::sync::atomic::AtomicBool;

    fn config() -> WholeStreamModelConfig {
        WholeStreamModelConfig {
            qpc_frequency_hz: 1_000_000,
            effective_min_hold_us: 10,
            physical_release_gap_us: 4,
            estimated_sender_setup_us: 0,
            estimated_per_input_us: 1,
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

    #[test]
    fn golden_repeat_chain_reports_h_and_t_floor_displacement_and_signed_slack() {
        let report = analyze_whole_stream(
            &[
                physical(0, 0, 1),
                physical(10, 1, 0),
                physical(14, 0, 1),
                physical(24, 1, 0),
            ],
            config(),
        )
        .expect("repeat-chain model");
        assert_eq!(
            report
                .packets
                .iter()
                .map(|packet| packet.modeled_ready_ticks)
                .collect::<Vec<_>>(),
            [0, 11, 16, 27]
        );
        assert_eq!(report.packets[1].h_floor_displacement_ticks, 1);
        assert_eq!(report.packets[2].t_floor_displacement_ticks, 2);
        assert_eq!(report.packets[2].signed_slack_ticks, -2);
        assert_eq!(report.packets[2].slack_class, SlackClass::Negative);
        assert_eq!(report.negative_slack_count, 3);
        assert_eq!(
            report.slack_ticks,
            TickPercentiles {
                p50: -2,
                p95: 0,
                p99: 0,
            }
        );
        assert!(!report.guarantee);
    }

    #[test]
    fn golden_unrelated_key_exposes_sequential_sender_hol() {
        let mut assumptions = config();
        assumptions.effective_min_hold_us = 0;
        assumptions.estimated_per_input_us = 8;
        assumptions.estimated_post_send_us = 5;
        let report = analyze_whole_stream(&[physical(0, 0, 1), physical(10, 0, 2)], assumptions)
            .expect("unrelated-key model");
        assert_eq!(report.packets[1].hol_delay_lower_bound_ticks, 3);
        assert_eq!(report.packets[1].modeled_lateness_ticks, 3);
        assert_eq!(report.packets[1].signed_slack_ticks, -3);
    }

    #[test]
    fn golden_mixed_packet_orders_ups_before_downs_and_keeps_long_authored_up() {
        let mut assumptions = config();
        assumptions.estimated_sender_setup_us = 2;
        assumptions.estimated_per_input_us = 3;
        let mixed = physical(0, 1, 2);
        let long_up = physical(1_000_000, 0, 1);
        let report =
            analyze_whole_stream(&[mixed, long_up], assumptions).expect("mixed and long-up model");
        assert_eq!(report.packets[0].modeled_sender_completion_ticks, Some(8));
        assert_eq!(report.packets[1].authored_target_ticks, 1_000_000);
        assert_eq!(report.packets[1].modeled_ready_ticks, 1_000_000);
        assert_eq!(report.packets[1].t_floor_displacement_ticks, 0);
        assert_eq!(report.packets[1].signed_slack_ticks, 999_988);
        assert_eq!(report.positive_slack_count, 1);
    }

    #[test]
    fn golden_zero_slack_and_resume_barrier_are_reported_without_rewriting_targets() {
        let mut assumptions = config();
        assumptions.effective_min_hold_us = 0;
        assumptions.estimated_per_input_us = 0;
        assumptions.resume_down_barrier_ticks = Some(20);
        let report =
            analyze_whole_stream(&[physical(20, 0, 1)], assumptions).expect("resume barrier model");
        assert_eq!(report.packets[0].authored_target_ticks, 20);
        assert_eq!(report.packets[0].signed_slack_ticks, 0);
        assert_eq!(report.packets[0].slack_class, SlackClass::Zero);
        assert_eq!(report.zero_slack_count, 1);
    }

    #[test]
    fn unusual_qpc_frequency_uses_ceiling_conversion_and_checks_overflow() {
        let mut assumptions = config();
        assumptions.qpc_frequency_hz = 3;
        assumptions.effective_min_hold_us = 1;
        assumptions.physical_release_gap_us = 333_333;
        assumptions.estimated_per_input_us = 333_334;
        let report = analyze_whole_stream(&[physical(0, 0, 1)], assumptions)
            .expect("unusual frequency model");
        assert_eq!(report.packets[0].modeled_sender_completion_ticks, Some(2));

        let overflow = analyze_whole_stream(
            &[physical(u64::MAX - 1, 0, 1)],
            WholeStreamModelConfig {
                qpc_frequency_hz: 1_000_000,
                effective_min_hold_us: 0,
                physical_release_gap_us: 0,
                estimated_sender_setup_us: 10,
                estimated_per_input_us: 0,
                estimated_post_send_us: 0,
                estimated_metadata_cost_us: 0,
                resume_down_barrier_ticks: None,
            },
        );
        assert_eq!(overflow, Err(WholeStreamModelError::ArithmeticOverflow));
    }

    #[test]
    fn cancellable_model_rejects_a_cancelled_request_before_allocating_rows() {
        let cancelled = AtomicBool::new(true);
        let result = analyze_whole_stream_cancellable(&[], config(), &cancelled);
        assert_eq!(result, Err(WholeStreamModelError::Cancelled));
    }

    #[test]
    fn completion_based_hold_and_release_floor_addition_checks_overflow() {
        let completion = i64::MAX as u64;
        let huge_duration_us = 1_u64 << 63;
        let mut hold_overflow = config();
        hold_overflow.effective_min_hold_us = huge_duration_us;
        hold_overflow.estimated_per_input_us = 1;
        assert_eq!(
            analyze_whole_stream(&[physical(completion, 0, 1)], hold_overflow),
            Err(WholeStreamModelError::ArithmeticOverflow)
        );

        let mut release_overflow = config();
        release_overflow.physical_release_gap_us = huge_duration_us;
        release_overflow.estimated_per_input_us = 1;
        assert_eq!(
            analyze_whole_stream(&[physical(completion, 1, 0)], release_overflow),
            Err(WholeStreamModelError::ArithmeticOverflow)
        );
    }

    #[test]
    fn malformed_masks_and_zero_frequency_fail_closed() {
        assert_eq!(
            analyze_whole_stream(&[physical(0, 1, 1)], config()),
            Err(WholeStreamModelError::InvalidPacketMask)
        );
        assert_eq!(
            analyze_whole_stream(&[physical(0, 1 << INSTRUMENT_SLOT_COUNT, 0)], config()),
            Err(WholeStreamModelError::InvalidPacketMask)
        );
        assert_eq!(
            analyze_whole_stream(
                &[],
                WholeStreamModelConfig {
                    qpc_frequency_hz: 0,
                    ..config()
                }
            ),
            Err(WholeStreamModelError::ZeroQpcFrequency)
        );
    }

    #[test]
    fn chronological_input_accepts_increasing_and_equal_targets_with_mixed_packets() {
        let mixed = physical(10, 0b0001, 0b0010);
        let metadata = ModelPacket {
            authored_target_ticks: 10,
            up_mask: 0,
            down_mask: 0,
            kind: ModelPacketKind::Metadata,
        };
        let later = physical(20, 0, 0b0100);
        let report = analyze_whole_stream(&[mixed, metadata, later], config())
            .expect("nondecreasing authored targets");

        assert_eq!(report.packet_count, 3);
        assert_eq!(report.packets[0].authored_target_ticks, 10);
        assert_eq!(report.packets[1].authored_target_ticks, 10);
        assert_eq!(report.packets[2].authored_target_ticks, 20);
        assert_eq!(report.packets[0].packet_index, 0);
        assert_eq!(report.packets[1].packet_index, 1);
        assert_eq!(report.packets[2].packet_index, 2);
    }

    #[test]
    fn decreasing_authored_target_is_a_typed_error_without_sorting() {
        let error = analyze_whole_stream(&[physical(10, 0, 1), physical(9, 0, 2)], config())
            .expect_err("decreasing authored target must be rejected");
        assert_eq!(
            error,
            WholeStreamModelError::NonChronologicalTargets {
                previous_index: 0,
                current_index: 1,
            }
        );
    }

    #[test]
    fn invalid_packet_kinds_are_rejected_for_both_empty_and_nonempty_masks() {
        let metadata_with_mask = ModelPacket {
            authored_target_ticks: 0,
            up_mask: 1,
            down_mask: 0,
            kind: ModelPacketKind::Metadata,
        };
        let empty_physical = physical(0, 0, 0);
        for packet in [metadata_with_mask, empty_physical] {
            assert_eq!(
                analyze_whole_stream(&[packet], config()),
                Err(WholeStreamModelError::InvalidPacketKind)
            );
        }
    }
}
