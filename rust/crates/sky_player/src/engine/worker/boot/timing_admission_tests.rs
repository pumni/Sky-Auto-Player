use super::startup_admission::validate_production_wait_backend;
use super::timing_admission::convert;
use crate::engine::WaitOptions;
use crate::engine::config::{EffectiveRuntimePolicy, WorkerConfig};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::wait::WaitFailure;
use std::num::NonZeroU64;

fn options(enable_waitable_timer: bool, enable_event_wait: bool) -> WaitOptions {
    WaitOptions {
        enable_waitable_timer,
        enable_event_wait,
        supervisor_lease_timeout_us: 0,
        modifier_hold_timeout_us: crate::engine::config::DEFAULT_MODIFIER_HOLD_TIMEOUT_US,
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
fn timing_admission_converts_hold_with_ceil_in_odd_qpc_domain() {
    let mut config = WorkerConfig::default();
    config.timing.frame_us = 16_667;
    config.timing.frame_base_hold_us = 16_667;
    config.timing.timing_margin_us = 500;
    config.timing.min_hold_us = 17_167;
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(3_579_545).expect("QPC frequency"));
    let effective_policy = EffectiveRuntimePolicy::for_worker(&config);

    let converted = convert(&config, clock, &effective_policy).expect("converted policy");
    assert_eq!(
        converted.min_hold_ticks,
        clock.duration_from_us(17_167).expect("ceil H conversion")
    );
}

#[test]
fn timing_admission_rejects_zero_modifier_hold_timeout() {
    let mut config = WorkerConfig::default();
    config.wait.modifier_hold_timeout_us = 0;
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).expect("QPC frequency"));
    let effective_policy = EffectiveRuntimePolicy::for_worker(&config);

    match convert(&config, clock, &effective_policy) {
        Ok(_) => panic!("zero timeout must be rejected"),
        Err(err) => assert!(err.contains("modifier_hold_timeout_us must be greater than zero")),
    }
}
