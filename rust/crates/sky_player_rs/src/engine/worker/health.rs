use super::{INPUT_PATH_WINDOW_CAPACITY, TrackedKeyState};
use crate::engine::telemetry::{SharedMetrics, WorkerMetricsLocal};
use std::collections::VecDeque;

pub(crate) const SEND_WARNING_MARGIN_US: u64 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DispatchHealthOptions {
    pub(crate) send_warn_floor_us: u64,
    pub(crate) bookkeeping_warn_us: u64,
    pub(crate) wait_warn_us: u64,
    pub(crate) window_capacity: usize,
    pub(crate) bad_sample_count: usize,
    pub(crate) degrade_hold_us: u64,
    pub(crate) recovery_hold_us: u64,
}

impl Default for DispatchHealthOptions {
    fn default() -> Self {
        Self {
            send_warn_floor_us: 300,
            bookkeeping_warn_us: 300,
            wait_warn_us: 300,
            window_capacity: INPUT_PATH_WINDOW_CAPACITY,
            bad_sample_count: 4,
            degrade_hold_us: 1_000_000,
            recovery_hold_us: 2_000_000,
        }
    }
}

impl DispatchHealthOptions {
    pub(crate) fn send_warn_threshold_us(self, expected_send_us: u64) -> u64 {
        self.send_warn_floor_us
            .max(expected_send_us.saturating_add(SEND_WARNING_MARGIN_US))
    }
}

pub(crate) fn record_degraded_sample(value_us: u64, threshold_us: u64, samples: &mut u64) {
    if value_us > threshold_us {
        *samples = samples.saturating_add(1);
    }
}

pub(crate) fn record_lateness(
    lateness_us: i64,
    is_release: bool,
    deferred_release: bool,
    local_metrics: &mut WorkerMetricsLocal,
) {
    if deferred_release {
        return;
    }
    let clamped = lateness_us.max(0) as u64;
    local_metrics.lateness_us = clamped;
    if is_release {
        local_metrics.release_max_us = local_metrics.release_max_us.max(clamped);
        if clamped > 2_000 {
            local_metrics.release_late_2ms = local_metrics.release_late_2ms.saturating_add(1);
        }
        return;
    }
    local_metrics.max_lateness_us = local_metrics.max_lateness_us.max(clamped);
    if clamped > 10_000 {
        local_metrics.late_10ms = local_metrics.late_10ms.saturating_add(1);
    }
    if clamped > 5_000 {
        local_metrics.late_5ms = local_metrics.late_5ms.saturating_add(1);
    }
    if clamped > 2_000 {
        local_metrics.late_2ms = local_metrics.late_2ms.saturating_add(1);
    }
    local_metrics.recent_latencies.push(lateness_us);
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn record_input_path_health(
    send_duration_us: u64,
    elapsed_us: u64,
    warn_us: u64,
    window: &mut VecDeque<u64>,
    over_warn_count: &mut usize,
    warn_started_us: &mut Option<u64>,
    healthy_started_us: &mut Option<u64>,
    degraded: &mut bool,
) {
    record_input_path_health_with_options(
        send_duration_us,
        elapsed_us,
        warn_us,
        INPUT_PATH_WINDOW_CAPACITY,
        4,
        1_000_000,
        2_000_000,
        window,
        over_warn_count,
        warn_started_us,
        healthy_started_us,
        degraded,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_input_path_health_with_options(
    send_duration_us: u64,
    elapsed_us: u64,
    warn_us: u64,
    window_capacity: usize,
    bad_sample_count: usize,
    degrade_hold_us: u64,
    recovery_hold_us: u64,
    window: &mut VecDeque<u64>,
    over_warn_count: &mut usize,
    warn_started_us: &mut Option<u64>,
    healthy_started_us: &mut Option<u64>,
    degraded: &mut bool,
) {
    if warn_us == 0 {
        *warn_started_us = None;
        *healthy_started_us = None;
        *degraded = false;
        return;
    }
    if window.len() == window_capacity
        && let Some(value) = window.pop_front()
        && value > warn_us
    {
        *over_warn_count = over_warn_count.saturating_sub(1);
    }
    let value = send_duration_us;
    window.push_back(value);
    debug_assert!(window.len() <= window_capacity);
    if value > warn_us {
        *over_warn_count += 1;
    }

    if window.len() < window_capacity {
        *warn_started_us = None;
        *healthy_started_us = None;
        return;
    }
    let bad_window = *over_warn_count >= bad_sample_count;
    if bad_window {
        *healthy_started_us = None;
        let started = warn_started_us.get_or_insert(elapsed_us);
        if elapsed_us.saturating_sub(*started) >= degrade_hold_us {
            *degraded = true;
        }
    } else {
        *warn_started_us = None;
        if *degraded {
            let started = healthy_started_us.get_or_insert(elapsed_us);
            if elapsed_us.saturating_sub(*started) >= recovery_hold_us {
                *degraded = false;
                *healthy_started_us = None;
            }
        } else {
            *healthy_started_us = None;
        }
    }
}

pub(crate) fn focus_gate_matches(
    require_focus: bool,
    validated_focus_active: bool,
    target_hwnd: isize,
    foreground_matches: bool,
) -> bool {
    if !require_focus {
        return true;
    }
    let hwnd_matches = target_hwnd != 0 && foreground_matches;
    validated_focus_active && hwnd_matches
}

pub(crate) fn publish_backend_metrics(
    backend: &TrackedKeyState,
    local_metrics: &mut WorkerMetricsLocal,
    shared_metrics: &SharedMetrics,
    last_published_error: &mut Option<String>,
) {
    local_metrics.active_count = backend.active_mask.count_ones() as u64;
    local_metrics.keys_dropped = backend.keys_dropped;
    local_metrics.possibly_active_count = backend.possibly_active_mask.count_ones() as u64;
    local_metrics.failed_release_count = backend.failed_release_mask.count_ones() as u64;
    // The healthy dispatch path never takes this lock. Error text is
    // published only when the backend error state changes, including the
    // transition back to None after a successful recovery.
    if last_published_error.as_ref() != backend.last_error.as_ref() {
        let mut published = shared_metrics.last_error.lock();
        *published = backend.last_error.clone();
        *last_published_error = backend.last_error.clone();
    }
    local_metrics.chord_split_events = backend.chord_split_events;
    local_metrics.sendinput_partial_events = backend.sendinput_partial_events;
    local_metrics.sendinput_zero_progress_failures = backend.sendinput_zero_progress_failures;
    local_metrics.chords_rejected = backend.chords_rejected;
    local_metrics.authored_keys_rejected = backend.authored_keys_rejected;
    local_metrics.keys_inserted_before_failure = backend.keys_inserted_before_failure;
    local_metrics.keys_rolled_back = backend.keys_rolled_back;
    local_metrics.rollback_residue_keys = backend.rollback_residue_keys;
}

#[cfg(test)]
mod tests {
    use super::{
        DispatchHealthOptions, SEND_WARNING_MARGIN_US, record_degraded_sample,
        record_input_path_health_with_options,
    };
    use std::collections::VecDeque;

    #[test]
    fn send_warning_budget_grows_with_expected_polyphony_cost() {
        let options = DispatchHealthOptions::default();
        assert_eq!(
            options.send_warn_threshold_us(0),
            options.send_warn_floor_us
        );
        assert_eq!(
            options.send_warn_threshold_us(900),
            900 + SEND_WARNING_MARGIN_US
        );
        assert!(options.send_warn_threshold_us(900) > options.send_warn_threshold_us(50));
    }

    #[test]
    fn health_paths_have_independent_default_thresholds() {
        let options = DispatchHealthOptions::default();
        assert_eq!(options.bookkeeping_warn_us, 300);
        assert_eq!(options.wait_warn_us, 300);
        assert_eq!(options.window_capacity, 64);
        assert_eq!(options.bad_sample_count, 4);
        assert_eq!(options.degrade_hold_us, 1_000_000);
        assert_eq!(options.recovery_hold_us, 2_000_000);
    }

    #[test]
    fn degraded_sample_counter_only_counts_samples_over_budget() {
        let mut samples = 0;
        record_degraded_sample(300, 300, &mut samples);
        record_degraded_sample(301, 300, &mut samples);
        assert_eq!(samples, 1);
    }

    #[test]
    fn health_hysteresis_uses_bounded_window_and_time_holds() {
        let mut window = VecDeque::new();
        let mut over_warn = 0;
        let mut warn_started = None;
        let mut healthy_started = None;
        let mut degraded = false;
        macro_rules! record {
            ($value:expr, $elapsed:expr) => {
                record_input_path_health_with_options(
                    $value,
                    $elapsed,
                    300,
                    4,
                    2,
                    10,
                    20,
                    &mut window,
                    &mut over_warn,
                    &mut warn_started,
                    &mut healthy_started,
                    &mut degraded,
                )
            };
        }

        record!(301, 0);
        record!(301, 1);
        record!(301, 2);
        record!(301, 3);
        assert!(!degraded);
        record!(300, 10);
        record!(300, 20);
        assert!(degraded);
        assert_eq!(window.len(), 4);
        record!(300, 30);
        record!(300, 40);
        assert!(degraded);
        record!(300, 50);
        record!(300, 60);
        assert!(!degraded);
    }
}
