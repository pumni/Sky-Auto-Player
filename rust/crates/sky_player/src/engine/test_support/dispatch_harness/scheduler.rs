#![cfg(any(test, feature = "test-support"))]

use super::ProductionDispatchTestHarness;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

impl ProductionDispatchTestHarness {
    pub fn current_executor_chord_for_test(
        &self,
    ) -> Option<sky_dispatch_core::executor::DownChordTicks> {
        self.resources.scheduler.current_chord().cloned()
    }

    pub fn attach_executor_for_test(&mut self) -> Result<(), String> {
        let min_release_gap_ticks = self
            .resources
            .clock
            .duration_from_us(self.config.timing.min_release_gap_us)
            .map_err(|e| format!("release-gap conversion failed: {e:?}"))?;
        let min_hold_ticks = self
            .resources
            .clock
            .duration_from_us(self.config.timing.min_hold_us)
            .map_err(|e| format!("min-hold conversion failed: {e:?}"))?;
        let clock = self.resources.clock;
        self.resources.scheduler = sky_dispatch_core::executor::DispatchExecutor::new(
            &self.legacy_coordinator_fixture.schedule,
            min_hold_ticks,
            min_release_gap_ticks,
            |microseconds| {
                clock
                    .timeline_from_us(microseconds)
                    .map_err(|e| format!("{e:?}"))
            },
        )?;
        self.legacy_coordinator_fixture
            .attach_executor_with_cancellation(
                min_hold_ticks,
                min_release_gap_ticks,
                |us| clock.timeline_from_us(us).map_err(|e| format!("{e:?}")),
                || Ok(()),
            )
    }

    pub fn stage_candidate_for_test(
        &mut self,
        up_mask: u16,
        down_chord: Option<sky_dispatch_core::executor::DownChordTicks>,
        fallback_ticks: TimelineTicks,
    ) -> Result<Option<crate::engine::worker::StagedCandidate>, String> {
        let up_slot_timings = self.resources.scheduler.up_slot_timings(up_mask);
        let authored_offset = down_chord
            .as_ref()
            .map(|chord| chord.scheduled_ticks)
            .or_else(|| {
                up_slot_timings
                    .iter()
                    .map(|timing| timing.authored_up_offset_ticks)
                    .min()
            })
            .unwrap_or(fallback_ticks);
        let authored_target_qpc = self
            .resources
            .playback
            .epoch
            .checked_add_duration(DurationTicks::from_raw(authored_offset.as_u64()))
            .map_err(|error| format!("test authored target overflow: {error}"))?;
        let packet_masks = sky_dispatch_win32::input::PhysicalPacket::new(
            up_mask,
            down_chord.as_ref().map_or(0, |chord| chord.down_mask),
        );
        let guard = self
            .runtime
            .physical_timing_guard
            .as_ref()
            .ok_or_else(|| "test physical timing guard is not initialized".to_string())?;
        let timing_window = guard
            .query(
                authored_target_qpc,
                packet_masks.up_mask,
                packet_masks.down_mask,
            )
            .map_err(|error| format!("test physical timing window query failed: {error:?}"))?;
        crate::engine::worker::stage_candidate(
            up_mask,
            down_chord,
            timing_window,
            &self.target,
            &mut self.resources,
            &mut self.runtime,
        )
    }
}
