//! Test-support adapter for the real-song prepared-stream arena comparison.
//! The original prepared stream remains the only production representation.

use super::dispatch_harness::{PreparedStartupBuilder, PreparedStartupEvidence};
use crate::engine::worker::{
    DispatchPreparationProbe, PreparedDispatchEntry, PreparedDispatchStream,
};
use sky_dispatch_win32::input::{
    MaterializedInstrumentKeyProfile, compact_study::CompactArenaSizes,
};

impl PreparedStartupBuilder {
    pub fn prepare_with_compact_arena_measurement(
        self,
    ) -> Result<(PreparedStartupEvidence, CompactArenaSizes, u64, u64), String> {
        let probe = DispatchPreparationProbe::default();
        let profile = MaterializedInstrumentKeyProfile::canonical();
        let (stream, coordinator) =
            PreparedDispatchStream::build(self.coordinator, self.qpc_clock, &probe, &profile)?;
        let (arena_sizes, copy_ns, verify_ns) =
            stream.compact_arena_measurement_for_test()?;
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
}
