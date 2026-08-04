#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PauseTimingPhase {
    Idle,
    Requested {
        generation: u64,
        requested_ticks: QpcTicks,
    },
    Observed {
        generation: u64,
        requested_ticks: QpcTicks,
        observed_ticks: QpcTicks,
    },
    Acknowledged {
        generation: u64,
        requested_ticks: QpcTicks,
        observed_ticks: QpcTicks,
        acknowledged_ticks: QpcTicks,
    },
    Cancelled {
        generation: u64,
    },
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandTimingLookup {
    Pending,
    Complete(CommandTimingResult),
    Cancelled,
    UnknownGeneration,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub(crate) enum CommandTimingError {
    InvalidGeneration,
    Clock(String),
    Ordering(String),
}

#[cfg(any(test, feature = "test-support"))]
impl std::fmt::Display for CommandTimingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGeneration => {
                write!(formatter, "pause timing generation must be non-zero")
            }
            Self::Clock(message) => {
                write!(formatter, "QPC pause timing conversion failed: {message}")
            }
            Self::Ordering(message) => {
                write!(formatter, "pause timing QPC ordering failed: {message}")
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl std::error::Error for CommandTimingError {}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub(crate) struct CommandTimingState {
    pub(crate) next_generation: AtomicU64,
    pub(crate) phase: Mutex<PauseTimingPhase>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandTimingResult {
    pub generation: u64,
    pub requested_ticks: QpcTicks,
    pub observed_ticks: QpcTicks,
    pub acknowledged_ticks: QpcTicks,
    pub observation_latency_us: u64,
    pub completion_latency_us: u64,
    pub cleanup_cost_us: u64,
}

#[cfg(any(test, feature = "test-support"))]
impl CommandTimingState {
    pub(crate) fn next_generation(&self) -> u64 {
        loop {
            let current = self.next_generation.load(Ordering::Relaxed);
            let next = current.wrapping_add(1).max(1);
            if self
                .next_generation
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return next;
            }
        }
    }

    pub(crate) fn request_pause(
        &self,
        requested_ticks: QpcTicks,
    ) -> Result<u64, CommandTimingError> {
        let mut phase = self.phase.lock();
        match *phase {
            PauseTimingPhase::Requested { generation, .. }
            | PauseTimingPhase::Observed { generation, .. }
            | PauseTimingPhase::Acknowledged { generation, .. } => Ok(generation),
            PauseTimingPhase::Idle | PauseTimingPhase::Cancelled { .. } => {
                let generation = self.next_generation();
                *phase = PauseTimingPhase::Requested {
                    generation,
                    requested_ticks,
                };
                Ok(generation)
            }
        }
    }

    pub(crate) fn observe_pause(&self, observed_ticks: QpcTicks) -> Option<u64> {
        let mut phase = self.phase.lock();
        let PauseTimingPhase::Requested {
            generation,
            requested_ticks,
        } = *phase
        else {
            return None;
        };
        *phase = PauseTimingPhase::Observed {
            generation,
            requested_ticks,
            observed_ticks,
        };
        Some(generation)
    }

    pub(crate) fn needs_observation(&self) -> bool {
        matches!(*self.phase.lock(), PauseTimingPhase::Requested { .. })
    }

    pub(crate) fn acknowledge_pause(&self, acknowledged_ticks: QpcTicks) -> Option<u64> {
        let mut phase = self.phase.lock();
        let PauseTimingPhase::Observed {
            generation,
            requested_ticks,
            observed_ticks,
        } = *phase
        else {
            return None;
        };
        *phase = PauseTimingPhase::Acknowledged {
            generation,
            requested_ticks,
            observed_ticks,
            acknowledged_ticks,
        };
        Some(generation)
    }

    pub(crate) fn needs_acknowledgment(&self) -> bool {
        matches!(*self.phase.lock(), PauseTimingPhase::Observed { .. })
    }

    pub(crate) fn cancel_pause_request(&self) -> Option<u64> {
        let mut phase = self.phase.lock();
        let generation = match *phase {
            PauseTimingPhase::Requested { generation, .. }
            | PauseTimingPhase::Observed { generation, .. } => generation,
            PauseTimingPhase::Idle
            | PauseTimingPhase::Acknowledged { .. }
            | PauseTimingPhase::Cancelled { .. } => return None,
        };
        *phase = PauseTimingPhase::Cancelled { generation };
        Some(generation)
    }

    pub(crate) fn result(
        &self,
        generation: u64,
        qpc_clock: QpcClock,
    ) -> Result<CommandTimingLookup, CommandTimingError> {
        if generation == 0 {
            return Err(CommandTimingError::InvalidGeneration);
        }
        let mut phase = self.phase.lock();
        match *phase {
            PauseTimingPhase::Requested {
                generation: current,
                ..
            }
            | PauseTimingPhase::Observed {
                generation: current,
                ..
            } if current == generation => Ok(CommandTimingLookup::Pending),
            PauseTimingPhase::Acknowledged {
                generation: current,
                requested_ticks,
                observed_ticks,
                acknowledged_ticks,
            } if current == generation => {
                let observation_ticks = observed_ticks
                    .checked_duration_since(requested_ticks)
                    .map_err(|error| CommandTimingError::Ordering(error.to_string()))?;
                let completion_ticks =
                    acknowledged_ticks
                        .checked_duration_since(requested_ticks)
                        .map_err(|error| CommandTimingError::Ordering(error.to_string()))?;
                let cleanup_ticks = acknowledged_ticks
                    .checked_duration_since(observed_ticks)
                    .map_err(|error| CommandTimingError::Ordering(error.to_string()))?;
                let result = CommandTimingResult {
                    generation,
                    requested_ticks,
                    observed_ticks,
                    acknowledged_ticks,
                    observation_latency_us: qpc_clock
                        .duration_to_us(observation_ticks)
                        .map_err(|error| CommandTimingError::Clock(format!("{error:?}")))?,
                    completion_latency_us: qpc_clock
                        .duration_to_us(completion_ticks)
                        .map_err(|error| CommandTimingError::Clock(format!("{error:?}")))?,
                    cleanup_cost_us: qpc_clock
                        .duration_to_us(cleanup_ticks)
                        .map_err(|error| CommandTimingError::Clock(format!("{error:?}")))?,
                };
                *phase = PauseTimingPhase::Idle;
                Ok(CommandTimingLookup::Complete(result))
            }
            PauseTimingPhase::Cancelled {
                generation: current,
            } if current == generation => {
                *phase = PauseTimingPhase::Idle;
                Ok(CommandTimingLookup::Cancelled)
            }
            _ => Ok(CommandTimingLookup::UnknownGeneration),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for CommandTimingState {
    fn default() -> Self {
        Self {
            next_generation: AtomicU64::new(0),
            phase: Mutex::new(PauseTimingPhase::Idle),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) struct CommandTimingCleanup<'a>(pub(crate) &'a CommandTimingState);

#[cfg(any(test, feature = "test-support"))]
impl Drop for CommandTimingCleanup<'_> {
    fn drop(&mut self) {
        self.0.cancel_pause_request();
    }
}
use parking_lot::Mutex;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use std::sync::atomic::{AtomicU64, Ordering};
