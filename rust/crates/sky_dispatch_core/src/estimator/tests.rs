use super::{
    DispatchCostEstimator, ESTIMATOR_STATE_VERSION, EstimatorStateError, MAX_SAMPLE_US, SendPath,
};
use crate::estimator::window::{ROLLING_WINDOW_CAPACITY, SEED_SAMPLES};

fn seed(estimator: &mut DispatchCostEstimator, path: SendPath, count: usize, value: u64) {
    for _ in 0..SEED_SAMPLES {
        estimator.update(path, count, value).unwrap();
    }
}

#[test]
fn empty_path_lead_is_zero() {
    let estimator = DispatchCostEstimator::new(5_000, 30);
    assert_eq!(
        estimator
            .estimate_lead(SendPath::DownOnly, 1, false)
            .applied_us,
        0
    );
}

#[test]
fn four_samples_are_unseeded_and_five_seed() {
    let mut estimator = DispatchCostEstimator::new(5_000, 30);
    for _ in 0..4 {
        estimator.update(SendPath::DownOnly, 1, 100).unwrap();
    }
    assert_eq!(
        estimator
            .estimate_lead(SendPath::DownOnly, 1, false)
            .applied_us,
        0
    );
    estimator.update(SendPath::DownOnly, 1, 100).unwrap();
    assert_eq!(
        estimator
            .estimate_lead(SendPath::DownOnly, 1, false)
            .applied_us,
        100
    );
}

#[test]
fn p95_uses_nearest_rank_for_five_and_thirty_two_samples() {
    let mut estimator = DispatchCostEstimator::new(10_000, 30);
    for value in [10, 20, 30, 40, 50] {
        estimator.update(SendPath::DownOnly, 1, value).unwrap();
    }
    assert_eq!(
        estimator
            .estimate_lead(SendPath::DownOnly, 1, false)
            .applied_us,
        50
    );

    for value in 1..=ROLLING_WINDOW_CAPACITY as u64 {
        estimator.update(SendPath::UpOnly, 1, value).unwrap();
    }
    assert_eq!(
        estimator
            .estimate_lead(SendPath::UpOnly, 1, false)
            .applied_us,
        31
    );
}

#[test]
fn rolling_window_overwrites_oldest() {
    let mut estimator = DispatchCostEstimator::new(10_000, 30);
    for value in 1..=ROLLING_WINDOW_CAPACITY as u64 {
        estimator.update(SendPath::Mixed, 30, value).unwrap();
    }
    estimator.update(SendPath::Mixed, 30, 1_000).unwrap();
    let state = estimator.export_state();
    assert_eq!(state.mixed[30].samples.len(), ROLLING_WINDOW_CAPACITY);
    assert_eq!(state.mixed[30].samples[0], 2);
    assert_eq!(state.mixed[30].samples[31], 1_000);
}

#[test]
fn strict_estimate_uses_rolling_max() {
    let mut estimator = DispatchCostEstimator::new(10_000, 30);
    for value in [100, 100, 100, 100, 3_000] {
        estimator.update(SendPath::DownOnly, 1, value).unwrap();
    }
    assert_eq!(
        estimator
            .estimate_lead(SendPath::DownOnly, 1, false)
            .applied_us,
        3_000
    );
    assert_eq!(
        estimator
            .estimate_lead(SendPath::DownOnly, 1, true)
            .applied_us,
        3_000
    );
}

#[test]
fn fallback_prefers_nearest_lower_then_higher() {
    let mut estimator = DispatchCostEstimator::new(10_000, 30);
    seed(&mut estimator, SendPath::DownOnly, 2, 200);
    seed(&mut estimator, SendPath::DownOnly, 5, 500);
    assert_eq!(
        estimator
            .estimate_lead(SendPath::DownOnly, 3, false)
            .applied_us,
        200
    );

    let mut higher_only = DispatchCostEstimator::new(10_000, 30);
    seed(&mut higher_only, SendPath::UpOnly, 5, 500);
    assert_eq!(
        higher_only
            .estimate_lead(SendPath::UpOnly, 3, false)
            .applied_us,
        500
    );
}

#[test]
fn event_count_cache_is_monotonic_and_supports_30() {
    let mut estimator = DispatchCostEstimator::new(10_000, 30);
    seed(&mut estimator, SendPath::Mixed, 30, 3_000);
    seed(&mut estimator, SendPath::Mixed, 15, 150);
    let lead_15 = estimator
        .estimate_lead(SendPath::Mixed, 15, false)
        .applied_us;
    let lead_30 = estimator
        .estimate_lead(SendPath::Mixed, 30, false)
        .applied_us;
    assert_eq!(lead_15, 150);
    assert_eq!(lead_30, 3_000);
    for count in 1..30 {
        assert!(
            estimator
                .estimate_lead(SendPath::Mixed, count, false)
                .applied_us
                <= estimator
                    .estimate_lead(SendPath::Mixed, count + 1, false)
                    .applied_us
        );
    }
}

#[test]
fn saturation_is_reported_and_applied_is_clamped() {
    let mut estimator = DispatchCostEstimator::new(500, 30);
    seed(&mut estimator, SendPath::DownOnly, 1, 1_000);
    let estimate = estimator.estimate_lead(SendPath::DownOnly, 1, false);
    assert_eq!(estimate.applied_us, 500);
    assert_eq!(estimate.uncapped_us, 1_000);
    assert!(estimate.saturated);
}

#[test]
fn paths_refresh_independently() {
    let mut estimator = DispatchCostEstimator::new(10_000, 30);
    seed(&mut estimator, SendPath::DownOnly, 1, 100);
    seed(&mut estimator, SendPath::UpOnly, 1, 200);
    assert_eq!(
        estimator
            .estimate_lead(SendPath::DownOnly, 1, false)
            .applied_us,
        100
    );
    assert_eq!(
        estimator
            .estimate_lead(SendPath::UpOnly, 1, false)
            .applied_us,
        200
    );
    assert_eq!(
        estimator
            .estimate_lead(SendPath::Mixed, 1, false)
            .applied_us,
        0
    );
}

#[test]
fn state_v11_round_trips_oldest_to_newest() {
    let mut source = DispatchCostEstimator::new(5_000, 30);
    for value in 1..=7 {
        source.update(SendPath::Mixed, 2, value).unwrap();
    }
    let json = serde_json::to_string(&source.export_state()).unwrap();
    let mut restored = DispatchCostEstimator::new(5_000, 30);
    restored.import_state(&json).unwrap();
    assert_eq!(restored.export_state(), source.export_state());
    assert_eq!(restored.export_state().version, ESTIMATOR_STATE_VERSION);
}

#[test]
fn state_validation_rejects_v10_large_buckets_and_mismatch() {
    let mut estimator = DispatchCostEstimator::new(5_000, 30);
    let mut state = estimator.export_state();
    state.version = 10;
    let json = serde_json::to_string(&state).unwrap();
    assert!(matches!(
        estimator.import_state(&json),
        Err(EstimatorStateError::UnsupportedVersion(10))
    ));

    state.version = ESTIMATOR_STATE_VERSION;
    state.down[1].samples = vec![0; ROLLING_WINDOW_CAPACITY + 1];
    let json = serde_json::to_string(&state).unwrap();
    assert!(matches!(
        estimator.import_state(&json),
        Err(EstimatorStateError::TooManySamples(33))
    ));

    state.down[1].samples.clear();
    state.max_events = 29;
    let json = serde_json::to_string(&state).unwrap();
    assert!(matches!(
        estimator.import_state(&json),
        Err(EstimatorStateError::MaxEventsMismatch { .. })
    ));
}

#[test]
fn persisted_sample_cap_and_live_sample_clamp_are_distinct() {
    let mut estimator = DispatchCostEstimator::new(10_000, 30);
    estimator
        .update(SendPath::DownOnly, 1, MAX_SAMPLE_US + 1)
        .unwrap();
    let state = estimator.export_state();
    assert_eq!(state.down[1].samples[0], MAX_SAMPLE_US);
    let mut invalid = estimator.export_state();
    invalid.down[1].samples[0] = MAX_SAMPLE_US + 1;
    let json = serde_json::to_string(&invalid).unwrap();
    assert!(matches!(
        estimator.import_state(&json),
        Err(EstimatorStateError::PersistedSampleTooLarge(_))
    ));
}

#[test]
fn invalid_event_count_is_rejected() {
    let mut estimator = DispatchCostEstimator::new(5_000, 30);
    assert!(matches!(
        estimator.update(SendPath::Mixed, 0, 100),
        Err(EstimatorStateError::InvalidEventCount(0))
    ));
    assert!(matches!(
        estimator.update(SendPath::Mixed, 31, 100),
        Err(EstimatorStateError::InvalidEventCount(31))
    ));
}
