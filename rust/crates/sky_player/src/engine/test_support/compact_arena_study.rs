#![cfg(any(test, feature = "test-support"))]

//! Test-support adapter for the real-song prepared-stream arena comparison.
//! The original prepared stream remains the only production representation.

use super::dispatch_harness::{PreparedStartupBuilder, PreparedStartupEvidence};
use crate::engine::worker::{
    DispatchPreparationProbe, PreparedDispatchEntry, PreparedDispatchStream,
};
use sky_dispatch_win32::input::{
    MaterializedInstrumentKeyProfile, compact_study::CompactArenaSizes,
};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactDirectPreparedEvidence {
    pub physical_entry_count: usize,
    pub metadata_entry_count: usize,
    pub entry_size_bytes: usize,
    pub entry_capacity: usize,
    pub entry_reserved_bytes: usize,
    pub descriptor_reserved_bytes: usize,
    pub input_reserved_bytes: usize,
    pub total_reserved_bytes: usize,
    pub physical_input_event_histogram: [usize; sky_dispatch_win32::input::MAX_PACKET_EVENTS + 1],
    pub input_event_count: usize,
    pub rebuilt_coordinator_heap_payload_bytes: usize,
    /// One-shot build including coordinator simulation, INPUT materialization,
    /// arena freeze and reset; excludes compiler/planner source loading.
    /// Not an interleaved/cold-warm A/B estimate.
    pub build_ns_unpaired: u64,
}


impl PreparedStartupBuilder {
    pub fn prepare_with_compact_arena_measurement(
        self,
    ) -> Result<(PreparedStartupEvidence, CompactArenaSizes, u64, u64), String> {
        let probe = DispatchPreparationProbe::default();
        let profile = MaterializedInstrumentKeyProfile::canonical();
        let (stream, coordinator) =
            PreparedDispatchStream::build(self.coordinator, self.qpc_clock, &probe, &profile)?;
        let (arena_sizes, copy_ns, verify_ns) = stream.compact_arena_measurement_for_test()?;
        let (entry_count, physical_count, metadata_count, used_bytes) =
            stream.prepared_storage_evidence_for_test();
        let evidence = PreparedStartupEvidence {
            prepared_entry_count: entry_count,
            physical_entry_count: physical_count,
            metadata_entry_count: metadata_count,
            prepared_entry_type_size_bytes: std::mem::size_of::<PreparedDispatchEntry>(),
            prepared_entry_array_bytes: used_bytes,
            prepared_entry_allocation: stream.prepared_entry_allocation_for_test(),
            physical_input_event_histogram: stream.physical_input_event_histogram_for_test(),
            cancellation_ledger_bytes: stream.cancellation_ledger_bytes_for_test(),
            rebuilt_coordinator_heap_payload_bytes: coordinator
                .heap_payload_bytes_for_test_support(),
        };
        if arena_sizes.packet_count != physical_count {
            return Err("arena packet count differs from prepared stream".into());
        }
        Ok((evidence, arena_sizes, copy_ns, verify_ns))
    }

    /// Directly builds compact storage; unlike post-hoc arena study this does
    /// not first construct a song-sized PreparedDispatchStream of inline packets.
    /// The result is deliberately consumed into evidence, not into a sender.
    pub fn prepare_compact_direct_measurement(
        self,
    ) -> Result<CompactDirectPreparedEvidence, String> {
        let probe = DispatchPreparationProbe::default();
        let profile = MaterializedInstrumentKeyProfile::canonical();
        let start = Instant::now();
        let (stream, coordinator) = PreparedDispatchStream::build_compact_direct_for_test(
            self.coordinator,
            self.qpc_clock,
            &probe,
            &profile,
        )?;
        let build_ns_unpaired = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let sizes = stream.sizes()?;
        if sizes.entry_count != sizes.physical_packet_count + sizes.metadata_entry_count {
            return Err("compact direct stream entry count mismatch".into());
        }
        Ok(CompactDirectPreparedEvidence {
            physical_entry_count: sizes.physical_packet_count,
            metadata_entry_count: sizes.metadata_entry_count,
            entry_size_bytes: sizes.entry_size_bytes,
            entry_capacity: sizes.entry_capacity,
            entry_reserved_bytes: sizes.entry_reserved_bytes,
            descriptor_reserved_bytes: sizes.descriptor_reserved_bytes,
            input_reserved_bytes: sizes.input_reserved_bytes,
            total_reserved_bytes: sizes.total_reserved_bytes,
            physical_input_event_histogram: sizes.input_event_histogram,
            input_event_count: sizes.input_event_count,
            rebuilt_coordinator_heap_payload_bytes: coordinator
                .heap_payload_bytes_for_test_support(),
            build_ns_unpaired,
        })
    }

}
