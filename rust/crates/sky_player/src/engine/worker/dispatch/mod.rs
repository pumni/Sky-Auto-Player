//! Production dispatch observation and forensics helpers.

pub(crate) mod hold_forensics;
pub(crate) mod observation;
pub(crate) mod observer;
pub(crate) mod observer_trace;
mod observer_wake;
pub(crate) mod timing;

/// Terminal result used by admission and observer code.
#[derive(Debug)]
pub enum DispatchStep {
    Terminate(String),
    TerminateStatic(&'static str),
}

pub(crate) use observer::{ObserverRuntime, PendingObservationQueue};
pub(crate) use observer_wake::{
    precision_handoff, publish_interrupted_physical_wait, publish_pending_wait_observation,
    record_wait_attempt, take_deadline_wake_qpc,
};
