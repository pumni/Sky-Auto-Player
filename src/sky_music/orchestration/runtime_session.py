import threading
from dataclasses import dataclass
from typing import TYPE_CHECKING

from sky_music.config import AppConfig, RtPriorityMode
from sky_music.domain.session_context import PlaybackSessionContext


@dataclass(frozen=True, slots=True)
class PlaybackOverrides:
    dry_run: bool = False
    profile: str | None = None
    tempo: float | None = None
    fps: int | None = None
    dispatch_lead_us: int = 0


@dataclass(slots=True)
class RuntimeSessionState:
    session: PlaybackSessionContext | None = None
    timing_policy: object | None = None
    sleep_policy: object | None = None
    scan_code_mode: str = "physical"
    telemetry_csv_enabled: bool = False
    dry_run: bool = False
    tempo_scale: float = 1.0
    timing_profile_name: str = "balanced"
    verbose_hud: bool = False
    use_dispatch_thread: bool = True
    enable_timer_guard: bool = True
    enable_waitable_timer: bool = True
    enable_gc_pause: bool = True
    enable_switch_interval_tuning: bool = True
    # Graduated defaults (2026-06-11): adaptive lead/spin, event-driven waits, and the MMCSS
    # priority ladder all ship ON; --no-* CLI flags are the per-feature kill switches.
    enable_adaptive_lead: bool = True
    enable_adaptive_spin: bool = True
    enable_event_wait: bool = True
    enable_epoch_rebase: bool = True
    rt_priority_mode: RtPriorityMode = "auto"
    check_input_path: bool = False
    spin_floor_us: int | None = None
    # When True, the launch-time auto update check is suppressed (set via
    # ``--no-update`` / ``--no-update-check``); manual checks via the ``u``
    # key still work. Honored by SkyPickerApp and the playback silent check.
    update_disabled: bool = False

    def apply_session(self, session: PlaybackSessionContext, cfg: AppConfig, *, spin_threshold_us: int | None = None) -> None:
        self.session = session
        self.timing_policy = session.resolve_effective_policy(cfg)
        self.sleep_policy = session.resolve_sleep_policy(cfg, spin_threshold_us=spin_threshold_us)
        self.scan_code_mode = session.scan_code_mode
        self.tempo_scale = session.tempo_scale
        self.timing_profile_name = session.display_profile_label()

    def clear_session(self) -> None:
        """Drop the last PlaybackSessionContext after playback ends (RAM hygiene)."""
        self.session = None


class _RuntimeStateProxy:
    """Thread-safe proxy for RuntimeSessionState.

    All attribute reads and writes go through a lock so that access from
    multiple threads (main thread + dispatch thread) is safe under
    free-threaded Python 3.14.  The proxy delegates method calls like
    ``apply_session`` directly under the lock.
    """

    def __init__(self) -> None:
        object.__setattr__(self, '_lock', threading.Lock())
        object.__setattr__(self, '_state', RuntimeSessionState())

    def __getattr__(self, name: str) -> object:
        state = object.__getattribute__(self, '_state')
        lock: threading.Lock = object.__getattribute__(self, '_lock')
        with lock:
            return getattr(state, name)

    def __setattr__(self, name: str, value: object) -> None:
        state = object.__getattribute__(self, '_state')
        lock: threading.Lock = object.__getattribute__(self, '_lock')
        with lock:
            setattr(state, name, value)

    def apply_session(
        self,
        session: PlaybackSessionContext,
        cfg: AppConfig,
        *,
        spin_threshold_us: int | None = None,
    ) -> None:
        state = object.__getattribute__(self, '_state')
        lock: threading.Lock = object.__getattribute__(self, '_lock')
        with lock:
            state.apply_session(session, cfg, spin_threshold_us=spin_threshold_us)

    def clear_session(self) -> None:
        """Drop the last PlaybackSessionContext after playback ends (RAM hygiene)."""
        state = object.__getattribute__(self, '_state')
        lock: threading.Lock = object.__getattribute__(self, '_lock')
        with lock:
            state.session = None


if TYPE_CHECKING:
    RUNTIME_STATE: RuntimeSessionState = RuntimeSessionState()
else:
    RUNTIME_STATE = _RuntimeStateProxy()
