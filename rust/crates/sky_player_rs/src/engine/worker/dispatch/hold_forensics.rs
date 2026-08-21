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
