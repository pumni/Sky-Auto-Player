use std::sync::atomic::Ordering;
use sky_dispatch_win32::input::ReleaseAllOutcome;
use parking_lot::Mutex;

#[derive(Debug, Clone, Default)]
pub struct WorkerMetricsLocal {
    pub elapsed_us: u64,
    pub total_us: u64,
    pub lateness_us: u64,
    pub max_lateness_us: u64,
    pub late_2ms: u64,
    pub late_5ms: u64,
    pub late_10ms: u64,
    pub release_max_us: u64,
    pub release_late_2ms: u64,
    pub active_count: u64,
    pub possibly_active_count: u64,
    pub failed_release_count: u64,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub sendinput_partial_events: u64,
    pub sendinput_zero_progress_failures: u64,
    pub chords_rejected: u64,
    pub authored_conflict_events: u64,
    pub authored_chords_rejected: u64,
    pub authored_keys_rejected: u64,
    pub keys_inserted_before_failure: u64,
    pub keys_rolled_back: u64,
    pub rollback_residue_keys: u64,
    pub lead_saturation_count_down: [u64; 16],
    pub lead_saturation_count_up: [u64; 16],
    pub positive_residual_at_cap: u64,
    pub recovered_zero_progress_but_late: u64,
    pub effective_spin_threshold_us: u64,
    pub wake_error_p50_us: u64,
    pub wake_error_p95_us: u64,
    pub wake_error_p99_us: u64,
    pub wake_error_max_us: u64,
    pub spin_time_us: u64,
    pub playback_wall_time_us: u64,
    pub spin_duty_cycle_ppm: u64,
    pub worker_cpu_time_us: u64,
    pub process_cpu_time_us: u64,
    pub power_throttling_disabled: bool,
    pub input_path_degraded: bool,
    pub sendinput_path_degraded: bool,
    pub bookkeeping_degraded: bool,
    pub wait_path_degraded: bool,
    pub wait_target_error_us: u64,
    pub idle_wake_count: u64,
    pub(crate) recent_latencies: crate::engine::telemetry::collector::RecentLatencyRing,
}

#[derive(Default)]
pub(crate) struct SharedMetrics {
    pub(crate) snapshot: parking_lot::Mutex<WorkerMetricsLocal>,
    pub(crate) last_publish_us: std::sync::atomic::AtomicU64,
    pub(crate) is_paused: std::sync::atomic::AtomicBool,
    pub(crate) panicked: std::sync::atomic::AtomicBool,
    pub(crate) last_error: parking_lot::Mutex<Option<String>>,
    pub(crate) wait_strategy_acquired: parking_lot::Mutex<String>,
    pub(crate) terminal_error: parking_lot::Mutex<Option<String>>,
    pub(crate) secondary_errors: parking_lot::Mutex<Vec<String>>,
    pub(crate) generation_status_counts: parking_lot::Mutex<std::collections::HashMap<String, u64>>,
    pub(crate) abort_counts_by_reason: parking_lot::Mutex<std::collections::HashMap<String, u64>>,
    pub(crate) terminal_release_outcome: parking_lot::Mutex<Option<ReleaseAllOutcome>>,
    #[cfg(test)]
    pub(crate) publish_count: std::sync::atomic::AtomicU64,
}

pub(crate) fn try_publish_metrics(
    local: &WorkerMetricsLocal,
    shared: &SharedMetrics,
    now_us: u64,
    force: bool,
) {
    let last = shared.last_publish_us.load(Ordering::Relaxed);
    if (force || now_us.saturating_sub(last) >= 50_000)
        && let Some(mut guard) = shared.snapshot.try_lock()
    {
        *guard = local.clone();
        shared.last_publish_us.store(now_us, Ordering::Relaxed);
        #[cfg(test)]
        shared.publish_count.fetch_add(1, Ordering::Relaxed);
    }
}
