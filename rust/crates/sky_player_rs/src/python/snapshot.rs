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
    pub(super) max_completion_error_us: u64,
    #[pyo3(get)]
    pub(super) late_2ms: u64,
    #[pyo3(get)]
    pub(super) late_5ms: u64,
    #[pyo3(get)]
    pub(super) late_10ms: u64,
    #[pyo3(get)]
    pub(super) release_max_us: u64,
    #[pyo3(get)]
    pub(super) release_late_2ms: u64,
    #[pyo3(get)]
    pub(super) recent_latencies_us: Vec<i64>,
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
    pub(super) bookkeeping_degraded: bool,
    #[pyo3(get)]
    pub(super) wait_path_degraded: bool,
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
            max_completion_error_us: snapshot.max_lateness_us,
            late_2ms: snapshot.late_2ms,
            late_5ms: snapshot.late_5ms,
            late_10ms: snapshot.late_10ms,
            release_max_us: snapshot.release_max_us,
            release_late_2ms: snapshot.release_late_2ms,
            recent_latencies_us: snapshot.recent_latencies_us.clone(),
            is_running: snapshot.is_running,
            is_finished: snapshot.is_finished,
            is_paused: snapshot.is_paused,
            input_path_degraded: snapshot.input_path_degraded,
            sendinput_path_degraded: snapshot.sendinput_path_degraded,
            bookkeeping_degraded: snapshot.bookkeeping_degraded,
            wait_path_degraded: snapshot.wait_path_degraded,
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
