use super::dispatch::timing::{EstimatorObservationEvidence, is_clean_estimator_observation};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator, SendPath};
#[cfg(test)]
use sky_dispatch_win32::input::{PacketRetryReason, SendTransactionStatus};

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_estimator_after_send(
    estimator: &mut SendLatencyEstimator,
    path: SendPath,
    duration_us: u64,
    sent_count: usize,
    authored_polyphony: usize,
    applied_lead_us: u64,
    applied_lead_saturated: bool,
    completion_error_us: i64,
    clean_sample: bool,
) {
    let evidence = EstimatorObservationEvidence {
        status: if clean_sample {
            SendTransactionStatus::Complete
        } else {
            SendTransactionStatus::IntegrityLost
        },
        attempts: 1,
        retry_reason: PacketRetryReason::None,
        requested_count: sent_count,
        confirmed_count: if clean_sample { sent_count } else { 0 },
        skipped_count: 0,
        timing_valid: true,
        transport_anomaly: false,
        recovery_used: false,
        chord_integrity_lost: !clean_sample,
    };
    let _ = update_estimator_after_send_class(
        estimator,
        path,
        duration_us,
        sent_count,
        authored_polyphony,
        applied_lead_us,
        applied_lead_saturated,
        completion_error_us,
        evidence,
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
    applied_lead_saturated: bool,
    completion_error_us: i64,
    evidence: EstimatorObservationEvidence,
    latency_class: LatencyClass,
) -> Result<(), sky_dispatch_core::estimator::EstimatorStateError> {
    if !is_clean_estimator_observation(evidence) || sent_count == 0 {
        return Ok(());
    }
    estimator.update_observation(
        path,
        latency_class,
        duration_us,
        authored_polyphony,
        (applied_lead_us > 0).then_some(completion_error_us),
        applied_lead_saturated,
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
