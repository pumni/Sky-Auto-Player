//! Offline-only direct preparation into compact entries and a frozen INPUT
//! arena. Planning still materializes one temporary inline Win32 packet, but
//! no full-stream inline entry Vec is allocated. No sender uses these entries.

use super::compact_replacement_study::{CompactAuthoredView, CompactPreparedEntry};
use super::{PreparedDispatchEntry, PreparedDispatchStream};
use crate::engine::worker::dispatch::PhysicalCommit;
use crate::engine::worker::planning::{
    NextDispatchPlan, PlanningInput, plan_next_dispatch_projected,
};
use crate::engine::worker::{DispatchPreparationProbe, QpcClock};
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::input::MaterializedInstrumentKeyProfile;
use sky_dispatch_win32::input::compact_study::{CompactInputArenaBuilder, FrozenCompactInputArena};
use std::mem::size_of;

#[allow(dead_code)]
pub(crate) struct CompactDirectPreparedStream {
    entries: Vec<CompactPreparedEntry>,
    arena: FrozenCompactInputArena,
    physical_count: usize,
    metadata_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompactDirectSizes {
    pub entry_count: usize,
    pub entry_size_bytes: usize,
    pub entry_capacity: usize,
    pub entry_reserved_bytes: usize,
    pub descriptor_reserved_bytes: usize,
    pub input_reserved_bytes: usize,
    pub input_capacity_events: usize,
    pub input_size_bytes: usize,
    pub total_reserved_bytes: usize,
    pub physical_packet_count: usize,
    pub metadata_entry_count: usize,
    pub input_event_count: usize,
    pub input_event_histogram: [usize; sky_dispatch_win32::input::MAX_PACKET_EVENTS + 1],
}

impl CompactDirectPreparedStream {
    pub(crate) fn sizes(&self) -> Result<CompactDirectSizes, String> {
        let arena = self.arena.sizes();
        let entry_reserved_bytes = self
            .entries
            .capacity()
            .checked_mul(size_of::<CompactPreparedEntry>())
            .ok_or_else(|| "compact direct entry reserved bytes overflow".to_string())?;
        let total_reserved_bytes = entry_reserved_bytes
            .checked_add(arena.total_arena_reserved_bytes)
            .ok_or_else(|| "compact direct total reserved bytes overflow".to_string())?;
        let mut histogram = [0usize; sky_dispatch_win32::input::MAX_PACKET_EVENTS + 1];
        for index in 0..self.physical_count {
            let count = self
                .arena
                .packet_inputs(index)
                .ok_or_else(|| "compact direct missing frozen INPUT span".to_string())?
                .len();
            if count == 0 || count >= histogram.len() {
                return Err("compact direct unexpected packet width".into());
            }
            histogram[count] += 1;
        }
        Ok(CompactDirectSizes {
            entry_count: self.entries.len(),
            entry_size_bytes: size_of::<CompactPreparedEntry>(),
            entry_capacity: self.entries.capacity(),
            entry_reserved_bytes,
            descriptor_reserved_bytes: arena.descriptor_reserved_bytes,
            input_reserved_bytes: arena.input_reserved_bytes,
            input_capacity_events: arena.input_reserved_bytes / arena.input_size,
            input_size_bytes: arena.input_size,
            total_reserved_bytes,
            physical_packet_count: self.physical_count,
            metadata_entry_count: self.metadata_count,
            input_event_count: arena.event_count,
            input_event_histogram: histogram,
        })
    }

    #[cfg(test)]
    fn packet_summaries_for_test(
        &self,
    ) -> Vec<(sky_dispatch_win32::input::PhysicalPacket, usize, usize)> {
        (0..self.physical_count)
            .map(|index| {
                let mask = self.arena.packet_identity(index).unwrap();
                let width = self.arena.packet_inputs(index).unwrap().len();
                let up_prefix = self
                    .arena
                    .mixed_up_prefix(index)
                    .map_or(0, |(_, inputs)| inputs.len());
                (mask, width, up_prefix)
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompactInputReserveHint {
    // Preserve the previously measured #638 baseline exactly.
    TwoPerAuthoredBatch,
    // Offline experiment: use canonical compiled packet masks only as
    // a storage hint. The planner remains authoritative for real INPUTs.
    CompiledPhysicalEvents,
}

impl PreparedDispatchStream {
    /// Builds only compact *stored* entries. Each NextDispatchPlan temporarily
    /// contains its existing 608B PreparedPhysicalPacket on the stack;
    /// appended INPUTs are copied to the owned arena before that plan is
    /// dropped. No alternate live dispatcher or SendInput path exists.
    pub(crate) fn build_compact_direct_for_test(
        simulator: RuntimeDispatchCoordinator,
        qpc_clock: QpcClock,
        preparation_probe: &DispatchPreparationProbe,
        instrument_key_profile: &MaterializedInstrumentKeyProfile,
    ) -> Result<(CompactDirectPreparedStream, RuntimeDispatchCoordinator), String> {
        Self::build_compact_direct_with_checkpoints_for_test(
            simulator,
            qpc_clock,
            preparation_probe,
            instrument_key_profile,
            CompactInputReserveHint::TwoPerAuthoredBatch,
            || Ok(()),
            || Ok(()),
        )
    }

    /// Offline-only alternative, never called from the realtime sender.
    pub(crate) fn build_compact_direct_mask_reserved_for_test(
        simulator: RuntimeDispatchCoordinator,
        qpc_clock: QpcClock,
        preparation_probe: &DispatchPreparationProbe,
        instrument_key_profile: &MaterializedInstrumentKeyProfile,
    ) -> Result<(CompactDirectPreparedStream, RuntimeDispatchCoordinator), String> {
        Self::build_compact_direct_with_checkpoints_for_test(
            simulator,
            qpc_clock,
            preparation_probe,
            instrument_key_profile,
            CompactInputReserveHint::CompiledPhysicalEvents,
            || Ok(()),
            || Ok(()),
        )
    }

    fn build_compact_direct_with_checkpoints_for_test<C, R>(
        mut simulator: RuntimeDispatchCoordinator,
        qpc_clock: QpcClock,
        preparation_probe: &DispatchPreparationProbe,
        instrument_key_profile: &MaterializedInstrumentKeyProfile,
        input_reserve_hint: CompactInputReserveHint,
        mut stream_checkpoint: C,
        mut coordinator_checkpoint: R,
    ) -> Result<(CompactDirectPreparedStream, RuntimeDispatchCoordinator), String>
    where
        C: FnMut() -> Result<(), String>,
        R: FnMut() -> Result<(), String>,
    {
        let entry_capacity = simulator.schedule.batches.len();
        let generation_capacity = usize::try_from(simulator.schedule.generation_count)
            .map_err(|_| "compact direct generation ledger is too large".to_string())?;
        let coordinator_bytes = entry_capacity
            .checked_mul(size_of::<TimelineTicks>())
            .and_then(|bytes| {
                generation_capacity
                    .checked_mul(
                        size_of::<sky_dispatch_core::coordinator::GenerationStatus>()
                            + size_of::<Option<(usize, usize)>>(),
                    )
                    .and_then(|ledger| bytes.checked_add(ledger))
            })
            .ok_or_else(|| "compact direct memory estimate overflowed".to_string())?;
        // Apply the original stream's admission bound, rather than allowing
        // an experimental alternative to silently admit larger schedules.
        let baseline_estimate = entry_capacity
            .checked_mul(size_of::<PreparedDispatchEntry>())
            .and_then(|bytes| {
                coordinator_bytes
                    .checked_mul(2)
                    .and_then(|both| bytes.checked_add(both))
            })
            .ok_or_else(|| "compact direct memory estimate overflowed".to_string())?;
        if baseline_estimate > Self::MAX_PREPARED_STREAM_BYTES {
            return Err("compact direct exceeds production stream admission budget".into());
        }

        let mut entries = Vec::new();
        entries
            .try_reserve_exact(entry_capacity)
            .map_err(|error| format!("reserve compact direct entries: {error}"))?;
        // A hint is never a correctness gate. Compiled masks exclude stale
        // releases; any planner-produced extra INPUTs can still grow the
        // arena before freeze. No alternate sender or SendInput path.
        let input_hint = match input_reserve_hint {
            CompactInputReserveHint::TwoPerAuthoredBatch => entry_capacity.checked_mul(2),
            CompactInputReserveHint::CompiledPhysicalEvents => simulator
                .schedule
                .packets
                .iter()
                .try_fold(0usize, |sum, packet| {
                    sum.checked_add((packet.up_mask | packet.down_mask).count_ones() as usize)
                }),
        }
        .ok_or_else(|| "compact direct INPUT reserve hint overflow".to_string())?;
        let mut builder = CompactInputArenaBuilder::try_with_capacity(entry_capacity, input_hint)?;

        let mut visited_batches = 0usize;
        let mut physical_count = 0usize;
        let mut metadata_count = 0usize;
        let mut total_inputs = 0usize;
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
            .map_err(|error| format!("compact direct planning failed: {error}"))?;
            match plan {
                NextDispatchPlan::NoWork => {
                    return Err("compact direct encountered work without a prepared entry".into());
                }
                NextDispatchPlan::Metadata(metadata) => {
                    let offset_ticks = metadata.deadline_ticks;
                    simulator
                        .commit_prepared_authored_frame_metadata_frozen(&metadata.commit)
                        .map_err(|error| {
                            format!("compact direct metadata simulation failed: {error}")
                        })?;
                    entries.push(CompactPreparedEntry::Metadata {
                        offset_ticks,
                        commit: metadata.commit,
                    });
                    metadata_count += 1;
                }
                NextDispatchPlan::Physical(physical) => {
                    let view = physical.authored_view;
                    let commit = match &view.commit {
                        PhysicalCommit::Authored(commit) => commit.clone(),
                        PhysicalCommit::PendingRelease { .. }
                        | PhysicalCommit::Coalesced { .. } => {
                            return Err(format!(
                                "compact direct requires authored-only physical frames at action {}",
                                view.batch_source_action_index
                            ));
                        }
                    };
                    if commit.frame.deferred_up_mask != 0 {
                        return Err(format!(
                            "compact direct requires deferred_up_mask == 0 at action {}",
                            view.batch_source_action_index
                        ));
                    }
                    if view.packet_masks.up_mask & view.packet_masks.down_mask != 0 {
                        return Err(format!(
                            "compact direct received overlapping masks at action {}",
                            view.batch_source_action_index
                        ));
                    }
                    let offset_ticks = view.prepared_batch.effective_scheduled_ticks;
                    let (compact_view, packet) = CompactAuthoredView::from_view(view);
                    if compact_view.packet_masks() != packet.packet() {
                        return Err("compact direct physical packet identity mismatch".into());
                    }
                    let packet_index = u32::try_from(physical_count)
                        .map_err(|_| "compact direct packet index exceeds u32".to_string())?;
                    total_inputs = total_inputs
                        .checked_add(usize::from(packet.event_count()))
                        .ok_or_else(|| "compact direct INPUT count overflow".to_string())?;
                    builder.append_prepared(&packet)?;
                    entries.push(CompactPreparedEntry::Physical {
                        packet_index,
                        view: compact_view,
                    });
                    physical_count += 1;
                    simulator
                        .commit_prepared_authored_frame_success_frozen(
                            &commit,
                            offset_ticks,
                            offset_ticks,
                        )
                        .map_err(|error| {
                            format!("compact direct physical simulation failed: {error}")
                        })?;
                }
            }
            visited_batches = visited_batches.saturating_add(1);
        }

        if simulator.earliest_pending_release_ticks().is_some() {
            return Err("compact direct left deferred authored release".into());
        }
        coordinator_checkpoint()?;
        let schedule = simulator.schedule;
        let min_hold_us = simulator.min_hold_us;
        let min_hold_ticks = simulator.min_hold_ticks;
        let reset_coordinator = RuntimeDispatchCoordinator::try_new_ticks_with_checkpoint(
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
        .map_err(|error| format!("compact direct coordinator reset failed: {error}"))?;

        let arena = builder.freeze()?;
        let mut count = 0usize;
        for entry in &entries {
            let CompactPreparedEntry::Physical { packet_index, view } = entry else {
                continue;
            };
            if usize::try_from(*packet_index).ok() != Some(count)
                || arena.packet_identity(count) != Some(view.packet_masks())
            {
                return Err("compact direct packet index/order/mask mismatch".into());
            }
            let frozen = arena
                .packet_inputs(count)
                .ok_or_else(|| "compact direct packet span missing".to_string())?;
            if frozen.len() != usize::from(view.packet_masks().event_count()) {
                return Err("compact direct INPUT count mismatch".into());
            }
            count += 1;
        }
        let arena_sizes = arena.sizes();
        if count != physical_count
            || arena_sizes.packet_count != physical_count
            || arena_sizes.event_count != total_inputs
            || physical_count.checked_add(metadata_count) != Some(entries.len())
        {
            return Err("compact direct prepared entry/arena conservation failed".into());
        }
        Ok((
            CompactDirectPreparedStream {
                entries,
                arena,
                physical_count,
                metadata_count,
            },
            reset_coordinator,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::DurationTicks;
    use std::num::NonZeroU64;

    fn action(index: u32, kind: ActionKind, at: u64, scan_codes: &[u16]) -> KeyActionInput {
        KeyActionInput {
            source_action_index: index,
            kind,
            scheduled_us: at,
            scan_codes: scan_codes.iter().copied().collect(),
            reason: format!("compact-direct-{index}").into(),
        }
    }

    fn coordinator(
        actions: &[KeyActionInput],
        canonical: &[u16],
    ) -> (RuntimeDispatchCoordinator, QpcClock) {
        let schedule = compile_runtime_intents(actions, canonical).unwrap();
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            500,
            DurationTicks::from_raw(500),
            |microseconds| {
                clock
                    .timeline_from_us(microseconds)
                    .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
            },
        )
        .unwrap();
        (coordinator, clock)
    }

    fn compare_direct_against_independent_inline(
        actions: &[KeyActionInput],
        canonical: &[u16],
    ) -> (CompactDirectSizes, usize) {
        // These are two *independent* simulator instances. The direct path
        // never builds an original PreparedDispatchStream even transiently.
        let (candidate_simulator, clock) = coordinator(actions, canonical);
        let (candidate, reset_direct) = PreparedDispatchStream::build_compact_direct_for_test(
            candidate_simulator,
            clock,
            &DispatchPreparationProbe::default(),
            &MaterializedInstrumentKeyProfile::canonical(),
        )
        .unwrap();

        let (control_simulator, _) = coordinator(actions, canonical);
        let (control, reset_inline) = PreparedDispatchStream::build(
            control_simulator,
            clock,
            &DispatchPreparationProbe::default(),
            &MaterializedInstrumentKeyProfile::canonical(),
        )
        .unwrap();
        let original_reserved_bytes = control.prepared_entry_allocation_for_test().1;
        let converted = control.into_compact_replacement_for_test().unwrap();

        assert_eq!(
            candidate
                .entries
                .iter()
                .map(|entry| format!("{entry:?}"))
                .collect::<Vec<_>>(),
            converted.entry_debug_fingerprints_for_test(),
            "frozen commits, deadlines, metadata and physical packet order differ"
        );
        assert_eq!(
            candidate.packet_summaries_for_test(),
            converted.packet_summaries_for_test(),
            "physical masks, lengths or Up-prefix boundaries differ"
        );
        assert_eq!(
            reset_direct.heap_payload_bytes_for_test_support(),
            reset_inline.heap_payload_bytes_for_test_support(),
        );
        let sizes = candidate.sizes().unwrap();
        assert_eq!(
            sizes.entry_count,
            sizes.physical_packet_count + sizes.metadata_entry_count
        );
        assert_eq!(
            sizes.total_reserved_bytes,
            sizes.entry_reserved_bytes
                + sizes.descriptor_reserved_bytes
                + sizes.input_reserved_bytes
        );
        assert_eq!(
            sizes.input_event_count,
            sizes
                .input_event_histogram
                .iter()
                .enumerate()
                .map(|(width, count)| width * count)
                .sum::<usize>()
        );
        (sizes, original_reserved_bytes)
    }

    #[test]
    fn direct_compact_small_with_stale_up_metadata_matches_control() {
        let keys = [0x15];
        let (sizes, original_reserved) = compare_direct_against_independent_inline(
            &[
                action(0, ActionKind::Down, 0, &keys),
                action(1, ActionKind::Up, 20_000, &keys),
                action(2, ActionKind::Up, 40_000, &keys),
            ],
            &keys,
        );
        assert_eq!(sizes.physical_packet_count, 2);
        assert_eq!(sizes.metadata_entry_count, 1);
        assert_eq!(sizes.input_event_count, 2);
        assert!(sizes.total_reserved_bytes < original_reserved);
        println!("DIRECT_COMPACT_SMALL original={original_reserved} direct={sizes:?}");
    }

    #[test]
    fn direct_compact_all_chord_widths_match_control() {
        let canonical = sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
        for width in 1..=canonical.len() {
            let keys = &canonical[..width];
            let (sizes, _) = compare_direct_against_independent_inline(
                &[
                    action(0, ActionKind::Down, 0, keys),
                    action(1, ActionKind::Up, 20_000, keys),
                ],
                &canonical,
            );
            assert_eq!(sizes.physical_packet_count, 2, "width={width}");
            assert_eq!(sizes.input_event_count, 2 * width, "width={width}");
        }
    }

    #[test]
    fn direct_compact_max_mixed_and_stale_metadata_match_control() {
        let keys = sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
        let (sizes, original_reserved) = compare_direct_against_independent_inline(
            &[
                action(0, ActionKind::Down, 0, &keys[..7]),
                action(1, ActionKind::Up, 20_000, &keys[..7]),
                action(2, ActionKind::Down, 20_000, &keys[7..]),
                action(3, ActionKind::Up, 40_000, &keys[7..]),
                action(4, ActionKind::Up, 60_000, &keys[..1]),
            ],
            &keys,
        );
        assert_eq!(sizes.physical_packet_count, 3);
        assert_eq!(sizes.metadata_entry_count, 1);
        assert_eq!(sizes.input_event_count, 30);
        assert!(sizes.total_reserved_bytes < original_reserved);
        println!("DIRECT_COMPACT_MIXED original={original_reserved} direct={sizes:?}");
    }

    #[test]
    fn compiled_mask_reservation_preserves_all_widths_and_stale_metadata() {
        let canonical = sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
        for width in 1..=canonical.len() {
            let keys = &canonical[..width];
            let actions = [
                action(0, ActionKind::Down, 0, keys),
                action(1, ActionKind::Up, 20_000, keys),
                action(2, ActionKind::Up, 40_000, keys),
            ];
            let (base_sim, clock) = coordinator(&actions, &canonical);
            let (baseline, reset_base) = PreparedDispatchStream::build_compact_direct_for_test(
                base_sim,
                clock,
                &DispatchPreparationProbe::default(),
                &MaterializedInstrumentKeyProfile::canonical(),
            )
            .unwrap();
            let (hint_sim, _) = coordinator(&actions, &canonical);
            let (hinted, reset_hinted) =
                PreparedDispatchStream::build_compact_direct_mask_reserved_for_test(
                    hint_sim,
                    clock,
                    &DispatchPreparationProbe::default(),
                    &MaterializedInstrumentKeyProfile::canonical(),
                )
                .unwrap();
            let a = baseline.sizes().unwrap();
            let b = hinted.sizes().unwrap();
            assert_eq!(
                baseline.packet_summaries_for_test(),
                hinted.packet_summaries_for_test()
            );
            assert_eq!(
                baseline
                    .entries
                    .iter()
                    .map(|x| format!("{x:?}"))
                    .collect::<Vec<_>>(),
                hinted
                    .entries
                    .iter()
                    .map(|x| format!("{x:?}"))
                    .collect::<Vec<_>>()
            );
            assert_eq!(a.input_event_count, width * 2, "width={width}");
            assert_eq!(a.input_event_histogram, b.input_event_histogram);
            assert_eq!(a.physical_packet_count, b.physical_packet_count);
            assert_eq!(a.metadata_entry_count, b.metadata_entry_count);
            assert!(b.input_capacity_events >= b.input_event_count);
            assert_eq!(b.input_size_bytes, 40);
            assert_eq!(
                reset_base.heap_payload_bytes_for_test_support(),
                reset_hinted.heap_payload_bytes_for_test_support()
            );
        }
    }

    #[test]
    fn direct_compact_stream_and_reset_checkpoint_errors_fail_closed() {
        let keys = [0x15];
        let actions = [
            action(0, ActionKind::Down, 0, &keys),
            action(1, ActionKind::Up, 20_000, &keys),
        ];
        let (simulator, clock) = coordinator(&actions, &keys);
        let blocked = PreparedDispatchStream::build_compact_direct_with_checkpoints_for_test(
            simulator,
            clock,
            &DispatchPreparationProbe::default(),
            &MaterializedInstrumentKeyProfile::canonical(),
            CompactInputReserveHint::TwoPerAuthoredBatch,
            || Err("stream_cancelled".into()),
            || Ok(()),
        );
        assert!(blocked.err().unwrap().contains("stream_cancelled"));

        let (simulator, _) = coordinator(&actions, &keys);
        let blocked = PreparedDispatchStream::build_compact_direct_with_checkpoints_for_test(
            simulator,
            clock,
            &DispatchPreparationProbe::default(),
            &MaterializedInstrumentKeyProfile::canonical(),
            CompactInputReserveHint::TwoPerAuthoredBatch,
            || Ok(()),
            || Err("reset_cancelled".into()),
        );
        assert!(blocked.err().unwrap().contains("reset_cancelled"));
    }
}
