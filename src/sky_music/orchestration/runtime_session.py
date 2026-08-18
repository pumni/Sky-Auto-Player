import threading
from dataclasses import dataclass
from typing import TYPE_CHECKING

from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy
from sky_music.orchestration.native_admission import RustBuildInfo


@dataclass(frozen=True, slots=True)
class PlaybackOverrides:
    dry_run: bool = False
    hold_frames: float | None = None
    tempo: float | None = None
    fps: int | None = None


@dataclass(slots=True)
class RuntimeSessionState:
    session: PlaybackSessionContext | None = None
    rust_build_info: RustBuildInfo | None = None
    timing_policy: object | None = None
    scan_code_mode: str = "physical"
    telemetry_csv_enabled: bool = False
    dry_run: bool = False
    tempo_scale: float = 1.0
    hold_frames: float = 1.0
    hold_label: str = "hold 1.00f"
    verbose_hud: bool = False
    # When True, the launch-time auto update check is suppressed (set via
    # ``--no-update`` / ``--no-update-check``); manual checks via the ``u``
    # key still work. Honored by SkyPickerApp and the playback silent check.
    update_disabled: bool = False

    def apply_session(self, session: PlaybackSessionContext, cfg: AppConfig) -> None:
        self.session = session
        self.timing_policy = resolve_calibrated_policy(session, cfg)
        self.scan_code_mode = session.scan_code_mode
        self.tempo_scale = session.tempo_scale
        self.hold_frames = session.hold_frames
        self.hold_label = session.display_hold_label()

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
    ) -> None:
        state = object.__getattribute__(self, '_state')
        lock: threading.Lock = object.__getattribute__(self, '_lock')
        with lock:
            state.apply_session(session, cfg)

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
