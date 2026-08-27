from collections.abc import Iterable, Mapping, Sequence
from typing import Any, Literal

PlaybackProfile = Literal["production", "strict_timing_diagnostic"]


class SessionConfig:
    game_fps: int
    min_hold_us: int
    min_release_gap_us: int
    down_late_grace_us: int
    require_focus: bool
    target_hwnd: int
    telemetry: bool
    focus_restore_grace_us: int
    profile: PlaybackProfile
    def __init__(self, *, game_fps: int, min_hold_us: int = ..., down_late_grace_us: int = 0, require_focus: bool = ..., target_hwnd: int = ..., telemetry: bool = ..., focus_restore_grace_us: int = ..., profile: PlaybackProfile = ..., min_release_gap_us: int | None = ...) -> None: ...


class BackendHealthSnapshot:
    active_count: int
    possibly_active_count: int
    failed_release_count: int
    last_error: str | None
    keys_dropped: int
    chord_split_events: int
    sendinput_partial_events: int
    sendinput_zero_progress_failures: int
    chords_rejected: int
    authored_conflict_events: int
    authored_chords_rejected: int
    authored_keys_rejected: int
    keys_inserted_before_failure: int
    keys_rolled_back: int
    rollback_residue_keys: int


class PollSnapshot:
    elapsed_us: int
    pre_roll_remaining_us: int
    is_finished: bool
    is_paused: bool
    status: str


class ProgressSnapshot:
    elapsed_us: int
    total_us: int
    pre_roll_remaining_us: int
    max_completion_error_us: int
    late_2ms: int
    late_5ms: int
    late_10ms: int
    max_sendinput_pre_call_lateness_us: int
    pre_call_late_2ms: int
    pre_call_late_5ms: int
    pre_call_late_10ms: int
    release_max_us: int
    release_late_2ms: int
    recent_latencies_us: Sequence[int]
    recent_latency_samples_available: bool
    is_running: bool
    is_finished: bool
    is_paused: bool
    input_path_degraded: bool
    sendinput_path_degraded: bool
    core_post_send_degraded: bool
    observer_degraded: bool
    wait_path_degraded: bool
    sendinput_warn_threshold_us: int
    core_post_send_warn_threshold_us: int
    observer_warn_threshold_us: int
    wait_warn_threshold_us: int
    sendinput_degraded_samples: int
    core_post_send_degraded_samples: int
    observer_degraded_samples: int
    wait_degraded_samples: int
    wait_backend_failures: int
    wait_clock_failures: int
    wait_interrupted_count: int
    sendinput_window_bad_count: int
    core_post_send_window_bad_count: int
    observer_window_bad_count: int
    wait_window_bad_count: int
    sendinput_window_sample_count: int
    core_post_send_window_sample_count: int
    observer_window_sample_count: int
    wait_window_sample_count: int
    timeline_rebase_count: int
    timeline_rebase_total_us: int
    timeline_rebase_max_us: int
    core_post_send_max_us: int
    observer_duration_max_us: int
    observer_dropped_samples: int
    observer_queue_high_watermark: int
    dispatch_occupancy_max_us: int
    recovered_zero_progress_but_late: int
    recovered_zero_progress_retries: int
    recovered_partial_up_retries: int
    status: str
    health: Literal["ok", "degraded", "error"]
    backend_health: BackendHealthSnapshot


class DispatchSession:
    def __init__(self, py_actions: Iterable[tuple[int, str, int, Sequence[int], str]], *, config: SessionConfig | None = ...) -> None: ...
    def arm(self, pre_roll_us: int = ...) -> None: ...
    def pause(self) -> None: ...
    def resume(self) -> None: ...
    def skip(self) -> None: ...
    def quit(self) -> None: ...
    def panic_release(self) -> None: ...
    def heartbeat(self) -> None: ...
    def set_target_hwnd(self, hwnd: int) -> None: ...
    def set_focus_hint(self, active: bool) -> None: ...
    def poll_state(self) -> PollSnapshot: ...
    def snapshot_lite(self) -> ProgressSnapshot: ...
    def snapshot(self) -> Mapping[str, Any]: ...
    def join(self, *, timeout_ms: int = ...) -> bool: ...
    def session_report(self) -> Mapping[str, Any]: ...
    def take_telemetry_json(self) -> str: ...


def build_info() -> Mapping[str, Any]: ...
def instrument_scan_codes() -> Sequence[int]: ...
