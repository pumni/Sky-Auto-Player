from collections.abc import Iterable, Mapping, Sequence
from typing import Any, Literal

PlaybackProfile = Literal["production", "strict_timing_diagnostic"]


class SessionConfig:
    game_fps: int
    min_hold_us: int
    require_focus: bool
    target_hwnd: int
    telemetry: bool
    profile: PlaybackProfile
    estimator_state_json: str | None

    def __init__(self, *, game_fps: int, min_hold_us: int = ..., require_focus: bool = ..., target_hwnd: int = ..., telemetry: bool = ..., profile: PlaybackProfile = ..., estimator_state_json: str | None = ...) -> None: ...


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


class ProgressSnapshot:
    elapsed_us: int
    total_us: int
    max_completion_error_us: int
    late_2ms: int
    late_5ms: int
    late_10ms: int
    release_max_us: int
    release_late_2ms: int
    recent_latencies_us: Sequence[int]
    is_running: bool
    is_finished: bool
    is_paused: bool
    input_path_degraded: bool
    sendinput_path_degraded: bool
    bookkeeping_degraded: bool
    wait_path_degraded: bool
    send_warn_threshold_us: int
    bookkeeping_warn_threshold_us: int
    wait_warn_threshold_us: int
    sendinput_degraded_samples: int
    bookkeeping_degraded_samples: int
    wait_degraded_samples: int
    status: str
    health: Literal["ok", "degraded", "error"]
    backend_health: BackendHealthSnapshot


class DispatchSession:
    def __init__(self, py_actions: Iterable[tuple[int, str, int, Sequence[int], str]], allowed_scan_codes: Sequence[int], *, config: SessionConfig | None = ...) -> None: ...
    def start(self) -> None: ...
    def pause(self) -> None: ...
    def resume(self) -> None: ...
    def skip(self) -> None: ...
    def quit(self) -> None: ...
    def panic_release(self) -> None: ...
    def heartbeat(self) -> None: ...
    def set_target_hwnd(self, hwnd: int) -> None: ...
    def snapshot_lite(self) -> ProgressSnapshot: ...
    def snapshot(self) -> Mapping[str, Any]: ...
    def join(self, *, timeout_ms: int = ...) -> bool: ...
    def session_report(self) -> Mapping[str, Any]: ...
    def take_telemetry_json(self) -> str: ...
    def estimator_state_json(self) -> str: ...


def build_info() -> Mapping[str, Any]: ...
def instrument_scan_codes() -> Sequence[int]: ...
