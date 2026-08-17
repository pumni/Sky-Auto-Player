use super::{CoordinatorError, RuntimeDispatchCoordinator};
use crate::time::TimelineTicks;

impl RuntimeDispatchCoordinator {
    pub fn effective_total_ticks(&self) -> Result<TimelineTicks, CoordinatorError> {
        Ok(self
            .batch_scheduled_ticks
            .last()
            .copied()
            .unwrap_or(TimelineTicks::ZERO))
    }

    pub fn effective_batch_scheduled_ticks(
        &self,
        index: usize,
    ) -> Result<TimelineTicks, CoordinatorError> {
        self.batch_scheduled_ticks
            .get(index)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex { index })
    }
}
