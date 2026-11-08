#![cfg(all(windows, feature = "test-support"))]

//! Ownership-only compact prepared-stream projection. The production stream is
//! consumed and dropped; no original inline packet is retained by this owner.
//! This prototype is NOT an alternate dispatch path or a startup A/B result.

use super::{PreparedDispatchEntry, PreparedDispatchStream};
use crate::engine::worker::DispatchPath;
use crate::engine::worker::dispatch::{AuthoredBatchView, PhysicalCommit, RecoveryDescriptor};
use sky_dispatch_core::coordinator::{PreparedAuthoredCommit, PreparedBatch};
use sky_dispatch_core::model::ActionKind;
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::input::PhysicalPacket;
use sky_dispatch_win32::input::compact_study::{CompactInputArenaBuilder, FrozenCompactInputArena};
use std::mem::size_of;

// Every field except the original 608-byte inline packet is retained in the
// alternate physical entry. This is intentionally not a minimal data summary.
#[allow(dead_code)]
#[derive(Debug)]
struct CompactAuthoredView {
    prepared_batch: PreparedBatch,
    batch_source_action_index: u32,
    batch_intent_count: usize,
    batch_kind: ActionKind,
    batch_scheduled_ticks: TimelineTicks,
    authored_batch_scheduled_ticks: TimelineTicks,
    conflict_mask: u16,
    dispatch_path: DispatchPath,
    packet_masks: PhysicalPacket,
    recovery: RecoveryDescriptor,
    commit: PhysicalCommit,
}

#[allow(dead_code)]
#[derive(Debug)]
enum CompactPreparedEntry {
    Physical {
        packet_index: u32,
        view: CompactAuthoredView,
    },
    Metadata {
        offset_ticks: TimelineTicks,
        commit: PreparedAuthoredCommit,
    },
}

// The support library is checked without cfg(test); only focused unit tests construct this.
#[allow(dead_code)]
pub(crate) struct CompactReplacementStudy {
    entries: Vec<CompactPreparedEntry>,
    arena: FrozenCompactInputArena,
    original_entry_reserved_bytes: usize,
    physical_count: usize,
    metadata_count: usize,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompactReplacementSizes {
    pub original_entry_reserved_bytes: usize,
    pub compact_entry_size_bytes: usize,
    pub compact_entry_reserved_bytes: usize,
    pub compact_descriptor_reserved_bytes: usize,
    pub compact_input_reserved_bytes: usize,
    pub compact_total_reserved_bytes: usize,
    pub physical_packet_count: usize,
    pub metadata_entry_count: usize,
    pub input_event_count: usize,
}

impl PreparedDispatchStream {
    /// Transcodes an already-prepared stream and consumes its inline owner.
    /// Transient peak memory includes BOTH representations during conversion.
    /// Do not treat this as direct compact preparation or a production sender.
    #[allow(dead_code)]
    pub(crate) fn into_compact_replacement_for_test(
        self,
    ) -> Result<CompactReplacementStudy, String> {
        if self.cursor != 0 {
            return Err("compact replacement requires a fresh prepared cursor".into());
        }
        let original_entry_reserved_bytes = self.prepared_entry_allocation_for_test().1;
        let histogram = self.physical_input_event_histogram_for_test();
        let event_count =
            histogram
                .iter()
                .enumerate()
                .try_fold(0usize, |sum, (width, count)| {
                    width
                        .checked_mul(*count)
                        .and_then(|events| sum.checked_add(events))
                        .ok_or_else(|| "compact replacement event count overflow".to_string())
                })?;
        let physical_count = self.physical_count();
        let mut arena_builder =
            CompactInputArenaBuilder::try_with_capacity(physical_count, event_count)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(self.entries.len())
            .map_err(|error| format!("reserve compact replacement entries: {error}"))?;

        let mut physical_written = 0usize;
        let mut metadata_count = 0usize;
        for entry in self.entries {
            match entry {
                PreparedDispatchEntry::Physical(frame) => {
                    let AuthoredBatchView {
                        prepared_batch,
                        batch_source_action_index,
                        batch_intent_count,
                        batch_kind,
                        batch_scheduled_ticks,
                        authored_batch_scheduled_ticks,
                        conflict_mask,
                        dispatch_path,
                        packet_masks,
                        prepared_packet,
                        recovery,
                        commit,
                    } = frame.view;
                    if packet_masks != prepared_packet.packet() {
                        return Err("compact replacement packet identity mismatch".into());
                    }
                    if !matches!(&commit, PhysicalCommit::Authored(_)) {
                        return Err("compact replacement must retain authored-only commits".into());
                    }
                    arena_builder.append_prepared(&prepared_packet)?;
                    let packet_index = u32::try_from(physical_written)
                        .map_err(|_| "compact replacement packet index exceeds u32".to_string())?;
                    entries.push(CompactPreparedEntry::Physical {
                        packet_index,
                        view: CompactAuthoredView {
                            prepared_batch,
                            batch_source_action_index,
                            batch_intent_count,
                            batch_kind,
                            batch_scheduled_ticks,
                            authored_batch_scheduled_ticks,
                            conflict_mask,
                            dispatch_path,
                            packet_masks,
                            recovery,
                            commit,
                        },
                    });
                    physical_written += 1;
                }
                PreparedDispatchEntry::Metadata {
                    offset_ticks,
                    commit,
                } => {
                    entries.push(CompactPreparedEntry::Metadata {
                        offset_ticks,
                        commit,
                    });
                    metadata_count += 1;
                }
            }
        }
        let arena = arena_builder.freeze()?;
        if physical_written != physical_count {
            return Err("compact replacement lost a physical entry".into());
        }
        let mut ordinal = 0usize;
        for entry in &entries {
            let CompactPreparedEntry::Physical { packet_index, view } = entry else {
                continue;
            };
            if usize::try_from(*packet_index).ok() != Some(ordinal)
                || arena.packet_identity(ordinal) != Some(view.packet_masks)
            {
                return Err("compact replacement physical order or mask mismatch".into());
            }
            let inputs = arena
                .packet_inputs(ordinal)
                .ok_or_else(|| "compact replacement missing INPUT span".to_string())?;
            if inputs.len() != usize::from(view.packet_masks.event_count()) {
                return Err("compact replacement INPUT event count mismatch".into());
            }
            match (view.recovery, arena.mixed_up_prefix(ordinal)) {
                (RecoveryDescriptor::None, None) => {}
                (RecoveryDescriptor::UpPrefix { up_len, up_mask }, Some((mask, prefix)))
                    if mask == PhysicalPacket::new(up_mask, 0)
                        && prefix.len() == usize::from(up_len) => {}
                _ => return Err("compact replacement mixed Up-prefix mismatch".into()),
            }
            ordinal += 1;
        }
        if ordinal != physical_count
            || arena.sizes().event_count != event_count
            || physical_count + metadata_count != entries.len()
        {
            return Err("compact replacement packet/metadata/event conservation failed".into());
        }
        Ok(CompactReplacementStudy {
            entries,
            arena,
            original_entry_reserved_bytes,
            physical_count,
            metadata_count,
        })
    }
}

impl CompactReplacementStudy {
    #[allow(dead_code)]
    pub(crate) fn sizes(&self) -> Result<CompactReplacementSizes, String> {
        let arena = self.arena.sizes();
        let compact_entry_reserved_bytes = self
            .entries
            .capacity()
            .checked_mul(size_of::<CompactPreparedEntry>())
            .ok_or_else(|| "compact entry reserved bytes overflow".to_string())?;
        let compact_total_reserved_bytes = compact_entry_reserved_bytes
            .checked_add(arena.total_arena_reserved_bytes)
            .ok_or_else(|| "compact total reserved bytes overflow".to_string())?;
        Ok(CompactReplacementSizes {
            original_entry_reserved_bytes: self.original_entry_reserved_bytes,
            compact_entry_size_bytes: size_of::<CompactPreparedEntry>(),
            compact_entry_reserved_bytes,
            compact_descriptor_reserved_bytes: arena.descriptor_reserved_bytes,
            compact_input_reserved_bytes: arena.input_reserved_bytes,
            compact_total_reserved_bytes,
            physical_packet_count: self.physical_count,
            metadata_entry_count: self.metadata_count,
            input_event_count: arena.event_count,
        })
    }
}
