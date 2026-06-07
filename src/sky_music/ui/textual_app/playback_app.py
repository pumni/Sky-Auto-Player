from __future__ import annotations

from dataclasses import dataclass
import threading
from typing import TYPE_CHECKING

from textual import work
from textual.app import App, ComposeResult
from textual.containers import Container
from textual.widgets import Static

from sky_music.config import load_config
from sky_music.ui.hud import format_duration
from sky_music.ui.textual_app.theme_css import TEXTUAL_THEME_TOKENS, TextualThemeTokens

if TYPE_CHECKING:
    from sky_music.infrastructure.backend import BackendHealth
    from sky_music.orchestration.engine import PlaybackEngine

@dataclass(frozen=True, slots=True)
class PlaybackSnapshot:
    current: float
    total: float
    song_name: str
    status: str = "playing"
    input_path_degraded: bool = False
    backend_health: BackendHealth | None = None

class SnapshotRenderer:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self.snapshot: PlaybackSnapshot | None = None
        self.max_lateness_us: int = 0
        self.done: bool = False
        self.finish_message: str = ""

    def render(
        self,
        current: float,
        total: float,
        song_name: str,
        status: str = "playing",
        force: bool = False,
        input_path_degraded: bool = False,
        backend_health: BackendHealth | None = None,
    ) -> None:
        with self._lock:
            self.snapshot = PlaybackSnapshot(
                current=current,
                total=total,
                song_name=song_name,
                status=status,
                input_path_degraded=input_path_degraded,
                backend_health=backend_health,
            )

    def update_counters(self, lateness_us: int) -> None:
        if lateness_us > self.max_lateness_us:
            self.max_lateness_us = lateness_us

    def finish(self, message: str = "") -> None:
        self.done = True
        self.finish_message = message

    def get_snapshot(self) -> PlaybackSnapshot | None:
        with self._lock:
            return self.snapshot

BASE_CSS = """
Screen {
    align: center middle;
}
#playback-card {
    width: 66;
    height: auto;
    padding: 1 2;
    border: round #38506f;
    background: #080e1c;
}
#song-name {
    text-align: center;
    text-style: bold;
    margin-bottom: 1;
}
#progress-bar {
    text-align: center;
    margin-bottom: 1;
}
#time-info {
    text-align: center;
    margin-bottom: 1;
}
#status-info {
    text-align: center;
    text-style: bold;
    margin-bottom: 1;
}
#warning-info {
    text-align: center;
    margin-bottom: 1;
}
#hotkeys-info {
    text-align: center;
}
"""

def _theme_css(name: str, t: TextualThemeTokens) -> str:
    s = f"Screen.theme-{name}"
    return f"""
    {s}.background-transparent {{ background: transparent; color: {t.foreground}; }}
    {s}.background-painted {{ background: {t.background}; color: {t.foreground}; }}
    {s} #playback-card {{
        border: round {t.accent};
        background: {t.modal_background};
        color: {t.foreground};
    }}
    {s} #song-name {{
        color: {t.foreground};
    }}
    {s} #time-info {{
        color: {t.muted};
    }}
    {s} #hotkeys-info {{
        color: {t.muted};
    }}
    """

APP_CSS = BASE_CSS + "\n".join(
    _theme_css(name, tokens) for name, tokens in TEXTUAL_THEME_TOKENS.items()
)

class PlaybackApp(App[str]):
    CSS = APP_CSS

    def __init__(
        self,
        engine: PlaybackEngine,
        renderer: SnapshotRenderer,
        theme_name: str,
        song_name: str,
        total_us: int,
    ) -> None:
        super().__init__()
        self.engine = engine
        self.renderer = renderer
        self.theme_name = (theme_name or "aurora").casefold()
        if self.theme_name not in TEXTUAL_THEME_TOKENS:
            self.theme_name = "aurora"
        self.song_name = song_name
        self.total_us = total_us
        self._exited = False

    def compose(self) -> ComposeResult:
        total_str = format_duration(self.total_us / 1_000_000)
        with Container(id="playback-card"):
            yield Static(f"♪ {self.song_name}", id="song-name")
            yield Static("", id="progress-bar")
            yield Static(f"0:00 / {total_str}", id="time-info")
            yield Static("Playing", id="status-info")
            yield Static("", id="warning-info")
            yield Static("F8 pause  ·  F9 skip  ·  F10 quit", id="hotkeys-info")

    def on_mount(self) -> None:
        self.screen.add_class(f"theme-{self.theme_name}")
        user_cfg = load_config()
        bg_mode = (user_cfg.ui_background_mode or "transparent").casefold()
        self.screen.add_class(f"background-{bg_mode}")
        
        # Initialize warn widget visibility
        warn_widget = self.query_one("#warning-info", Static)
        warn_widget.styles.display = "none"

        self.run_engine()
        self.set_interval(0.1, self._poll)

    @work(thread=True, exclusive=True)
    def run_engine(self) -> None:
        try:
            result = self.engine.play()
            self.call_from_thread(self._safe_exit, result)
        except Exception:
            self.call_from_thread(self._safe_exit, "quit")

    def _safe_exit(self, result: str) -> None:
        if not self._exited:
            self._exited = True
            self.exit(result)

    def _poll(self) -> None:
        snap = self.renderer.get_snapshot()
        if snap is not None:
            self._update_ui(snap)

    def _update_ui(self, snap: PlaybackSnapshot) -> None:
        bar_width = 40
        total = max(snap.total, 0.001)
        fraction = min(1.0, max(0.0, snap.current / total))
        filled = int(round(fraction * bar_width))

        t = TEXTUAL_THEME_TOKENS[self.theme_name]
        bar_str = f"[{t.accent}]" + "█" * filled + "[/]" + f"[{t.muted}]" + "░" * (bar_width - filled) + "[/]"
        self.query_one("#progress-bar", Static).update(bar_str)

        current_str = format_duration(snap.current)
        total_str = format_duration(snap.total)
        remaining = max(0.0, snap.total - snap.current)
        remaining_str = format_duration(remaining)
        time_str = f"{current_str} / {total_str}  ·  ETA {remaining_str}"
        self.query_one("#time-info", Static).update(time_str)

        status_colors = {
            "playing": t.accent,
            "paused": t.warning,
            "focus_lost": t.danger,
            "waiting_for_focus": t.warning,
            "refocus": t.accent,
            "panic": t.warning,
            "done": t.accent,
        }
        status_labels = {
            "playing": "Playing",
            "paused": "Paused",
            "focus_lost": "Focus Lost",
            "waiting_for_focus": "Waiting for Focus",
            "refocus": "Refocusing",
            "panic": "Panic Release",
            "done": "Done",
        }
        status_val = snap.status
        color = status_colors.get(status_val, t.accent)
        label = status_labels.get(status_val, status_val.replace("_", " ").title())
        self.query_one("#status-info", Static).update(f"[{color}]{label}[/]")

        warn_widget = self.query_one("#warning-info", Static)
        if snap.input_path_degraded:
            warn_widget.update(f"[{t.warning}]Input path throttled (Filter Keys?) - playback may stutter[/]")
            warn_widget.styles.display = "block"
        else:
            warn_widget.update("")
            warn_widget.styles.display = "none"

def run_playback_textual(
    engine: PlaybackEngine,
    renderer: SnapshotRenderer,
    *,
    theme_name: str,
    song_name: str,
    total_us: int,
) -> str:
    app = PlaybackApp(
        engine=engine,
        renderer=renderer,
        theme_name=theme_name,
        song_name=song_name,
        total_us=total_us,
    )
    return app.run()
