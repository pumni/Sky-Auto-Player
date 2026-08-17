use parking_lot::Mutex;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::input::ReleaseAllOutcome;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};

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
    /// Legacy worker-publication field. Live session snapshots derive
    /// playback progress from the shared transition-only clock projection.
    pub elapsed_us: u64,
    pub total_us: u64,
    pub lateness_us: u64,
    pub max_lateness_us: u64,
    pub late_2ms: u64,
    pub late_5ms: u64,
    pub late_10ms: u64,
    /// Essential production scalar evidence measured immediately before the
    /// prepared SendInput call.
    pub max_sendinput_pre_call_lateness_us: u64,
    pub pre_call_late_2ms: u64,
    pub pre_call_late_5ms: u64,
    pub pre_call_late_10ms: u64,
    pub release_max_us: u64,
    pub release_late_2ms: u64,
    pub active_count: u64,
    pub possibly_active_count: u64,
    pub failed_release_count: u64,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub chord_integrity_lost: u64,
    pub sendinput_partial_events: u64,
    pub sendinput_zero_progress_failures: u64,
    pub chords_rejected: u64,
    pub authored_conflict_events: u64,
    pub authored_chords_rejected: u64,
    pub authored_keys_rejected: u64,
    pub keys_inserted_before_failure: u64,
    pub keys_rolled_back: u64,
    pub rollback_residue_keys: u64,
    /// Saturation diagnostics indexed by physical event count. Index 0 is
    /// unused; index 15 is the explicit `15_plus` overflow bucket.
    pub lead_saturation_count_down: [u64; 16],
    pub lead_saturation_count_up: [u64; 16],
    pub positive_residual_at_cap: u64,
    pub recovered_zero_progress_retries: u64,
    pub recovered_partial_up_retries: u64,
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
    pub core_post_send_degraded: bool,
    pub post_send_metrics_available: bool,
    pub observer_degraded: bool,
    pub wait_path_degraded: bool,
    pub sendinput_warn_threshold_us: u64,
    pub core_post_send_warn_threshold_us: u64,
    pub observer_warn_threshold_us: u64,
    pub wait_warn_threshold_us: u64,
    pub sendinput_degraded_samples: u64,
    pub core_post_send_degraded_samples: u64,
    pub observer_degraded_samples: u64,
    pub wait_degraded_samples: u64,
    pub wait_backend_failures: u64,
    pub wait_clock_failures: u64,
    pub wait_interrupted_count: u64,
    pub sendinput_window_bad_count: u64,
    pub core_post_send_window_bad_count: u64,
    pub observer_window_bad_count: u64,
    pub wait_window_bad_count: u64,
    pub sendinput_window_sample_count: u64,
    pub core_post_send_window_sample_count: u64,
    pub observer_window_sample_count: u64,
    pub wait_window_sample_count: u64,
    pub timeline_rebase_count: u64,
    pub timeline_rebase_total_us: u64,
    pub timeline_rebase_max_us: u64,
    pub timeline_rebase_total_ticks: DurationTicks,
    pub timeline_rebase_max_ticks: DurationTicks,
    /// Deprecated compatibility field. Authored dispatch never rebases its
    /// timeline, so production always leaves this at zero.
    pub timeline_rebase_last_reason: u8,
    pub dispatch_occupancy_max_us: u64,
    pub send_down_degraded_samples: u64,
    pub send_up_degraded_samples: u64,
    pub send_mixed_degraded_samples: u64,
    pub send_down_warn_threshold_us: u64,
    pub send_up_warn_threshold_us: u64,
    pub send_mixed_warn_threshold_us: u64,
    pub wait_target_error_us: u64,
    pub idle_wake_count: u64,
    /// Time between `sender_completed` and `dispatch_ready` on the hard
    /// critical path (typed QPC derivation), in microseconds.
    pub core_post_send_max_us: u64,
    /// Peak QPC duration from a blocking deadline wake to the next
    /// `SendInput` call entry (us). Only dispatches that followed a blocking
    /// deadline wait contribute a sample.
    pub wake_to_send_max_us: u64,
    /// Peak wall-clock duration of a single deferred observer drain step.
    pub observer_duration_max_us: u64,
    /// How many `DispatchObservation` samples were dropped because the fixed
    /// diagnostic observer queue was full (drop-new policy).
    pub observer_dropped_samples: u64,
    /// Largest number of entries ever held by the diagnostic observer queue.
    pub observer_queue_high_watermark: u64,
    pub(crate) recent_latencies: RecentLatencyRing,
}

impl WorkerMetricsLocal {
    /// Merge fields produced by the dedicated observer consumer into the
    /// dispatch-owned snapshot before terminal publication. The observer
    /// starts from zero, so producer-owned backend counters remain untouched.
    pub(crate) fn merge_observer(&mut self, observer: &Self) {
        self.total_us = self.total_us.saturating_add(observer.total_us);
        self.lateness_us = observer.lateness_us;
        self.max_lateness_us = self.max_lateness_us.max(observer.max_lateness_us);
        self.late_2ms = self.late_2ms.saturating_add(observer.late_2ms);
        self.late_5ms = self.late_5ms.saturating_add(observer.late_5ms);
        self.late_10ms = self.late_10ms.saturating_add(observer.late_10ms);
        self.release_max_us = self.release_max_us.max(observer.release_max_us);
        self.release_late_2ms = self
            .release_late_2ms
            .saturating_add(observer.release_late_2ms);
        self.recovered_zero_progress_retries = self
            .recovered_zero_progress_retries
            .saturating_add(observer.recovered_zero_progress_retries);
        self.recovered_partial_up_retries = self
            .recovered_partial_up_retries
            .saturating_add(observer.recovered_partial_up_retries);
        self.recovered_zero_progress_but_late = self
            .recovered_zero_progress_but_late
            .saturating_add(observer.recovered_zero_progress_but_late);
        self.effective_spin_threshold_us = self
            .effective_spin_threshold_us
            .max(observer.effective_spin_threshold_us);
        self.post_send_metrics_available |= observer.post_send_metrics_available;
        self.sendinput_warn_threshold_us = self
            .sendinput_warn_threshold_us
            .max(observer.sendinput_warn_threshold_us);
        self.core_post_send_warn_threshold_us = self
            .core_post_send_warn_threshold_us
            .max(observer.core_post_send_warn_threshold_us);
        self.observer_warn_threshold_us = self
            .observer_warn_threshold_us
            .max(observer.observer_warn_threshold_us);
        self.wait_warn_threshold_us = self
            .wait_warn_threshold_us
            .max(observer.wait_warn_threshold_us);
        self.input_path_degraded |= observer.input_path_degraded;
        self.sendinput_path_degraded |= observer.sendinput_path_degraded;
        self.core_post_send_degraded |= observer.core_post_send_degraded;
        self.observer_degraded |= observer.observer_degraded;
        self.wait_path_degraded |= observer.wait_path_degraded;
        self.sendinput_degraded_samples = self
            .sendinput_degraded_samples
            .saturating_add(observer.sendinput_degraded_samples);
        self.core_post_send_degraded_samples = self
            .core_post_send_degraded_samples
            .saturating_add(observer.core_post_send_degraded_samples);
        self.observer_degraded_samples = self
            .observer_degraded_samples
            .saturating_add(observer.observer_degraded_samples);
        self.wait_degraded_samples = self
            .wait_degraded_samples
            .saturating_add(observer.wait_degraded_samples);
        self.sendinput_window_bad_count = observer.sendinput_window_bad_count;
        self.core_post_send_window_bad_count = observer.core_post_send_window_bad_count;
        self.observer_window_bad_count = observer.observer_window_bad_count;
        self.wait_window_bad_count = observer.wait_window_bad_count;
        self.sendinput_window_sample_count = observer.sendinput_window_sample_count;
        self.core_post_send_window_sample_count = observer.core_post_send_window_sample_count;
        self.observer_window_sample_count = observer.observer_window_sample_count;
        self.wait_window_sample_count = observer.wait_window_sample_count;
        self.send_down_degraded_samples = self
            .send_down_degraded_samples
            .saturating_add(observer.send_down_degraded_samples);
        self.send_up_degraded_samples = self
            .send_up_degraded_samples
            .saturating_add(observer.send_up_degraded_samples);
        self.send_mixed_degraded_samples = self
            .send_mixed_degraded_samples
            .saturating_add(observer.send_mixed_degraded_samples);
        self.dispatch_occupancy_max_us = self
            .dispatch_occupancy_max_us
            .max(observer.dispatch_occupancy_max_us);
        self.core_post_send_max_us = self
            .core_post_send_max_us
            .max(observer.core_post_send_max_us);
        self.wake_to_send_max_us = self.wake_to_send_max_us.max(observer.wake_to_send_max_us);
        self.observer_duration_max_us = self
            .observer_duration_max_us
            .max(observer.observer_duration_max_us);
        self.timeline_rebase_count = self
            .timeline_rebase_count
            .max(observer.timeline_rebase_count);
        self.timeline_rebase_total_us = self
            .timeline_rebase_total_us
            .max(observer.timeline_rebase_total_us);
        self.timeline_rebase_max_us = self
            .timeline_rebase_max_us
            .max(observer.timeline_rebase_max_us);
        self.timeline_rebase_total_ticks = self
            .timeline_rebase_total_ticks
            .max(observer.timeline_rebase_total_ticks);
        self.timeline_rebase_max_ticks = self
            .timeline_rebase_max_ticks
            .max(observer.timeline_rebase_max_ticks);
        if observer.timeline_rebase_last_reason != 0 {
            self.timeline_rebase_last_reason = observer.timeline_rebase_last_reason;
        }
        self.observer_dropped_samples = self
            .observer_dropped_samples
            .saturating_add(observer.observer_dropped_samples);
        self.observer_queue_high_watermark = self
            .observer_queue_high_watermark
            .max(observer.observer_queue_high_watermark);
        for latency in observer.recent_latencies.to_vec() {
            self.recent_latencies.push(latency);
        }
    }
}

pub(crate) struct SnapshotBuffer {
    buffers: [UnsafeCell<WorkerMetricsLocal>; 2],
    active: AtomicU8,
    readers: [AtomicU32; 2],
}

// Safety: only the worker writes the inactive slot, and it skips a slot while
// a reader has pinned it. Readers validate the active index before and after
// copying, so an active slot is never concurrently overwritten.
unsafe impl Sync for SnapshotBuffer {}

impl Default for SnapshotBuffer {
    fn default() -> Self {
        Self {
            buffers: [
                UnsafeCell::new(WorkerMetricsLocal::default()),
                UnsafeCell::new(WorkerMetricsLocal::default()),
            ],
            active: AtomicU8::new(0),
            readers: [AtomicU32::new(0), AtomicU32::new(0)],
        }
    }
}

impl SnapshotBuffer {
    pub(crate) fn load(&self) -> WorkerMetricsLocal {
        loop {
            let index = self.active.load(Ordering::Acquire) as usize;
            self.readers[index].fetch_add(1, Ordering::Acquire);
            if self.active.load(Ordering::Acquire) as usize == index {
                let value = unsafe { (*self.buffers[index].get()).clone() };
                let stable = self.active.load(Ordering::Acquire) as usize == index;
                self.readers[index].fetch_sub(1, Ordering::Release);
                if stable {
                    return value;
                }
            } else {
                self.readers[index].fetch_sub(1, Ordering::Release);
            }
        }
    }

    pub(crate) fn try_publish(&self, local: &WorkerMetricsLocal) -> bool {
        let current = self.active.load(Ordering::Acquire) as usize;
        let target = 1 - current;
        if self.readers[target].load(Ordering::Acquire) != 0 {
            return false;
        }
        unsafe {
            *self.buffers[target].get() = local.clone();
        }
        self.active.store(target as u8, Ordering::Release);
        true
    }
}

#[derive(Default)]
pub(crate) struct SharedMetrics {
    pub(crate) snapshot: SnapshotBuffer,
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
    if (force || now_us.saturating_sub(last) >= 50_000) && shared.snapshot.try_publish(local) {
        shared.last_publish_us.store(now_us, Ordering::Relaxed);
        #[cfg(test)]
        shared.publish_count.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn cpu_metrics_sample_due(now_us: u64, last_sample_us: u64, interval_us: u64) -> bool {
    now_us.saturating_sub(last_sample_us) >= interval_us
}

#[cfg(test)]
mod tests {
    use super::{SnapshotBuffer, WorkerMetricsLocal};
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn snapshot_buffer_never_publishes_torn_scalar_pair() {
        let buffer = Arc::new(SnapshotBuffer::default());
        let writer_buffer = Arc::clone(&buffer);
        let writer = thread::spawn(move || {
            for value in 1..=10_000u64 {
                let local = WorkerMetricsLocal {
                    elapsed_us: value,
                    total_us: value,
                    ..WorkerMetricsLocal::default()
                };
                let _ = writer_buffer.try_publish(&local);
            }
        });
        for _ in 0..10_000 {
            let snapshot = buffer.load();
            assert_eq!(snapshot.elapsed_us, snapshot.total_us);
        }
        writer.join().expect("snapshot writer thread");
    }
}
