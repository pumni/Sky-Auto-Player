//! Startup-owned immutable normal-playback dispatch stream.
//!
//! The stream is built from the coordinator's prepared authored contract.  It
//! is deliberately not a second compiler: the coordinator simulator advances
//! with the same frozen commits that the runtime coordinator applies after a
//! successful send.

use super::dispatch::{AuthoredBatchView, PhysicalCommit};
use super::planning::{NextDispatchPlan, PlanningInput, plan_next_dispatch_projected};
use super::{DispatchPreparationProbe, QpcClock};
use sky_dispatch_core::coordinator::{
    CoordinatorError, PreparedAuthoredCommit, RuntimeDispatchCoordinator,
};
use sky_dispatch_core::model::{GenerationId, MAX_KEYS};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::input::MaterializedInstrumentKeyProfile;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreparedDownHoldLimit {
    /// An unpaired Down has no invented finite sender cutoff.  It still
    /// requires the exact future authorization proof before its target.
    NoPairedRelease,
    /// Static authored hold-validity slack, frozen during preparation.  This
    /// is not a dynamic PhysicalTimingWindow or scheduler-lateness allowance.
    HoldSlack(DurationTicks),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreparedDownPolicy {
    pub(crate) hold_limit: PreparedDownHoldLimit,
}

#[derive(Debug)]
pub(crate) struct PreparedDispatchFrame {
    pub(crate) offset_ticks: TimelineTicks,
    pub(crate) view: AuthoredBatchView,
    pub(crate) down_policy: Option<PreparedDownPolicy>,
}

#[derive(Debug)]
pub(crate) enum PreparedDispatchEntry {
    Physical(Box<PreparedDispatchFrame>),
    Metadata {
        offset_ticks: TimelineTicks,
        commit: Box<PreparedAuthoredCommit>,
    },
}

#[derive(Debug)]
pub(crate) struct PreparedDispatchStream {
    entries: Box<[PreparedDispatchEntry]>,
    cursor: usize,
    cancelled_generation_ids: Box<[GenerationId]>,
    cancelled_generation_count: usize,
}

impl PreparedDispatchStream {
    pub(crate) fn build(
        mut simulator: RuntimeDispatchCoordinator,
        qpc_clock: QpcClock,
        preparation_probe: &DispatchPreparationProbe,
        instrument_key_profile: &MaterializedInstrumentKeyProfile,
    ) -> Result<(Self, RuntimeDispatchCoordinator), String> {
        let mut entries = Vec::new();
        let mut open_by_slot: [Option<OpenPreparedDown>; MAX_KEYS] = [None; MAX_KEYS];
        let min_hold_ticks = simulator.min_hold_ticks;

        while simulator.cursor < simulator.schedule.batches.len() {
            let mut plan = NextDispatchPlan::default();
            plan_next_dispatch_projected(
                PlanningInput {
                    coordinator: &simulator,
                    epoch_qpc: QpcTicks::ZERO,
                    preparation_probe,
                    instrument_key_profile,
                },
                &mut plan,
            )
            .map_err(|error| format!("normal prepared stream planning failed: {error}"))?;

            match plan {
                NextDispatchPlan::NoWork => {
                    return Err(
                        "normal prepared stream encountered authored work without a prepared entry"
                            .to_string(),
                    );
                }
                NextDispatchPlan::Metadata(metadata) => {
                    let offset_ticks = metadata.deadline_ticks;
                    record_prepared_commit_intents(
                        &mut entries,
                        &metadata.commit,
                        None,
                        offset_ticks,
                        min_hold_ticks,
                        &mut open_by_slot,
                    )?;
                    simulator
                        .commit_prepared_authored_frame_metadata_frozen(&metadata.commit)
                        .map_err(|error| {
                            format!("normal prepared metadata simulation failed: {error}")
                        })?;
                    entries.push(PreparedDispatchEntry::Metadata {
                        offset_ticks,
                        commit: Box::new(metadata.commit),
                    });
                }
                NextDispatchPlan::Physical(physical) => {
                    let view = physical.authored_view;
                    let commit = match &view.commit {
                        PhysicalCommit::Authored(commit) => commit.clone(),
                        PhysicalCommit::PendingRelease { .. }
                        | PhysicalCommit::Coalesced { .. } => {
                            return Err(format!(
                                "normal prepared stream requires authored-only physical frames at action {}; deferred release was prepared",
                                view.batch_source_action_index
                            ));
                        }
                    };
                    if commit.frame.deferred_up_mask != 0 {
                        return Err(format!(
                            "normal prepared stream requires deferred_up_mask == 0 at action {}",
                            view.batch_source_action_index
                        ));
                    }
                    if view.packet_masks.up_mask & view.packet_masks.down_mask != 0 {
                        return Err(format!(
                            "normal prepared stream received overlapping masks at action {}",
                            view.batch_source_action_index
                        ));
                    }
                    let offset_ticks = view.prepared_batch.effective_scheduled_ticks;
                    let entry_index = entries.len();
                    let down_policy = (commit.frame.down_mask != 0).then_some(PreparedDownPolicy {
                        hold_limit: PreparedDownHoldLimit::NoPairedRelease,
                    });
                    entries.push(PreparedDispatchEntry::Physical(Box::new(
                        PreparedDispatchFrame {
                            offset_ticks,
                            view,
                            down_policy,
                        },
                    )));
                    record_prepared_commit_intents(
                        &mut entries,
                        &commit,
                        Some(entry_index),
                        offset_ticks,
                        min_hold_ticks,
                        &mut open_by_slot,
                    )?;
                    simulator
                        .commit_prepared_authored_frame_success_frozen(
                            &commit,
                            offset_ticks,
                            offset_ticks,
                        )
                        .map_err(|error| {
                            format!("normal prepared physical simulation failed: {error}")
                        })?;
                }
            }
        }

        if simulator.earliest_pending_release_ticks().is_some() {
            return Err(
                "normal prepared stream left a deferred authored release after preparation"
                    .to_string(),
            );
        }

        let schedule = simulator.schedule;
        let cancellation_capacity = usize::try_from(schedule.generation_count)
            .map_err(|_| "normal prepared stream generation ledger is too large".to_string())?;
        let cancelled_generation_ids = vec![0; cancellation_capacity].into_boxed_slice();
        let min_hold_us = simulator.min_hold_us;
        let min_hold_ticks = simulator.min_hold_ticks;
        let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            min_hold_us,
            min_hold_ticks,
            |microseconds| {
                qpc_clock
                    .timeline_from_us(microseconds)
                    .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
            },
        )
        .map_err(|error| format!("normal prepared stream coordinator reset failed: {error}"))?;
        Ok((
            Self {
                entries: entries.into_boxed_slice(),
                cursor: 0,
                cancelled_generation_ids,
                cancelled_generation_count: 0,
            },
            coordinator,
        ))
    }

    #[inline]
    pub(crate) fn current(&self) -> Option<&PreparedDispatchEntry> {
        self.entries.get(self.cursor)
    }

    #[inline]
    pub(crate) fn advance(&mut self) -> Result<(), &'static str> {
        self.cursor = self
            .cursor
            .checked_add(1)
            .ok_or("prepared dispatch stream cursor overflow")?;
        Ok(())
    }

    #[inline]
    pub(crate) fn is_exhausted(&self) -> bool {
        self.cursor >= self.entries.len()
    }

    /// Record only generation IDs returned by the shared verified suspension
    /// path.  The bounded ledger is allocated during stream preparation, so a
    /// later focus/manual/system suspension does not allocate on the worker's
    /// dispatch path.
    pub(crate) fn reconcile_resumable_suspension(
        &mut self,
        cancelled_generation_ids: &[GenerationId],
    ) -> Result<(), String> {
        for &generation_id in cancelled_generation_ids {
            if self.cancelled_generation_ids[..self.cancelled_generation_count]
                .contains(&generation_id)
            {
                continue;
            }
            let Some(slot) = self
                .cancelled_generation_ids
                .get_mut(self.cancelled_generation_count)
            else {
                return Err(
                    "normal prepared stream exceeded its cancellation ledger capacity".to_string(),
                );
            };
            *slot = generation_id;
            self.cancelled_generation_count += 1;
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn explicitly_cancelled_generation_ids(&self) -> &[GenerationId] {
        &self.cancelled_generation_ids[..self.cancelled_generation_count]
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn physical_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| matches!(entry, PreparedDispatchEntry::Physical(_)))
            .count()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn entries(&self) -> &[PreparedDispatchEntry] {
        &self.entries
    }
}

#[derive(Clone, Copy, Debug)]
struct OpenPreparedDown {
    generation_id: GenerationId,
    entry_index: usize,
    down_offset_ticks: TimelineTicks,
}

fn record_prepared_commit_intents(
    entries: &mut [PreparedDispatchEntry],
    commit: &PreparedAuthoredCommit,
    entry_index: Option<usize>,
    offset_ticks: TimelineTicks,
    min_hold_ticks: DurationTicks,
    open_by_slot: &mut [Option<OpenPreparedDown>; MAX_KEYS],
) -> Result<(), String> {
    for up_intent in &commit.up_intents {
        let slot = usize::from(up_intent.intent.key_slot());
        let Some(open) = open_by_slot.get(slot).copied().flatten() else {
            // Unmatched/stale Up metadata retains its existing behavior and
            // does not manufacture a pairing.
            continue;
        };
        if open.generation_id != up_intent.intent.generation_id() {
            return Err(format!(
                "prepared hold pairing generation mismatch at key slot {slot}: open {}, Up {}",
                open.generation_id,
                up_intent.intent.generation_id()
            ));
        }
        let hold_interval = offset_ticks
            .checked_duration_since(open.down_offset_ticks)
            .map_err(|error| format!("prepared hold interval arithmetic failed: {error}"))?;
        let hold_slack = hold_interval
            .checked_sub(min_hold_ticks)
            .map_err(|error| format!("prepared hold slack underflow: {error}"))?;
        let Some(PreparedDispatchEntry::Physical(frame)) = entries.get_mut(open.entry_index) else {
            return Err(format!(
                "prepared hold pairing entry {} is not physical",
                open.entry_index
            ));
        };
        let Some(policy) = frame.down_policy.as_mut() else {
            return Err(format!(
                "prepared hold pairing entry {} has no Down policy",
                open.entry_index
            ));
        };
        policy.hold_limit = match policy.hold_limit {
            PreparedDownHoldLimit::NoPairedRelease => PreparedDownHoldLimit::HoldSlack(hold_slack),
            PreparedDownHoldLimit::HoldSlack(existing) => {
                PreparedDownHoldLimit::HoldSlack(existing.min(hold_slack))
            }
        };
        open_by_slot[slot] = None;
    }

    for down_intent in &commit.down_intents {
        let slot = usize::from(down_intent.intent.key_slot());
        if open_by_slot.get(slot).is_some_and(Option::is_some) {
            return Err(format!(
                "prepared Down slot {slot} already has an open generation"
            ));
        }
        let Some(entry_index) = entry_index else {
            return Err("prepared Down metadata has no physical entry".to_string());
        };
        let Some(slot_state) = open_by_slot.get_mut(slot) else {
            return Err(format!("prepared Down key slot {slot} is outside MAX_KEYS"));
        };
        *slot_state = Some(OpenPreparedDown {
            generation_id: down_intent.intent.generation_id(),
            entry_index,
            down_offset_ticks: offset_ticks,
        });
    }

    Ok(())
}
