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
use sky_dispatch_core::model::GenerationId;
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::input::MaterializedInstrumentKeyProfile;

#[derive(Debug)]
pub(crate) struct PreparedDispatchFrame {
    pub(crate) offset_ticks: TimelineTicks,
    pub(crate) view: AuthoredBatchView,
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
                    simulator
                        .commit_prepared_authored_frame_success_frozen(
                            &commit,
                            offset_ticks,
                            offset_ticks,
                        )
                        .map_err(|error| {
                            format!("normal prepared physical simulation failed: {error}")
                        })?;
                    entries.push(PreparedDispatchEntry::Physical(Box::new(
                        PreparedDispatchFrame { offset_ticks, view },
                    )));
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
