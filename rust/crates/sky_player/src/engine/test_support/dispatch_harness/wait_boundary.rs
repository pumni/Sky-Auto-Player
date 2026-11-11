#![cfg(any(test, feature = "test-support"))]

use super::ProductionDispatchTestHarness;
use crate::engine::worker::dispatch::publish_interrupted_physical_wait;
use crate::engine::worker::{WaitBoundary, WaitResult};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcTicks;

impl ProductionDispatchTestHarness {
    pub(super) fn resolve_wait_boundary(
        &mut self,
        boundary: WaitBoundary,
        deadline_ticks: TimelineTicks,
        interrupted_reason: &'static str,
        exit_reason: &'static str,
    ) -> Result<(Option<WaitResult>, QpcTicks, DurationTicks, QpcTicks), String> {
        match boundary {
            WaitBoundary::Due {
                wait_result,
                target_qpc,
                dispatch_qpc,
                planned_wait_ticks,
            } => Ok((wait_result, dispatch_qpc, planned_wait_ticks, target_qpc)),
            WaitBoundary::Interrupted(wait_result, target_qpc, planned_wait_ticks) => {
                publish_interrupted_physical_wait(
                    &mut self.runtime,
                    Some(&self.observer),
                    &mut self.local_metrics,
                    wait_result,
                    target_qpc,
                    planned_wait_ticks,
                    deadline_ticks,
                    self.resources.playback.epoch,
                );
                Err(interrupted_reason.into())
            }
            WaitBoundary::Exit => Err(self
                .runtime
                .terminal_error
                .take()
                .unwrap_or_else(|| exit_reason.to_string())),
        }
    }
}
