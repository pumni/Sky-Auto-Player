use super::startup_admission::{startup_probe_deadline, validate_production_wait_backend};
use crate::engine::WaitOptions;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::wait::WaitFailure;

fn options(enable_waitable_timer: bool, enable_event_wait: bool) -> WaitOptions {
    WaitOptions {
        enable_waitable_timer,
        enable_event_wait,
        supervisor_lease_timeout_us: 0,
        #[cfg(any(test, feature = "test-support"))]
        test_spin_threshold_us: None,
        #[cfg(any(test, feature = "test-support"))]
        test_wait_policy: crate::engine::TestWaitPolicy::LegacyTestWideSpin,
    }
}

#[test]
fn production_wait_backend_requires_both_precision_primitives() {
    assert!(validate_production_wait_backend(true, &options(true, true), None).is_ok());
    assert!(validate_production_wait_backend(true, &options(false, true), None).is_err());
    assert!(validate_production_wait_backend(true, &options(true, false), None).is_err());
}

#[test]
fn production_wait_backend_rejects_startup_failure() {
    let result = validate_production_wait_backend(
        true,
        &options(true, true),
        Some(WaitFailure::TimerCreate { win32_error: 5 }),
    );

    assert!(result.is_err());
    assert!(
        result
            .expect_err("initial wait failure must reject production")
            .contains("high-resolution waitable timer creation failed")
    );
}

#[test]
fn non_production_wait_backend_can_keep_explicit_fallback_support() {
    assert!(
        validate_production_wait_backend(
            false,
            &options(false, false),
            Some(WaitFailure::TimerCreate { win32_error: 5 }),
        )
        .is_ok()
    );
}

#[test]
fn calibration_budget_is_independent_of_published_preroll() {
    let initial_now = QpcTicks::from_raw(100_000);
    let calibration_budget = DurationTicks::from_raw(20_000);

    assert_eq!(
        startup_probe_deadline(initial_now, calibration_budget),
        Some(QpcTicks::from_raw(120_000))
    );
}

#[test]
fn calibration_deadline_overflow_fails_closed() {
    let initial_now = QpcTicks::from_raw(100_000);
    let calibration_budget = DurationTicks::from_raw(u64::MAX);
    assert_eq!(
        startup_probe_deadline(initial_now, calibration_budget),
        None
    );
}
