use super::super::super::{DurationTicks, QpcClock};
use super::super::WorkerMetricsLocal;
use super::DispatchStep;
use super::observation::ObserverLifecycle;
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::input::PhysicalPacket;

const MAX_KEYS: usize = sky_dispatch_core::model::MAX_KEYS;

#[derive(Clone, Copy, Debug)]
struct HoldAnchor {
    target_qpc: QpcTicks,
    pre_call_qpc: QpcTicks,
    completion_qpc: QpcTicks,
}

#[derive(Default)]
pub(crate) struct HoldForensics {
    active: [Option<HoldAnchor>; MAX_KEYS],
}

impl HoldForensics {
    pub(crate) fn observe_lifecycle(&mut self, lifecycle: ObserverLifecycle) {
        match lifecycle {
            ObserverLifecycle::RecoveryUp { up_mask } => self.clear_mask(up_mask),
            ObserverLifecycle::ResetAll => self.active.fill(None),
        }
    }

    fn clear_mask(&mut self, mask: u16) {
        for slot in 0..MAX_KEYS {
            if mask & (1u16 << slot) != 0 {
                self.active[slot] = None;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_packet(
        &mut self,
        packet: PhysicalPacket,
        target_qpc: QpcTicks,
        pre_call_qpc: QpcTicks,
        completion_qpc: QpcTicks,
        full_transport_success: bool,
        metrics: &mut WorkerMetricsLocal,
        qpc_clock: QpcClock,
        down_late_grace_ticks: DurationTicks,
    ) -> Result<(), DispatchStep> {
        if !full_transport_success {
            return Ok(());
        }

        let retrigger_mask = packet.up_mask & packet.down_mask;
        if retrigger_mask != 0 {
            metrics.same_call_retrigger_boundaries =
                metrics.same_call_retrigger_boundaries.saturating_add(1);
            metrics.same_call_retrigger_keys = metrics
                .same_call_retrigger_keys
                .saturating_add(u64::from(retrigger_mask.count_ones()));
        }

        // PhysicalPacket/build_inputs canonical order is all UP, then all DOWN.
        for slot in 0..MAX_KEYS {
            let bit = 1u16 << slot;
            if packet.up_mask & bit == 0 {
                continue;
            }
            if let Some(anchor) = self.active[slot].take() {
                observe_hold_pair(
                    anchor,
                    target_qpc,
                    pre_call_qpc,
                    completion_qpc,
                    metrics,
                    qpc_clock,
                    down_late_grace_ticks,
                )?;
            } else {
                metrics.hold_unmatched_up_count = metrics.hold_unmatched_up_count.saturating_add(1);
            }
        }

        for slot in 0..MAX_KEYS {
            let bit = 1u16 << slot;
            if packet.down_mask & bit == 0 {
                continue;
            }
            if self.active[slot].is_some() {
                metrics.hold_anchor_overwrite_count =
                    metrics.hold_anchor_overwrite_count.saturating_add(1);
            }
            self.active[slot] = Some(HoldAnchor {
                target_qpc,
                pre_call_qpc,
                completion_qpc,
            });
        }
        Ok(())
    }
}

fn observe_hold_pair(
    anchor: HoldAnchor,
    up_target_qpc: QpcTicks,
    up_pre_call_qpc: QpcTicks,
    up_completion_qpc: QpcTicks,
    metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
    down_late_grace_ticks: DurationTicks,
) -> Result<(), DispatchStep> {
    let authored_hold_ticks = up_target_qpc
        .checked_duration_since(anchor.target_qpc)
        .map_err(|error| {
            DispatchStep::Terminate(format!("hold forensics target ordering failure: {error}"))
        })?;
    let pre_call_hold_ticks = up_pre_call_qpc
        .checked_duration_since(anchor.pre_call_qpc)
        .map_err(|error| {
            DispatchStep::Terminate(format!("hold forensics pre-call ordering failure: {error}"))
        })?;
    let completion_hold_ticks = up_completion_qpc
        .checked_duration_since(anchor.completion_qpc)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "hold forensics completion ordering failure: {error}"
            ))
        })?;

    let pre_call_hold_us = qpc_clock
        .duration_to_us(pre_call_hold_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "hold forensics pre-call conversion failure: {error:?}"
            ))
        })?;
    let completion_hold_us = qpc_clock
        .duration_to_us(completion_hold_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "hold forensics completion conversion failure: {error:?}"
            ))
        })?;
    let pre_call_shrink_ticks = authored_hold_ticks
        .as_u64()
        .saturating_sub(pre_call_hold_ticks.as_u64());
    let completion_shrink_ticks = authored_hold_ticks
        .as_u64()
        .saturating_sub(completion_hold_ticks.as_u64());
    let pre_call_shrink_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(pre_call_shrink_ticks))
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "hold forensics pre-call shrink conversion failure: {error:?}"
            ))
        })?;
    let completion_shrink_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(completion_shrink_ticks))
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "hold forensics completion shrink conversion failure: {error:?}"
            ))
        })?;

    let first_sample = metrics.hold_pair_samples == 0;
    metrics.hold_pair_samples = metrics.hold_pair_samples.saturating_add(1);
    if first_sample {
        metrics.min_pre_call_hold_us = pre_call_hold_us;
        metrics.min_completion_hold_us = completion_hold_us;
    } else {
        metrics.min_pre_call_hold_us = metrics.min_pre_call_hold_us.min(pre_call_hold_us);
        metrics.min_completion_hold_us = metrics.min_completion_hold_us.min(completion_hold_us);
    }
    metrics.max_pre_call_hold_shrink_us =
        metrics.max_pre_call_hold_shrink_us.max(pre_call_shrink_us);
    metrics.max_completion_hold_shrink_us = metrics
        .max_completion_hold_shrink_us
        .max(completion_shrink_us);
    if pre_call_shrink_ticks > down_late_grace_ticks.as_u64() {
        metrics.pre_call_hold_shrink_over_grace_count = metrics
            .pre_call_hold_shrink_over_grace_count
            .saturating_add(1);
    }
    Ok(())
}

/// Fixed-size sender evidence owned by the worker itself. It deliberately
/// consumes the timestamps already returned by the trusted sender: it does
/// not sample QPC, allocate, lock, format, or walk the schedule.
pub(crate) const PRODUCTION_FORENSICS_VERSION: u32 = 1;
const PRODUCTION_ANOMALY_CAPACITY: usize = 32;

#[derive(Clone, Copy, Default)]
struct ProductionHoldAnchor {
    valid: bool,
    target_ticks: u64,
    pre_call_ticks: u64,
    completion_ticks: u64,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct ProductionForensicsAnomaly {
    kind: u8,
    slot: u8,
    source_action_index: u32,
    mask: u16,
    target_ticks: u64,
    observed_ticks: u64,
    aux_ticks: u64,
    delta_ticks: u64,
}

#[derive(Default)]
pub(crate) struct ProductionHoldForensics {
    anchors: [ProductionHoldAnchor; MAX_KEYS],
    last_up_completion_ticks: [Option<u64>; MAX_KEYS],
    anomalies: [ProductionForensicsAnomaly; PRODUCTION_ANOMALY_CAPACITY],
    anomaly_valid: [bool; PRODUCTION_ANOMALY_CAPACITY],
    next_anomaly: usize,
    anomaly_count: u64,
    anomaly_ring_overwrites: u64,
    pair_samples: u64,
    min_pre_call_hold_ticks: u64,
    min_completion_hold_ticks: u64,
    max_pre_call_shrink_ticks: u64,
    max_completion_shrink_ticks: u64,
    completion_hold_below_frame_count: u64,
    release_gap_samples: u64,
    min_release_gap_ticks: u64,
    release_gap_below_policy_count: u64,
    same_call_same_key_retrigger_count: u64,
    anchor_overwrite_count: u64,
    unmatched_up_count: u64,
    frame_policy_ticks: u64,
    release_gap_policy_ticks: u64,
}

impl ProductionHoldForensics {
    pub(crate) fn set_frame_policies(
        &mut self,
        frame_policy_ticks: DurationTicks,
        release_gap_policy_ticks: DurationTicks,
    ) {
        self.frame_policy_ticks = frame_policy_ticks.as_u64();
        self.release_gap_policy_ticks = release_gap_policy_ticks.as_u64();
    }

    pub(crate) fn observe_lifecycle(&mut self, lifecycle: ObserverLifecycle) {
        match lifecycle {
            ObserverLifecycle::RecoveryUp { up_mask } => self.clear_mask(up_mask),
            ObserverLifecycle::ResetAll => {
                self.anchors.fill(ProductionHoldAnchor::default());
                self.last_up_completion_ticks.fill(None);
            }
        }
    }

    fn clear_mask(&mut self, mask: u16) {
        for slot in 0..MAX_KEYS {
            if mask & (1u16 << slot) != 0 {
                self.anchors[slot] = ProductionHoldAnchor::default();
                self.last_up_completion_ticks[slot] = None;
            }
        }
    }

    pub(crate) fn observe_packet_result(
        &mut self,
        packet: PhysicalPacket,
        target_qpc: QpcTicks,
        pre_call_qpc: QpcTicks,
        completion_qpc: QpcTicks,
        status: sky_dispatch_win32::input::SendTransactionStatus,
        metrics: &mut WorkerMetricsLocal,
    ) {
        self.observe_packet(
            packet,
            target_qpc,
            pre_call_qpc,
            completion_qpc,
            matches!(
                status,
                sky_dispatch_win32::input::SendTransactionStatus::Complete
            ),
            metrics,
        );
    }

    pub(crate) fn observe_packet(
        &mut self,
        packet: PhysicalPacket,
        target_qpc: QpcTicks,
        pre_call_qpc: QpcTicks,
        completion_qpc: QpcTicks,
        full_transport_success: bool,
        metrics: &mut WorkerMetricsLocal,
    ) {
        if !full_transport_success {
            return;
        }
        let target = target_qpc.as_u64();
        let pre_call = pre_call_qpc.as_u64();
        let completion = completion_qpc.as_u64();
        for slot in 0..MAX_KEYS {
            let bit = 1u16 << slot;
            if packet.up_mask & bit == 0 {
                continue;
            }
            if self.anchors[slot].valid {
                let anchor = self.anchors[slot];
                if packet.down_mask & bit != 0 {
                    self.same_call_same_key_retrigger_count =
                        self.same_call_same_key_retrigger_count.saturating_add(1);
                    self.record_anomaly(6, slot);
                }
                let Some(authored_hold) = target.checked_sub(anchor.target_ticks) else {
                    self.record_anomaly(1, slot);
                    self.anchors[slot].valid = false;
                    continue;
                };
                let Some(pre_call_hold) = pre_call.checked_sub(anchor.pre_call_ticks) else {
                    self.record_anomaly(2, slot);
                    self.anchors[slot].valid = false;
                    continue;
                };
                let Some(completion_hold) = completion.checked_sub(anchor.completion_ticks) else {
                    self.record_anomaly(3, slot);
                    self.anchors[slot].valid = false;
                    continue;
                };
                self.pair_samples = self.pair_samples.saturating_add(1);
                if self.pair_samples == 1 {
                    self.min_pre_call_hold_ticks = pre_call_hold;
                    self.min_completion_hold_ticks = completion_hold;
                } else {
                    self.min_pre_call_hold_ticks = self.min_pre_call_hold_ticks.min(pre_call_hold);
                    self.min_completion_hold_ticks =
                        self.min_completion_hold_ticks.min(completion_hold);
                }
                let pre_call_shrink = authored_hold.saturating_sub(pre_call_hold);
                let completion_shrink = authored_hold.saturating_sub(completion_hold);
                self.max_pre_call_shrink_ticks =
                    self.max_pre_call_shrink_ticks.max(pre_call_shrink);
                self.max_completion_shrink_ticks =
                    self.max_completion_shrink_ticks.max(completion_shrink);
                if self.frame_policy_ticks > 0 && completion_hold < self.frame_policy_ticks {
                    self.completion_hold_below_frame_count =
                        self.completion_hold_below_frame_count.saturating_add(1);
                }
                self.last_up_completion_ticks[slot] = Some(completion);
            } else {
                self.unmatched_up_count = self.unmatched_up_count.saturating_add(1);
                self.record_anomaly(4, slot);
            }
            self.anchors[slot].valid = false;
        }
        for slot in 0..MAX_KEYS {
            let bit = 1u16 << slot;
            if packet.down_mask & bit == 0 {
                continue;
            }
            if let Some(previous_up_completion) = self.last_up_completion_ticks[slot] {
                let gap = target.saturating_sub(previous_up_completion);
                self.release_gap_samples = self.release_gap_samples.saturating_add(1);
                if self.release_gap_samples == 1 {
                    self.min_release_gap_ticks = gap;
                } else {
                    self.min_release_gap_ticks = self.min_release_gap_ticks.min(gap);
                }
                if self.release_gap_policy_ticks > 0 && gap < self.release_gap_policy_ticks {
                    self.release_gap_below_policy_count =
                        self.release_gap_below_policy_count.saturating_add(1);
                }
            }
            if self.anchors[slot].valid {
                self.anchor_overwrite_count = self.anchor_overwrite_count.saturating_add(1);
                self.record_anomaly(5, slot);
            }
            self.anchors[slot] = ProductionHoldAnchor {
                valid: true,
                target_ticks: target,
                pre_call_ticks: pre_call,
                completion_ticks: completion,
            };
        }
        self.publish_metrics(metrics);
    }

    fn record_anomaly(&mut self, kind: u8, slot: usize) {
        if self.anomaly_valid[self.next_anomaly] {
            self.anomaly_ring_overwrites = self.anomaly_ring_overwrites.saturating_add(1);
        }
        self.anomalies[self.next_anomaly] = ProductionForensicsAnomaly {
            kind,
            slot: slot as u8,
            ..ProductionForensicsAnomaly::default()
        };
        self.anomaly_valid[self.next_anomaly] = true;
        self.next_anomaly = (self.next_anomaly + 1) % PRODUCTION_ANOMALY_CAPACITY;
        self.anomaly_count = self.anomaly_count.saturating_add(1);
    }

    fn publish_metrics(&self, metrics: &mut WorkerMetricsLocal) {
        metrics.production_forensics_available = true;
        metrics.production_forensics_version = PRODUCTION_FORENSICS_VERSION;
        metrics.production_hold_pair_samples = self.pair_samples;
        metrics.production_min_pre_call_hold_ticks = self.min_pre_call_hold_ticks;
        metrics.production_min_completion_hold_ticks = self.min_completion_hold_ticks;
        metrics.production_max_pre_call_shrink_ticks = self.max_pre_call_shrink_ticks;
        metrics.production_max_completion_shrink_ticks = self.max_completion_shrink_ticks;
        metrics.production_completion_hold_below_frame_count =
            self.completion_hold_below_frame_count;
        metrics.production_release_gap_samples = self.release_gap_samples;
        metrics.production_min_release_gap_ticks = self.min_release_gap_ticks;
        metrics.production_release_gap_below_policy_count = self.release_gap_below_policy_count;
        metrics.production_same_call_same_key_retrigger_count =
            self.same_call_same_key_retrigger_count;
        metrics.production_anchor_overwrite_count = self.anchor_overwrite_count;
        metrics.production_unmatched_up_count = self.unmatched_up_count;
        metrics.production_anomaly_ring_overwrite_count = self.anomaly_ring_overwrites;
        metrics.production_forensics_anomaly_count = self.anomaly_count;
    }
}
