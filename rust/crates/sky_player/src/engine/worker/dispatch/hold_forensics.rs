use super::super::super::DurationTicks;
use super::super::WorkerMetricsLocal;
use super::observation::ObserverLifecycle;
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::input::PhysicalPacket;

const MAX_KEYS: usize = sky_dispatch_core::model::MAX_KEYS;

/// Fixed-size sender evidence owned by the worker itself. It deliberately
/// consumes the timestamps already returned by the trusted sender: it does
/// not sample QPC, allocate, lock, format, or walk the schedule.
pub(crate) const PRODUCTION_FORENSICS_VERSION: u32 = 4;
const PRODUCTION_ANOMALY_CAPACITY: usize = 32;

#[derive(Clone, Copy, Default)]
struct ProductionHoldAnchor {
    valid: bool,
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
    min_hold_start_after_down_completion_ticks: u64,
    hold_floor_violation_count: u64,
    release_floor_samples: u64,
    min_down_start_after_up_completion_ticks: u64,
    release_floor_violation_count: u64,
    /// Corruption/forensics evidence only: production packet preparation is
    /// required to reject overlapping directions before realtime execution.
    same_key_overlap_forensics_count: u64,
    anchor_overwrite_count: u64,
    unmatched_up_count: u64,
    structural_anomaly_count: u64,
    timing_diagnostic_count: u64,
    effective_min_hold_ticks: u64,
    frame_ticks: u64,
}

impl ProductionHoldForensics {
    pub(crate) fn set_frame_policies(
        &mut self,
        effective_min_hold_ticks: DurationTicks,
        frame_ticks: DurationTicks,
    ) {
        self.effective_min_hold_ticks = effective_min_hold_ticks.as_u64();
        self.frame_ticks = frame_ticks.as_u64();
    }

    pub(crate) fn observe_lifecycle(&mut self, lifecycle: ObserverLifecycle) {
        match lifecycle {
            ObserverLifecycle::ResetAll => {
                self.anchors.fill(ProductionHoldAnchor::default());
                self.last_up_completion_ticks.fill(None);
            }
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
        self.observe_ups(
            packet.up_mask,
            packet.down_mask,
            source_action_index,
            target,
            pre_call,
            completion,
        );
        self.observe_downs(
            packet.down_mask,
            source_action_index,
            target,
            pre_call,
            completion,
        );
        self.publish_metrics(metrics);
    }

    /// Record a fully confirmed musical Up-prefix recovery using the sender's
    /// existing start/completion timestamps. Failed or uncertain transport
    /// must never create an optimistic physical anchor.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_recovery_up(
        &mut self,
        up_mask: u16,
        source_action_index: u32,
        target_qpc: QpcTicks,
        started_qpc: QpcTicks,
        completed_qpc: QpcTicks,
        full_transport_success: bool,
        metrics: &mut WorkerMetricsLocal,
    ) {
        if !full_transport_success {
            return;
        }
        self.observe_ups(
            up_mask,
            0,
            source_action_index,
            target_qpc.as_u64(),
            started_qpc.as_u64(),
            completed_qpc.as_u64(),
        );
        self.publish_metrics(metrics);
    }

    fn observe_ups(
        &mut self,
        mut up_mask: u16,
        same_call_down_mask: u16,
        source_action_index: u32,
        target: u64,
        up_start: u64,
        completion: u64,
    ) {
        while up_mask != 0 {
            let slot = up_mask.trailing_zeros() as usize;
            let bit = 1u16 << slot;
            up_mask &= up_mask - 1;
            if self.anchors[slot].valid {
                let anchor = self.anchors[slot];
                if same_call_down_mask & bit != 0 {
                    self.same_key_overlap_forensics_count =
                        self.same_key_overlap_forensics_count.saturating_add(1);
                    self.record_anomaly(
                        6,
                        slot,
                        source_action_index,
                        bit,
                        target,
                        up_start,
                        anchor.completion_ticks,
                        completion.abs_diff(anchor.completion_ticks),
                    );
                }
                let hold_start = up_start.checked_sub(anchor.completion_ticks);
                self.pair_samples = self.pair_samples.saturating_add(1);
                let hold_start_ticks = hold_start.unwrap_or_default();
                self.min_hold_start_after_down_completion_ticks = if self.pair_samples == 1 {
                    hold_start_ticks
                } else {
                    self.min_hold_start_after_down_completion_ticks
                        .min(hold_start_ticks)
                };
                if hold_start.is_none() || hold_start_ticks < self.effective_min_hold_ticks {
                    self.hold_floor_violation_count =
                        self.hold_floor_violation_count.saturating_add(1);
                    self.record_anomaly(
                        9,
                        slot,
                        source_action_index,
                        bit,
                        target,
                        up_start,
                        anchor.completion_ticks,
                        hold_start_ticks,
                    );
                }
                self.last_up_completion_ticks[slot] = Some(completion);
            } else {
                self.unmatched_up_count = self.unmatched_up_count.saturating_add(1);
                self.record_anomaly(4, slot, source_action_index, bit, target, completion, 0, 0);
                // Even without a paired Down sample, a successful Up is the
                // physical release anchor for the next same-key Down.
                self.last_up_completion_ticks[slot] = Some(completion);
            }
            self.anchors[slot].valid = false;
        }
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
                        self.release_floor_samples = self.release_floor_samples.saturating_add(1);
                        self.min_down_start_after_up_completion_ticks = 0;
                        self.release_floor_violation_count =
                            self.release_floor_violation_count.saturating_add(1);
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
                        self.release_floor_samples = self.release_floor_samples.saturating_add(1);
                        self.min_down_start_after_up_completion_ticks =
                            if self.release_floor_samples == 1 {
                                gap
                            } else {
                                self.min_down_start_after_up_completion_ticks.min(gap)
                            };
                        if gap < self.frame_ticks {
                            self.release_floor_violation_count =
                                self.release_floor_violation_count.saturating_add(1);
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
                    previous_anchor.completion_ticks,
                    completion.abs_diff(previous_anchor.completion_ticks),
                );
            }
            self.anchors[slot] = ProductionHoldAnchor {
                valid: true,
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
        metrics.production_min_hold_start_after_down_completion_ticks =
            self.min_hold_start_after_down_completion_ticks;
        metrics.production_hold_floor_violation_count = self.hold_floor_violation_count;
        metrics.production_release_floor_samples = self.release_floor_samples;
        metrics.production_min_down_start_after_up_completion_ticks =
            self.min_down_start_after_up_completion_ticks;
        metrics.production_release_floor_violation_count = self.release_floor_violation_count;
        metrics.production_hold_floor_ticks = self.effective_min_hold_ticks;
        metrics.production_release_floor_ticks = self.frame_ticks;
        metrics.production_same_key_overlap_forensics_count = self.same_key_overlap_forensics_count;
        metrics.production_anchor_overwrite_count = self.anchor_overwrite_count;
        metrics.production_unmatched_up_count = self.unmatched_up_count;
        metrics.production_anomaly_ring_overwrite_count = self.anomaly_ring_overwrites;
        metrics.production_forensics_anomaly_count = self.anomaly_count;
        metrics.production_structural_anomaly_count = self.structural_anomaly_count;
        metrics.production_timing_diagnostic_count = self.timing_diagnostic_count;
    }
}
