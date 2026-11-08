//! Offline-only packet-component arena measurement of a real frozen stream.
//! No allocations or arena lookups are added to the production dispatch path.

use super::{PreparedDispatchEntry, PreparedDispatchStream};
use sky_dispatch_win32::input::compact_study::{
    CompactArenaSizes, CompactInputArenaBuilder,
};
use std::time::Instant;

impl PreparedDispatchStream {
    pub(crate) fn compact_arena_measurement_for_test(
        &self,
    ) -> Result<(CompactArenaSizes, u64, u64), String> {
        let histogram = self.physical_input_event_histogram_for_test();
        let event_count = histogram
            .iter()
            .enumerate()
            .try_fold(0usize, |sum, (width, count)| {
                sum.checked_add(
                    width
                        .checked_mul(*count)
                        .ok_or_else(|| "event count multiplication overflow".to_string())?,
                )
                .ok_or_else(|| "event count aggregation overflow".to_string())
            })?;
        let physical_count = self.physical_count();

        let start = Instant::now();
        let mut builder =
            CompactInputArenaBuilder::try_with_capacity(physical_count, event_count)?;
        for entry in &self.entries {
            if let PreparedDispatchEntry::Physical(frame) = entry {
                builder.append_prepared(&frame.view.prepared_packet)?;
            }
        }
        let arena = builder.freeze()?;
        let build_ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);

        let verify_start = Instant::now();
        let mut packet_index = 0usize;
        for entry in &self.entries {
            let PreparedDispatchEntry::Physical(frame) = entry else {
                continue;
            };
            let packet = &frame.view.prepared_packet;
            let stored = arena
                .packet_inputs(packet_index)
                .ok_or_else(|| "compact arena packet span missing".to_string())?;
            if stored.len() != usize::from(packet.event_count())
                || arena.packet_identity(packet_index) != Some(packet.packet())
            {
                return Err("compact packet identity or event count mismatch".into());
            }
            match (packet.up_recovery_view(), arena.mixed_up_prefix(packet_index)) {
                (None, None) => {}
                (Some(up), Some((mask, prefix))) => {
                    if mask != up.packet()
                        || prefix.len() != usize::from(up.packet().event_count())
                    {
                        return Err("compact mixed Up-prefix identity mismatch".into());
                    }
                }
                _ => return Err("compact mixed Up-prefix presence mismatch".into()),
            }
            packet_index += 1;
        }
        let verify_ns = u64::try_from(verify_start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let sizes = arena.sizes();
        if packet_index != physical_count
            || sizes.packet_count != physical_count
            || sizes.event_count != event_count
        {
            return Err("compact arena physical packet/event conservation failed".into());
        }
        Ok((sizes, build_ns, verify_ns))
    }
}
