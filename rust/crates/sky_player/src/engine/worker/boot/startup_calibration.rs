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
