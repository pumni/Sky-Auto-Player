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
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::input::MaterializedInstrumentKeyProfile;

#[derive(Debug)]
pub(crate) struct PreparedDispatchFrame {
    pub(crate) offset_ticks: TimelineTicks,
    pub(crate) view: AuthoredBatchView,
}

#[derive(Debug)]
// Inline entries keep the pre-reserved stream contiguous and avoid one heap
// allocation per authored boundary during startup preparation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum PreparedDispatchEntry {
    Physical(PreparedDispatchFrame),
    Metadata {
        offset_ticks: TimelineTicks,
        commit: PreparedAuthoredCommit,
    },
}

#[derive(Debug)]
pub(crate) struct PreparedDispatchStream {
    entries: Vec<PreparedDispatchEntry>,
    cursor: usize,
}

impl PreparedDispatchStream {
    /// Bound the additional worker-owned storage used while constructing the
    /// immutable dispatch stream and its reset coordinator.  The estimate is
    /// conservative: it reserves one maximum-sized stream entry per authored
    /// batch and accounts for both coordinator ledgers while the reset is
    /// built alongside the simulator.
    pub(crate) const MAX_PREPARED_STREAM_BYTES: usize = 512 * 1024 * 1024;

    pub(crate) fn build(
        simulator: RuntimeDispatchCoordinator,
        qpc_clock: QpcClock,
        preparation_probe: &DispatchPreparationProbe,
        instrument_key_profile: &MaterializedInstrumentKeyProfile,
    ) -> Result<(Self, RuntimeDispatchCoordinator), String> {
        Self::build_with_checkpoints(
            simulator,
            qpc_clock,
            preparation_probe,
            instrument_key_profile,
            || Ok(()),
            || Ok(()),
        )
    }

    pub(crate) fn build_with_checkpoints<C, R>(
        mut simulator: RuntimeDispatchCoordinator,
        qpc_clock: QpcClock,
        preparation_probe: &DispatchPreparationProbe,
        instrument_key_profile: &MaterializedInstrumentKeyProfile,
        mut stream_checkpoint: C,
        mut coordinator_checkpoint: R,
    ) -> Result<(Self, RuntimeDispatchCoordinator), String>
    where
        C: FnMut() -> Result<(), String>,
        R: FnMut() -> Result<(), String>,
    {
        let entry_capacity = simulator.schedule.batches.len();
        let generation_capacity = usize::try_from(simulator.schedule.generation_count)
            .map_err(|_| "normal prepared stream generation ledger is too large".to_string())?;
        let coordinator_bytes = entry_capacity
            .checked_mul(std::mem::size_of::<TimelineTicks>())
            .and_then(|bytes| {
                generation_capacity
                    .checked_mul(
                        std::mem::size_of::<sky_dispatch_core::coordinator::GenerationStatus>()
                            + std::mem::size_of::<Option<(usize, usize)>>(),
                    )
                    .and_then(|ledger| bytes.checked_add(ledger))
            })
            .ok_or_else(|| "normal prepared stream memory estimate overflowed".to_string())?;
        let estimated_bytes = entry_capacity
            .checked_mul(std::mem::size_of::<PreparedDispatchEntry>())
            .and_then(|bytes| {
                coordinator_bytes
                    .checked_mul(2)
                    .and_then(|both| bytes.checked_add(both))
            })
            .ok_or_else(|| "normal prepared stream memory estimate overflowed".to_string())?;
        if estimated_bytes > Self::MAX_PREPARED_STREAM_BYTES {
            return Err(format!(
                "normal prepared stream requires an estimated {estimated_bytes} bytes, exceeding the {} byte admission bound",
                Self::MAX_PREPARED_STREAM_BYTES
            ));
        }

        let mut entries = Vec::new();
        entries
            .try_reserve_exact(entry_capacity)
            .map_err(|error| format!("normal prepared stream allocation failed: {error}"))?;

        let mut visited_batches = 0usize;
        while simulator.cursor < simulator.schedule.batches.len() {
            if visited_batches.is_multiple_of(64) {
                stream_checkpoint()?;
            }
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
                        commit: metadata.commit,
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
                    entries.push(PreparedDispatchEntry::Physical(PreparedDispatchFrame {
                        offset_ticks,
                        view,
                    }));
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
            visited_batches = visited_batches.saturating_add(1);
        }

        if simulator.earliest_pending_release_ticks().is_some() {
            return Err(
                "normal prepared stream left a deferred authored release after preparation"
                    .to_string(),
            );
        }

        coordinator_checkpoint()?;
        let schedule = simulator.schedule;
        let min_hold_us = simulator.min_hold_us;
        let min_hold_ticks = simulator.min_hold_ticks;
        let coordinator = RuntimeDispatchCoordinator::try_new_ticks_with_checkpoint(
            schedule,
            min_hold_us,
            min_hold_ticks,
            |microseconds| {
                qpc_clock
                    .timeline_from_us(microseconds)
                    .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
            },
            || coordinator_checkpoint().map_err(|_| CoordinatorError::PreparationCancelled),
        )
        .map_err(|error| format!("normal prepared stream coordinator reset failed: {error}"))?;
        Ok((Self { entries, cursor: 0 }, coordinator))
    }

    #[inline]
    pub(crate) fn current(&self) -> Option<&PreparedDispatchEntry> {
        self.entries.get(self.cursor)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn cursor_for_test(&self) -> usize {
        self.cursor
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

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn prepared_storage_evidence_for_test(&self) -> (usize, usize, usize, usize) {
        let mut physical_count = 0usize;
        let mut metadata_count = 0usize;
        for entry in self.entries.iter() {
            match entry {
                PreparedDispatchEntry::Physical(_) => physical_count += 1,
                PreparedDispatchEntry::Metadata { .. } => metadata_count += 1,
            }
        }
        (
            self.entries.len(),
            physical_count,
            metadata_count,
            self.entries.len() * std::mem::size_of::<PreparedDispatchEntry>(),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn physical_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| matches!(entry, PreparedDispatchEntry::Physical(_)))
            .count()
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn cancellation_ledger_bytes_for_test(&self) -> usize {
        // Resumable cancellation provenance is owned by the coordinator's
        // fixed fifteen-slot token table, never by a song-sized stream list.
        0
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn entries(&self) -> &[PreparedDispatchEntry] {
        &self.entries
    }
}
