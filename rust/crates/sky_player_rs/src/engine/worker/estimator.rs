use super::dispatch::timing::{EstimatorObservationEvidence, is_clean_estimator_observation};
use sky_dispatch_core::estimator::{DispatchCostEstimator, SendPath};

/// Update the dispatch-cost estimator from the one canonical clean
/// observation predicate.  The value trained here is physical completion
/// minus the immutable physical target, never the sender syscall duration.
pub(crate) fn update_estimator_after_send_observation(
    estimator: &mut DispatchCostEstimator,
    path: SendPath,
    event_count: usize,
    dispatch_cost_us: u64,
    evidence: EstimatorObservationEvidence,
) -> Result<(), sky_dispatch_core::estimator::EstimatorStateError> {
    if !is_clean_estimator_observation(evidence) || event_count == 0 {
        return Ok(());
    }
    estimator.update(path, event_count, dispatch_cost_us)
}

pub(crate) fn record_lead_saturation(
    counters: &mut [u64; 16],
    positive_residual_at_cap: &mut u64,
    polyphony: usize,
    completion_error_us: i64,
) {
    let bucket = polyphony.clamp(1, 15);
    counters[bucket] = counters[bucket].saturating_add(1);
    if completion_error_us > 0 {
        *positive_residual_at_cap = positive_residual_at_cap.saturating_add(1);
    }
}
