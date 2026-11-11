use super::{CoordinatorError, GenerationStatus, RuntimeDispatchCoordinator};
use crate::model::MAX_RETAINED_DISPATCH_BYTES;
use crate::time::TimelineTicks;

/// Capacity-based accounting for one retained compiler schedule and its
/// runtime coordinator/executor. Allocator metadata and process RSS are not
/// included. `startup_peak_capacity_bytes` also includes a conservative copy
/// bound for coordinator buffers while they become boxed slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchMemoryMetrics {
    pub schedule_heap_bytes: usize,
    pub coordinator_heap_bytes: usize,
    pub executor_heap_bytes: usize,
    pub coordinator_struct_bytes: usize,
    pub retained_capacity_bytes: usize,
    pub startup_peak_capacity_bytes: usize,
}

pub(super) fn checked_storage_bytes(
    count: usize,
    element_size: usize,
) -> Result<usize, CoordinatorError> {
    count
        .checked_mul(element_size)
        .ok_or_else(|| CoordinatorError::Allocation("dispatch storage size overflow".into()))
}

pub(super) fn checked_storage_sum(
    values: impl IntoIterator<Item = usize>,
) -> Result<usize, CoordinatorError> {
    values.into_iter().try_fold(0usize, |total, value| {
        total.checked_add(value).ok_or_else(|| {
            CoordinatorError::Allocation("combined dispatch storage size overflow".into())
        })
    })
}

pub(super) fn check_retained_budget(required: usize) -> Result<(), CoordinatorError> {
    if required > MAX_RETAINED_DISPATCH_BYTES {
        return Err(CoordinatorError::Allocation(format!(
            "combined schedule, coordinator and executor capacity ({required} bytes) exceeds dispatch budget of {MAX_RETAINED_DISPATCH_BYTES} bytes"
        )));
    }
    Ok(())
}

impl RuntimeDispatchCoordinator {
    pub fn memory_metrics(&self) -> Result<DispatchMemoryMetrics, String> {
        let schedule = self.schedule.memory_metrics()?;
        let batch_ticks = self
            .batch_scheduled_ticks
            .len()
            .checked_mul(std::mem::size_of::<TimelineTicks>())
            .ok_or_else(|| "coordinator batch timing size overflow".to_string())?;
        let generation_states = self
            .generation_states
            .len()
            .checked_mul(std::mem::size_of::<GenerationStatus>())
            .ok_or_else(|| "coordinator generation ledger size overflow".to_string())?;
        let up_locations = self
            .up_intent_locations
            .len()
            .checked_mul(std::mem::size_of::<Option<(usize, usize)>>())
            .ok_or_else(|| "coordinator Up location size overflow".to_string())?;
        let coordinator_heap_bytes = [batch_ticks, generation_states, up_locations]
            .into_iter()
            .try_fold(0usize, |total, value| {
                total
                    .checked_add(value)
                    .ok_or_else(|| "coordinator heap payload size overflow".to_string())
            })?;
        let executor_heap_bytes = self
            .executor
            .as_ref()
            .map(crate::executor::DispatchExecutor::try_memory_metrics)
            .transpose()?
            .map_or(0, |metrics| metrics.chord_heap_bytes);
        let coordinator_struct_bytes = std::mem::size_of::<Self>();
        let retained_capacity_bytes = [
            schedule.heap_payload_bytes,
            coordinator_heap_bytes,
            executor_heap_bytes,
            coordinator_struct_bytes,
        ]
        .into_iter()
        .try_fold(0usize, |total, value| {
            total
                .checked_add(value)
                .ok_or_else(|| "combined retained dispatch size overflow".to_string())
        })?;
        let startup_peak_capacity_bytes = retained_capacity_bytes
            .checked_add(self.startup_peak_extra_bytes)
            .ok_or_else(|| "dispatch startup peak capacity size overflow".to_string())?;
        Ok(DispatchMemoryMetrics {
            schedule_heap_bytes: schedule.heap_payload_bytes,
            coordinator_heap_bytes,
            executor_heap_bytes,
            coordinator_struct_bytes,
            retained_capacity_bytes,
            startup_peak_capacity_bytes,
        })
    }

    pub(super) fn check_retained_with_executor_bytes(
        &self,
        executor_heap_bytes: usize,
    ) -> Result<(), String> {
        let schedule = self.schedule.memory_metrics()?.heap_payload_bytes;
        let coordinator_heap = self
            .batch_scheduled_ticks
            .len()
            .checked_mul(std::mem::size_of::<TimelineTicks>())
            .and_then(|bytes| {
                self.generation_states
                    .len()
                    .checked_mul(std::mem::size_of::<GenerationStatus>())
                    .and_then(|value| bytes.checked_add(value))
            })
            .and_then(|bytes| {
                self.up_intent_locations
                    .len()
                    .checked_mul(std::mem::size_of::<Option<(usize, usize)>>())
                    .and_then(|value| bytes.checked_add(value))
            })
            .ok_or_else(|| "coordinator storage size overflow".to_string())?;
        let required = [
            schedule,
            coordinator_heap,
            executor_heap_bytes,
            std::mem::size_of::<Self>(),
            self.startup_peak_extra_bytes,
        ]
        .into_iter()
        .try_fold(0usize, |total, value| total.checked_add(value))
        .ok_or_else(|| "combined retained dispatch size overflow".to_string())?;
        if required > MAX_RETAINED_DISPATCH_BYTES {
            return Err(format!(
                "combined schedule, coordinator and executor capacity ({required} bytes) exceeds dispatch budget of {MAX_RETAINED_DISPATCH_BYTES} bytes"
            ));
        }
        Ok(())
    }

    /// Approximate coordinator-owned heap payload for test-support
    /// characterization; excludes allocator metadata and schedule vectors.
    #[cfg(any(test, feature = "test-support"))]
    pub fn heap_payload_bytes_for_test_support(&self) -> usize {
        self.batch_scheduled_ticks.len() * std::mem::size_of::<TimelineTicks>()
            + self.generation_states.len() * std::mem::size_of::<GenerationStatus>()
            + self.up_intent_locations.len() * std::mem::size_of::<Option<(usize, usize)>>()
    }
}
