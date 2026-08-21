use crate::engine::EngineProgressSnapshot;
use pyo3::prelude::*;

#[pyclass(name = "BackendHealthSnapshot", frozen, skip_from_py_object)]
#[derive(Clone)]
pub(super) struct BackendHealthSnapshotPy {
    #[pyo3(get)]
    pub(super) active_count: usize,
    #[pyo3(get)]
    pub(super) possibly_active_count: usize,
    #[pyo3(get)]
    pub(super) failed_release_count: usize,
    #[pyo3(get)]
    pub(super) last_error: Option<String>,
    #[pyo3(get)]
    pub(super) keys_dropped: u64,
    #[pyo3(get)]
    pub(super) chord_split_events: u64,
    #[pyo3(get)]
    pub(super) sendinput_partial_events: u64,
    #[pyo3(get)]
    pub(super) sendinput_zero_progress_failures: u64,
    #[pyo3(get)]
    pub(super) chords_rejected: u64,
    #[pyo3(get)]
    pub(super) authored_conflict_events: u64,
    #[pyo3(get)]
    pub(super) authored_chords_rejected: u64,
    #[pyo3(get)]
    pub(super) authored_keys_rejected: u64,
    #[pyo3(get)]
    pub(super) keys_inserted_before_failure: u64,
    #[pyo3(get)]
    pub(super) keys_rolled_back: u64,
    #[pyo3(get)]
    pub(super) rollback_residue_keys: u64,
}

#[pyclass(name = "ProgressSnapshot", frozen)]
pub(super) struct ProgressSnapshotPy {
    #[pyo3(get)]
    pub(super) elapsed_us: u64,
    #[pyo3(get)]
    pub(super) total_us: u64,
    #[pyo3(get)]
    pub(super) pre_roll_remaining_us: u64,
    #[pyo3(get)]
    pub(super) max_completion_error_us: u64,
    #[pyo3(get)]
    pub(super) late_2ms: u64,
    #[pyo3(get)]
    pub(super) late_5ms: u64,
    #[pyo3(get)]
    pub(super) late_10ms: u64,
    #[pyo3(get)]
    pub(super) max_sendinput_pre_call_lateness_us: u64,
    #[pyo3(get)]
    pub(super) pre_call_late_2ms: u64,
    #[pyo3(get)]
    pub(super) pre_call_late_5ms: u64,
    #[pyo3(get)]
    pub(super) pre_call_late_10ms: u64,
    #[pyo3(get)]
    pub(super) release_max_us: u64,
    #[pyo3(get)]
    pub(super) release_late_2ms: u64,
    #[pyo3(get)]
    pub(super) recent_latencies_us: Vec<i64>,
    #[pyo3(get)]
    pub(super) recent_latency_samples_available: bool,
    #[pyo3(get)]
    pub(super) is_running: bool,
    #[pyo3(get)]
    pub(super) is_finished: bool,
    #[pyo3(get)]
    pub(super) is_paused: bool,
    #[pyo3(get)]
    pub(super) input_path_degraded: bool,
    #[pyo3(get)]
    pub(super) sendinput_path_degraded: bool,
    #[pyo3(get)]
    pub(super) core_post_send_degraded: bool,
    #[pyo3(get)]
    pub(super) observer_degraded: bool,
    #[pyo3(get)]
    pub(super) wait_path_degraded: bool,
    #[pyo3(get)]
    pub(super) sendinput_warn_threshold_us: u64,
    #[pyo3(get)]
    pub(super) core_post_send_warn_threshold_us: u64,
    #[pyo3(get)]
    pub(super) observer_warn_threshold_us: u64,
    #[pyo3(get)]
    pub(super) wait_warn_threshold_us: u64,
    #[pyo3(get)]
    pub(super) sendinput_degraded_samples: u64,
    #[pyo3(get)]
    pub(super) core_post_send_degraded_samples: u64,
    #[pyo3(get)]
    pub(super) observer_degraded_samples: u64,
    #[pyo3(get)]
    pub(super) wait_degraded_samples: u64,
    #[pyo3(get)]
    pub(super) wait_backend_failures: u64,
    #[pyo3(get)]
    pub(super) wait_clock_failures: u64,
    #[pyo3(get)]
    pub(super) wait_interrupted_count: u64,
    #[pyo3(get)]
    pub(super) sendinput_window_bad_count: u64,
    #[pyo3(get)]
    pub(super) core_post_send_window_bad_count: u64,
    #[pyo3(get)]
    pub(super) observer_window_bad_count: u64,
    #[pyo3(get)]
    pub(super) wait_window_bad_count: u64,
    #[pyo3(get)]
    pub(super) sendinput_window_sample_count: u64,
    #[pyo3(get)]
    pub(super) core_post_send_window_sample_count: u64,
    #[pyo3(get)]
    pub(super) observer_window_sample_count: u64,
    #[pyo3(get)]
    pub(super) wait_window_sample_count: u64,
    #[pyo3(get)]
    pub(super) timeline_rebase_count: u64,
    #[pyo3(get)]
    pub(super) timeline_rebase_total_us: u64,
    #[pyo3(get)]
    pub(super) timeline_rebase_max_us: u64,
    #[pyo3(get)]
    pub(super) core_post_send_max_us: u64,
    #[pyo3(get)]
    pub(super) wake_to_send_max_us: u64,
    #[pyo3(get)]
    pub(super) observer_duration_max_us: u64,
    #[pyo3(get)]
    pub(super) observer_dropped_samples: u64,
    #[pyo3(get)]
    pub(super) observer_queue_high_watermark: u64,
    #[pyo3(get)]
    pub(super) dispatch_occupancy_max_us: u64,
    #[pyo3(get)]
    pub(super) recovered_zero_progress_but_late: u64,
    #[pyo3(get)]
    pub(super) recovered_zero_progress_retries: u64,
    #[pyo3(get)]
    pub(super) recovered_partial_up_retries: u64,
    #[pyo3(get)]
    pub(super) missed_down_boundaries: u64,
    #[pyo3(get)]
    pub(super) missed_down_keys: u64,
    #[pyo3(get)]
    pub(super) missed_backlog_boundaries: u64,
    #[pyo3(get)]
    pub(super) missed_hard_late_boundaries: u64,
    #[pyo3(get)]
    pub(super) late_authorized_boundaries: u64,
    #[pyo3(get)]
    pub(super) deadline_authorization_reuses: u64,
    #[pyo3(get)]
    pub(super) late_discovery_rescue_attempts: u64,
    #[pyo3(get)]
    pub(super) late_discovery_rescue_sent: u64,
    #[pyo3(get)]
    pub(super) late_discovery_rescue_sender_cutoff_misses: u64,
    #[pyo3(get)]
    pub(super) late_discovery_rescue_credit_exhausted: u64,
    #[pyo3(get)]
    pub(super) late_discovery_rescue_blocked_control: u64,
    #[pyo3(get)]
    pub(super) late_discovery_rescue_blocked_focus_or_target: u64,
    #[pyo3(get)]
    pub(super) max_missed_lateness_ticks: u64,
    #[pyo3(get)]
    pub(super) production_forensics_available: bool,
    #[pyo3(get)]
    pub(super) production_forensics_version: u32,
    #[pyo3(get)]
    pub(super) production_hold_pair_samples: u64,
    #[pyo3(get)]
    pub(super) production_min_pre_call_hold_ticks: u64,
    #[pyo3(get)]
    pub(super) production_min_completion_hold_ticks: u64,
    #[pyo3(get)]
    pub(super) production_max_pre_call_shrink_ticks: u64,
    #[pyo3(get)]
    pub(super) production_max_completion_shrink_ticks: u64,
    #[pyo3(get)]
    pub(super) production_completion_hold_below_frame_count: u64,
    #[pyo3(get)]
    pub(super) production_release_gap_samples: u64,
    #[pyo3(get)]
    pub(super) production_min_release_gap_ticks: u64,
    #[pyo3(get)]
    pub(super) production_release_gap_below_policy_count: u64,
    #[pyo3(get)]
    pub(super) production_same_call_same_key_retrigger_count: u64,
    #[pyo3(get)]
    pub(super) production_anchor_overwrite_count: u64,
    #[pyo3(get)]
    pub(super) production_unmatched_up_count: u64,
    #[pyo3(get)]
    pub(super) production_anomaly_ring_overwrite_count: u64,
    #[pyo3(get)]
    pub(super) production_forensics_anomaly_count: u64,
    #[pyo3(get)]
    pub(super) status: String,
    #[pyo3(get)]
    pub(super) health: String,
    #[pyo3(get)]
    pub(super) backend_health: BackendHealthSnapshotPy,
}

impl ProgressSnapshotPy {
    pub(super) fn from_snapshot(snapshot: &EngineProgressSnapshot) -> Self {
        let health = if snapshot.has_terminal_error || snapshot.failed_release_count > 0 {
            "error"
        } else if snapshot.input_path_degraded {
            "degraded"
        } else {
            "ok"
        };
        Self {
            elapsed_us: snapshot.elapsed_us,
            total_us: snapshot.total_us,
            pre_roll_remaining_us: snapshot.pre_roll_remaining_us,
            max_completion_error_us: snapshot.max_lateness_us,
            late_2ms: snapshot.late_2ms,
            late_5ms: snapshot.late_5ms,
            late_10ms: snapshot.late_10ms,
            max_sendinput_pre_call_lateness_us: snapshot.max_sendinput_pre_call_lateness_us,
            pre_call_late_2ms: snapshot.pre_call_late_2ms,
            pre_call_late_5ms: snapshot.pre_call_late_5ms,
            pre_call_late_10ms: snapshot.pre_call_late_10ms,
            release_max_us: snapshot.release_max_us,
            release_late_2ms: snapshot.release_late_2ms,
            recent_latencies_us: snapshot.recent_latencies_us.clone(),
            recent_latency_samples_available: snapshot.recent_latency_samples_available,
            is_running: snapshot.is_running,
            is_finished: snapshot.is_finished,
            is_paused: snapshot.is_paused,
            input_path_degraded: snapshot.input_path_degraded,
            sendinput_path_degraded: snapshot.sendinput_path_degraded,
            core_post_send_degraded: snapshot.core_post_send_degraded,
            observer_degraded: snapshot.observer_degraded,
            wait_path_degraded: snapshot.wait_path_degraded,
            sendinput_warn_threshold_us: snapshot.sendinput_warn_threshold_us,
            core_post_send_warn_threshold_us: snapshot.core_post_send_warn_threshold_us,
            observer_warn_threshold_us: snapshot.observer_warn_threshold_us,
            wait_warn_threshold_us: snapshot.wait_warn_threshold_us,
            sendinput_degraded_samples: snapshot.sendinput_degraded_samples,
            core_post_send_degraded_samples: snapshot.core_post_send_degraded_samples,
            observer_degraded_samples: snapshot.observer_degraded_samples,
            wait_degraded_samples: snapshot.wait_degraded_samples,
            wait_backend_failures: snapshot.wait_backend_failures,
            wait_clock_failures: snapshot.wait_clock_failures,
            wait_interrupted_count: snapshot.wait_interrupted_count,
            sendinput_window_bad_count: snapshot.sendinput_window_bad_count,
            core_post_send_window_bad_count: snapshot.core_post_send_window_bad_count,
            observer_window_bad_count: snapshot.observer_window_bad_count,
            wait_window_bad_count: snapshot.wait_window_bad_count,
            sendinput_window_sample_count: snapshot.sendinput_window_sample_count,
            core_post_send_window_sample_count: snapshot.core_post_send_window_sample_count,
            observer_window_sample_count: snapshot.observer_window_sample_count,
            wait_window_sample_count: snapshot.wait_window_sample_count,
            timeline_rebase_count: snapshot.timeline_rebase_count,
            timeline_rebase_total_us: snapshot.timeline_rebase_total_us,
            timeline_rebase_max_us: snapshot.timeline_rebase_max_us,
            core_post_send_max_us: snapshot.core_post_send_max_us,
            wake_to_send_max_us: snapshot.wake_to_send_max_us,
            observer_duration_max_us: snapshot.observer_duration_max_us,
            observer_dropped_samples: snapshot.observer_dropped_samples,
            observer_queue_high_watermark: snapshot.observer_queue_high_watermark,
            dispatch_occupancy_max_us: snapshot.dispatch_occupancy_max_us,
            recovered_zero_progress_but_late: snapshot.recovered_zero_progress_but_late,
            recovered_zero_progress_retries: snapshot.recovered_zero_progress_retries,
            recovered_partial_up_retries: snapshot.recovered_partial_up_retries,
            missed_down_boundaries: snapshot.missed_down_boundaries,
            missed_down_keys: snapshot.missed_down_keys,
            missed_backlog_boundaries: snapshot.missed_backlog_boundaries,
            missed_hard_late_boundaries: snapshot.missed_hard_late_boundaries,
            late_authorized_boundaries: snapshot.late_authorized_boundaries,
            deadline_authorization_reuses: snapshot.deadline_authorization_reuses,
            late_discovery_rescue_attempts: snapshot.late_discovery_rescue_attempts,
            late_discovery_rescue_sent: snapshot.late_discovery_rescue_sent,
            late_discovery_rescue_sender_cutoff_misses: snapshot
                .late_discovery_rescue_sender_cutoff_misses,
            late_discovery_rescue_credit_exhausted: snapshot.late_discovery_rescue_credit_exhausted,
            late_discovery_rescue_blocked_control: snapshot.late_discovery_rescue_blocked_control,
            late_discovery_rescue_blocked_focus_or_target: snapshot
                .late_discovery_rescue_blocked_focus_or_target,
            max_missed_lateness_ticks: snapshot.max_missed_lateness_ticks,
            production_forensics_available: snapshot.production_forensics_available,
            production_forensics_version: snapshot.production_forensics_version,
            production_hold_pair_samples: snapshot.production_hold_pair_samples,
            production_min_pre_call_hold_ticks: snapshot.production_min_pre_call_hold_ticks,
            production_min_completion_hold_ticks: snapshot.production_min_completion_hold_ticks,
            production_max_pre_call_shrink_ticks: snapshot.production_max_pre_call_shrink_ticks,
            production_max_completion_shrink_ticks: snapshot.production_max_completion_shrink_ticks,
            production_completion_hold_below_frame_count: snapshot
                .production_completion_hold_below_frame_count,
            production_release_gap_samples: snapshot.production_release_gap_samples,
            production_min_release_gap_ticks: snapshot.production_min_release_gap_ticks,
            production_release_gap_below_policy_count: snapshot
                .production_release_gap_below_policy_count,
            production_same_call_same_key_retrigger_count: snapshot
                .production_same_call_same_key_retrigger_count,
            production_anchor_overwrite_count: snapshot.production_anchor_overwrite_count,
            production_unmatched_up_count: snapshot.production_unmatched_up_count,
            production_anomaly_ring_overwrite_count: snapshot
                .production_anomaly_ring_overwrite_count,
            production_forensics_anomaly_count: snapshot.production_forensics_anomaly_count,
            status: snapshot.status.clone(),
            health: health.to_string(),
            backend_health: BackendHealthSnapshotPy {
                active_count: snapshot.active_count,
                possibly_active_count: snapshot.possibly_active_count,
                failed_release_count: snapshot.failed_release_count,
                last_error: snapshot.last_error.clone(),
                keys_dropped: snapshot.keys_dropped,
                chord_split_events: snapshot.chord_split_events,
                sendinput_partial_events: snapshot.sendinput_partial_events,
                sendinput_zero_progress_failures: snapshot.sendinput_zero_progress_failures,
                chords_rejected: snapshot.chords_rejected,
                authored_conflict_events: snapshot.authored_conflict_events,
                authored_chords_rejected: snapshot.authored_chords_rejected,
                authored_keys_rejected: snapshot.authored_keys_rejected,
                keys_inserted_before_failure: snapshot.keys_inserted_before_failure,
                keys_rolled_back: snapshot.keys_rolled_back,
                rollback_residue_keys: snapshot.rollback_residue_keys,
            },
        }
    }
}
