use super::{INPUT_PATH_WINDOW_CAPACITY, TrackedKeyState};
use crate::engine::telemetry::{SharedMetrics, WorkerMetricsLocal};
use std::collections::VecDeque;

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
    if warn_us == 0 {
        *warn_started_us = None;
        *healthy_started_us = None;
        *degraded = false;
        return;
    }
    if window.len() == INPUT_PATH_WINDOW_CAPACITY
        && let Some(value) = window.pop_front()
        && value > warn_us
    {
        *over_warn_count = over_warn_count.saturating_sub(1);
    }
    let value = send_duration_us;
    window.push_back(value);
    debug_assert!(window.len() <= INPUT_PATH_WINDOW_CAPACITY);
    if value > warn_us {
        *over_warn_count += 1;
    }

    if window.len() < INPUT_PATH_WINDOW_CAPACITY {
        *warn_started_us = None;
        *healthy_started_us = None;
        return;
    }
    let bad_window = *over_warn_count >= 4;
    if bad_window {
        *healthy_started_us = None;
        let started = warn_started_us.get_or_insert(elapsed_us);
        if elapsed_us.saturating_sub(*started) >= 1_000_000 {
            *degraded = true;
        }
    } else {
        *warn_started_us = None;
        if *degraded {
            let started = healthy_started_us.get_or_insert(elapsed_us);
            if elapsed_us.saturating_sub(*started) >= 2_000_000 {
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
