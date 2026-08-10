use super::{
    ActiveGeneration, CoordinatorError, CoordinatorInvariantError, GenerationStatus,
    RuntimeDispatchCoordinator,
};
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput, PhysicalPacketKind};
use crate::time::{DurationTicks, TimelineTicks};

mod authored;
mod invariants;
mod lifecycle;
