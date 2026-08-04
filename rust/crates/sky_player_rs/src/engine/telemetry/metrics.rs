use parking_lot::Mutex;
use sky_dispatch_win32::input::ReleaseAllOutcome;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug, Clone, Default)]
pub(crate) struct RecentLatencyRing {
    values: [i32; 32],
    next: u8,
    len: u8,
}

impl RecentLatencyRing {
    pub(crate) fn push(&mut self, value: i64) {
        self.values[usize::from(self.next)] =
            value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.next = (self.next + 1) % self.values.len() as u8;
        self.len = self.len.saturating_add(1).min(self.values.len() as u8);
    }

    pub(crate) fn to_vec(&self) -> Vec<i64> {
        let len = usize::from(self.len);
        let start = if self.len == self.values.len() as u8 {
            usize::from(self.next)
        } else {
            0
        };
        (0..len)
            .map(|offset| i64::from(self.values[(start + offset) % self.values.len()]))
            .collect()
    }
}

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
    pub(crate) recent_latencies: RecentLatencyRing,
}

#[derive(Default)]
pub(crate) struct SharedMetrics {
    pub(crate) snapshot: parking_lot::Mutex<WorkerMetricsLocal>,
    pub(crate) last_publish_us: AtomicU64,
    pub(crate) is_paused: AtomicBool,
    pub(crate) panicked: AtomicBool,
    pub(crate) last_error: Mutex<Option<String>>,
    pub(crate) wait_strategy_acquired: Mutex<String>,
    pub(crate) terminal_error: Mutex<Option<String>>,
    pub(crate) secondary_errors: Mutex<Vec<String>>,
    pub(crate) generation_status_counts: Mutex<HashMap<String, u64>>,
    pub(crate) abort_counts_by_reason: Mutex<HashMap<String, u64>>,
    pub(crate) terminal_release_outcome: Mutex<Option<ReleaseAllOutcome>>,
    #[cfg(test)]
    pub(crate) publish_count: AtomicU64,
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

pub(crate) fn cpu_metrics_sample_due(now_us: u64, last_sample_us: u64, interval_us: u64) -> bool {
    now_us.saturating_sub(last_sample_us) >= interval_us
}
