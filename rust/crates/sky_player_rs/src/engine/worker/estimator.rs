use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator, SendPath};

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_estimator_after_send(
    estimator: &mut SendLatencyEstimator,
    path: SendPath,
    duration_us: u64,
    sent_count: usize,
    authored_polyphony: usize,
    applied_lead_us: u64,
    completion_error_us: i64,
    clean_sample: bool,
) {
    let _ = update_estimator_after_send_class(
        estimator,
        path,
        duration_us,
        sent_count,
        authored_polyphony,
        applied_lead_us,
        completion_error_us,
        clean_sample,
        LatencyClass::Hot,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_estimator_after_send_class(
    estimator: &mut SendLatencyEstimator,
    path: SendPath,
    duration_us: u64,
    sent_count: usize,
    authored_polyphony: usize,
    applied_lead_us: u64,
    completion_error_us: i64,
    clean_sample: bool,
    latency_class: LatencyClass,
) -> Result<(), sky_dispatch_core::estimator::EstimatorStateError> {
    if !clean_sample || sent_count == 0 {
        return Ok(());
    }
    estimator.update_observation(
        path,
        latency_class,
        duration_us,
        authored_polyphony,
        (applied_lead_us > 0).then_some(completion_error_us),
    )
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
