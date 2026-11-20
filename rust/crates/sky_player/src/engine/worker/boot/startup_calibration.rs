use super::startup_admission::{startup_abort_trigger, startup_probe_deadline, startup_wake_probe};
use crate::engine::config::{
    CALIBRATION_MAX_STARTUP_BUDGET_US, SpinThresholdSource, StartupCalibrationResult,
};
use crate::engine::shared::SessionShared;
use crate::engine::{QpcClock, QpcTicks};

pub(super) struct StartupCalibrationMeasurement {
    pub(super) stats: Option<sky_dispatch_win32::wait::WakeErrorStats>,
    pub(super) duration_us: u64,
    pub(super) result: StartupCalibrationResult,
}

impl StartupCalibrationMeasurement {
    pub(super) fn not_requested() -> Self {
        Self {
            stats: None,
            duration_us: 0,
            result: StartupCalibrationResult::NotRequested,
        }
    }
}

pub(super) fn select_startup_spin_threshold(
    config: &super::super::super::WorkerConfig,
    stats: Option<sky_dispatch_win32::wait::WakeErrorStats>,
) -> (u64, SpinThresholdSource) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(threshold_us) = config.wait.test_spin_threshold_us {
        let source = if threshold_us == 20_000 {
            SpinThresholdSource::LegacyTestWideSpin
        } else {
            SpinThresholdSource::TestFixedOverride
        };
        return (threshold_us, source);
    }

    #[cfg(not(any(test, feature = "test-support")))]
    let _ = config;

    if stats.is_some() {
        (
            super::super::timing::select_spin_threshold_us(stats),
            SpinThresholdSource::ProductionStartupCalibration,
        )
    } else {
        (
            super::super::timing::select_spin_threshold_us(None),
            SpinThresholdSource::ProductionFallback,
        )
    }
}

pub(super) fn apply_startup_calibration_result(
    config: &super::super::super::WorkerConfig,
    calibration: &StartupCalibrationMeasurement,
    metrics: &mut crate::engine::telemetry::WorkerMetricsLocal,
    qpc_clock: QpcClock,
) -> Result<crate::engine::DurationTicks, String> {
    let calibration_stats = calibration.stats;
    let (effective_spin_threshold_us, spin_threshold_source) =
        select_startup_spin_threshold(config, calibration_stats);
    metrics.startup_calibration_executed = calibration_stats.is_some();
    metrics.startup_calibration_sample_count = calibration_stats
        .as_ref()
        .map(|_| crate::engine::config::CALIBRATION_SAMPLES as u64)
        .unwrap_or(0);
    metrics.startup_calibration_duration_us = calibration.duration_us;
    metrics.startup_calibration_result = calibration.result as u8;
    metrics.spin_threshold_source = spin_threshold_source as u8;
    metrics.effective_spin_threshold_us = effective_spin_threshold_us;
    if let Some(stats) = calibration_stats {
        metrics.wake_error_p50_us = stats.p50_us;
        metrics.wake_error_p95_us = stats.p95_us;
        metrics.wake_error_p99_us = stats.p99_us;
        metrics.wake_error_max_us = stats.max_us;
        metrics.startup_wake_error_robust_us = stats.robust_us;
    }
    qpc_clock
        .duration_from_us(effective_spin_threshold_us)
        .map_err(|error| format!("spin threshold conversion failed: {error:?}"))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn measure_startup_calibration(
    shared: &SessionShared,
    require_focus: bool,
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    qpc_clock: QpcClock,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    deadline_origin: QpcTicks,
    #[cfg(any(test, feature = "test-support"))] startup_hook: Option<
        &crate::engine::config::StartupOrderingHook,
    >,
) -> StartupCalibrationMeasurement {
    let started = qpc_clock.now().ok();
    let stats = startup_wake_probe(
        waiter,
        qpc_clock,
        interrupt,
        deadline_origin,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    );
    let finished = qpc_clock.now().ok();
    let duration_us = started
        .zip(finished)
        .and_then(|(start, end)| end.checked_duration_since(start).ok())
        .and_then(|duration| qpc_clock.duration_to_us(duration).ok())
        .unwrap_or(0);
    let result = if stats.is_some() {
        StartupCalibrationResult::Completed
    } else if startup_abort_trigger(shared, require_focus).is_some() {
        StartupCalibrationResult::Interrupted
    } else {
        let budget_ticks = qpc_clock
            .duration_from_us(CALIBRATION_MAX_STARTUP_BUDGET_US)
            .ok();
        let deadline =
            budget_ticks.and_then(|budget| startup_probe_deadline(deadline_origin, budget));
        if deadline.is_some_and(|deadline| finished.is_some_and(|now| now >= deadline)) {
            StartupCalibrationResult::BudgetExpired
        } else {
            StartupCalibrationResult::Unavailable
        }
    };
    StartupCalibrationMeasurement {
        stats,
        duration_us,
        result,
    }
}
