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
pub(crate) const PRODUCTION_FORENSICS_VERSION: u32 = 2;
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
pub(crate) struct ProductionForensicsAnomaly {
    pub(crate) kind: u8,
    pub(crate) slot: u8,
    pub(crate) source_action_index: u32,
    pub(crate) mask: u16,
    pub(crate) target_ticks: u64,
    pub(crate) observed_ticks: u64,
    pub(crate) aux_ticks: u64,
    pub(crate) delta_ticks: u64,
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
    // Observed completion-to-next-pre-call intervals are hard only against
    // the base frame visibility floor. Authored sender headroom is allowed to
    // be consumed by transport and is reported separately below.
    release_gap_below_policy_count: u64,
    release_headroom_consumed_count: u64,
    max_release_headroom_consumed_ticks: u64,
    same_call_same_key_retrigger_count: u64,
    anchor_overwrite_count: u64,
    unmatched_up_count: u64,
    structural_anomaly_count: u64,
    timing_diagnostic_count: u64,
    frame_policy_ticks: u64,
    authored_release_gap_ticks: u64,
}

impl ProductionHoldForensics {
    pub(crate) fn set_frame_policies(
        &mut self,
        frame_policy_ticks: DurationTicks,
        release_gap_policy_ticks: DurationTicks,
    ) {
        self.frame_policy_ticks = frame_policy_ticks.as_u64();
        self.authored_release_gap_ticks = release_gap_policy_ticks.as_u64();
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
        let mut touched = mask;
        while touched != 0 {
            let slot = touched.trailing_zeros() as usize;
            touched &= touched - 1;
            self.anchors[slot] = ProductionHoldAnchor::default();
            self.last_up_completion_ticks[slot] = None;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_packet_result(
        &mut self,
        packet: PhysicalPacket,
        source_action_index: u32,
        target_qpc: QpcTicks,
        pre_call_qpc: QpcTicks,
        completion_qpc: QpcTicks,
        status: sky_dispatch_win32::input::SendTransactionStatus,
        metrics: &mut WorkerMetricsLocal,
    ) {
        self.observe_packet(
            packet,
            source_action_index,
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_packet(
        &mut self,
        packet: PhysicalPacket,
        source_action_index: u32,
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
        let mut up_mask = packet.up_mask;
        while up_mask != 0 {
            let slot = up_mask.trailing_zeros() as usize;
            let bit = 1u16 << slot;
            up_mask &= up_mask - 1;
            if self.anchors[slot].valid {
                let anchor = self.anchors[slot];
                if packet.down_mask & bit != 0 {
                    self.same_call_same_key_retrigger_count =
                        self.same_call_same_key_retrigger_count.saturating_add(1);
                    self.record_anomaly(
                        6,
                        slot,
                        source_action_index,
                        bit,
                        target,
                        pre_call,
                        anchor.target_ticks,
                        target.abs_diff(anchor.target_ticks),
                    );
                }
                let Some(authored_hold) = target.checked_sub(anchor.target_ticks) else {
                    self.record_anomaly(
                        1,
                        slot,
                        source_action_index,
                        bit,
                        target,
                        target,
                        anchor.target_ticks,
                        target.abs_diff(anchor.target_ticks),
                    );
                    self.anchors[slot].valid = false;
                    continue;
                };
                let Some(pre_call_hold) = pre_call.checked_sub(anchor.pre_call_ticks) else {
                    self.record_anomaly(
                        2,
                        slot,
                        source_action_index,
                        bit,
                        target,
                        pre_call,
                        anchor.pre_call_ticks,
                        pre_call.abs_diff(anchor.pre_call_ticks),
                    );
                    self.anchors[slot].valid = false;
                    continue;
                };
                let Some(completion_hold) = completion.checked_sub(anchor.completion_ticks) else {
                    self.record_anomaly(
                        3,
                        slot,
                        source_action_index,
                        bit,
                        target,
                        completion,
                        anchor.completion_ticks,
                        completion.abs_diff(anchor.completion_ticks),
                    );
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
                self.record_anomaly(4, slot, source_action_index, bit, target, completion, 0, 0);
            }
            self.anchors[slot].valid = false;
        }
        self.observe_downs(
            packet.down_mask,
            source_action_index,
            target,
            pre_call,
            completion,
        );
        self.publish_metrics(metrics);
    }

    fn observe_downs(
        &mut self,
        mut down_mask: u16,
        source_action_index: u32,
        target: u64,
        pre_call: u64,
        completion: u64,
    ) {
        while down_mask != 0 {
            let slot = down_mask.trailing_zeros() as usize;
            let bit = 1u16 << slot;
            down_mask &= down_mask - 1;
            if let Some(previous_up_completion) = self.last_up_completion_ticks[slot] {
                match pre_call.checked_sub(previous_up_completion) {
                    None => {
                        // A pre-call sample before the previous Up completion
                        // is an ordering fault. Retain both timestamps.
                        self.release_gap_below_policy_count =
                            self.release_gap_below_policy_count.saturating_add(1);
                        self.record_anomaly(
                            8,
                            slot,
                            source_action_index,
                            bit,
                            target,
                            pre_call,
                            previous_up_completion,
                            previous_up_completion.abs_diff(pre_call),
                        );
                    }
                    Some(gap) => {
                        self.release_gap_samples = self.release_gap_samples.saturating_add(1);
                        self.min_release_gap_ticks = if self.release_gap_samples == 1 {
                            gap
                        } else {
                            self.min_release_gap_ticks.min(gap)
                        };
                        let headroom_consumed = self.authored_release_gap_ticks.saturating_sub(gap);
                        if headroom_consumed > 0 {
                            self.release_headroom_consumed_count =
                                self.release_headroom_consumed_count.saturating_add(1);
                            self.max_release_headroom_consumed_ticks = self
                                .max_release_headroom_consumed_ticks
                                .max(headroom_consumed);
                        }
                        if self.frame_policy_ticks > 0 && gap < self.frame_policy_ticks {
                            self.release_gap_below_policy_count =
                                self.release_gap_below_policy_count.saturating_add(1);
                            self.record_anomaly(
                                7,
                                slot,
                                source_action_index,
                                bit,
                                target,
                                pre_call,
                                previous_up_completion,
                                gap,
                            );
                        }
                    }
                }
            }
            if self.anchors[slot].valid {
                let previous_anchor = self.anchors[slot];
                self.anchor_overwrite_count = self.anchor_overwrite_count.saturating_add(1);
                self.record_anomaly(
                    5,
                    slot,
                    source_action_index,
                    bit,
                    target,
                    pre_call,
                    previous_anchor.target_ticks,
                    target.abs_diff(previous_anchor.target_ticks),
                );
            }
            self.anchors[slot] = ProductionHoldAnchor {
                valid: true,
                target_ticks: target,
                pre_call_ticks: pre_call,
                completion_ticks: completion,
            };
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_anomaly(
        &mut self,
        kind: u8,
        slot: usize,
        source_action_index: u32,
        mask: u16,
        target_ticks: u64,
        observed_ticks: u64,
        aux_ticks: u64,
        delta_ticks: u64,
    ) {
        if self.anomaly_valid[self.next_anomaly] {
            self.anomaly_ring_overwrites = self.anomaly_ring_overwrites.saturating_add(1);
        }
        self.anomalies[self.next_anomaly] = ProductionForensicsAnomaly {
            kind,
            slot: slot as u8,
            source_action_index,
            mask,
            target_ticks,
            observed_ticks,
            aux_ticks,
            delta_ticks,
        };
        self.anomaly_valid[self.next_anomaly] = true;
        self.next_anomaly = (self.next_anomaly + 1) % PRODUCTION_ANOMALY_CAPACITY;
        self.anomaly_count = self.anomaly_count.saturating_add(1);
        if matches!(kind, 1..=6 | 8) {
            self.structural_anomaly_count = self.structural_anomaly_count.saturating_add(1);
        } else {
            self.timing_diagnostic_count = self.timing_diagnostic_count.saturating_add(1);
        }
    }

    #[allow(dead_code)]
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn latest_anomaly_for_test(&self) -> Option<ProductionForensicsAnomaly> {
        if self.anomaly_count == 0 {
            return None;
        }
        let index = if self.next_anomaly == 0 {
            PRODUCTION_ANOMALY_CAPACITY - 1
        } else {
            self.next_anomaly - 1
        };
        self.anomaly_valid[index].then_some(self.anomalies[index])
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
        metrics.production_release_visibility_floor_ticks = self.frame_policy_ticks;
        metrics.production_release_headroom_consumed_count = self.release_headroom_consumed_count;
        metrics.production_max_release_headroom_consumed_ticks =
            self.max_release_headroom_consumed_ticks;
        metrics.production_same_call_same_key_retrigger_count =
            self.same_call_same_key_retrigger_count;
        metrics.production_anchor_overwrite_count = self.anchor_overwrite_count;
        metrics.production_unmatched_up_count = self.unmatched_up_count;
        metrics.production_anomaly_ring_overwrite_count = self.anomaly_ring_overwrites;
        metrics.production_forensics_anomaly_count = self.anomaly_count;
        metrics.production_structural_anomaly_count = self.structural_anomaly_count;
        metrics.production_timing_diagnostic_count = self.timing_diagnostic_count;
    }
}
