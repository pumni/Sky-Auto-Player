from __future__ import annotations

from collections import deque
from dataclasses import dataclass
import threading
from typing import TYPE_CHECKING, Any

from textual import work
from textual.app import App, ComposeResult
from textual.containers import Container
from textual.widgets import Static
from textual.screen import Screen

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

@dataclass(frozen=True, slots=True)
class DebugStats:
    max_lateness_us: int
    late_2ms: int
    late_5ms: int
    late_10ms: int
    p50_ms: float
    p95_ms: float
    jitter_ms: float
    active_keys: int
    stuck_keys: int
    backend_status: str

class SnapshotRenderer:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self.snapshot: PlaybackSnapshot | None = None
        self.max_lateness_us: int = 0
        self.late_2ms: int = 0
        self.late_5ms: int = 0
        self.late_10ms: int = 0
        self._latencies: deque = deque(maxlen=4096)
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
        if lateness_us > 2000:
            self.late_2ms += 1
        if lateness_us > 5000:
            self.late_5ms += 1
        if lateness_us > 10000:
            self.late_10ms += 1
        self._latencies.append(lateness_us)

    def debug_stats(self) -> DebugStats:
        samples = list(self._latencies)
        if samples:
            sorted_samples = sorted(samples)
            n = len(sorted_samples)
            p50_us = sorted_samples[n // 2]
            p95_index = min(n - 1, int(n * 0.95))
            p95_us = sorted_samples[p95_index]
            
            mean_us = sum(sorted_samples) / n
            variance = sum((x - mean_us) ** 2 for x in sorted_samples) / n
            stdev_ms = (variance ** 0.5) / 1000.0
        else:
            p50_us = 0
            p95_us = 0
            stdev_ms = 0.0

        snap = self.snapshot
        active_keys = 0
        stuck_keys = 0
        backend_status = "healthy"
        if snap is not None and snap.backend_health is not None:
            active_keys = snap.backend_health.active_count
            stuck_keys = snap.backend_health.failed_release_count
            if stuck_keys > 0:
                backend_status = f"stuck:{stuck_keys}"

        return DebugStats(
            max_lateness_us=self.max_lateness_us,
            late_2ms=self.late_2ms,
            late_5ms=self.late_5ms,
            late_10ms=self.late_10ms,
            p50_ms=p50_us / 1000.0,
            p95_ms=p95_us / 1000.0,
            jitter_ms=stdev_ms,
            active_keys=active_keys,
            stuck_keys=stuck_keys,
            backend_status=backend_status,
        )

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
    width: 78;
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
#debug-panel {
    align: center middle;
    margin-top: 1;
    margin-bottom: 1;
    height: auto;
}
#debug-backend, #debug-lateness, #debug-timing {
    text-align: center;
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
    {s} #debug-backend, {s} #debug-lateness, {s} #debug-timing {{
        color: {t.foreground};
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

class PlaybackScreen(Screen[str]):
    BINDINGS = [
        ("f2", "toggle_debug", "Toggle Debug"),
    ]

    def __init__(
        self,
        engine: PlaybackEngine,
        renderer: SnapshotRenderer,
        theme_name: str,
        song_name: str,
        total_us: int,
        violations: tuple[Any, ...] = (),
        active_policy: Any = None,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
        debug_mode: bool = False,
    ) -> None:
        super().__init__()
        self.engine = engine
        self.renderer = renderer
        self.theme_name = (theme_name or "aurora").casefold()
        if self.theme_name not in TEXTUAL_THEME_TOKENS:
            self.theme_name = "aurora"
        self.song_name = song_name
        self.total_us = total_us
        self.violations = violations
        self.active_policy = active_policy
        self.profile_name = profile_name
        self.tempo_scale = tempo_scale
        self.debug_mode = debug_mode
        self._exited = False

    def compose(self) -> ComposeResult:
        total_str = format_duration(self.total_us / 1_000_000)
        with Container(id="playback-card"):
            yield Static(f"♪ {self.song_name}", id="song-name")
            yield Static("", id="progress-bar")
            yield Static(f"0:00 / {total_str}", id="time-info")
            yield Static("Playing", id="status-info")
            yield Static("", id="warning-info")
            with Container(id="debug-panel"):
                yield Static("", id="debug-backend")
                yield Static("", id="debug-lateness")
                yield Static("", id="debug-timing")
            yield Static("F8 pause  ·  F9 skip  ·  F10 quit", id="hotkeys-info")

    def on_mount(self) -> None:
        self.screen.add_class(f"theme-{self.theme_name}")
        user_cfg = load_config()
        bg_mode = (user_cfg.ui_background_mode or "transparent").casefold()
        self.screen.add_class(f"background-{bg_mode}")

        warn_widget = self.query_one("#warning-info", Static)
        warn_widget.styles.display = "none"

        self._update_debug_visibility()

        self.run_engine()
        self.set_interval(0.1, self._poll)

    @work(thread=True, exclusive=True)
    def run_engine(self) -> None:
        try:
            result = self.engine.play()
            self.app.call_from_thread(self._safe_exit, result)
        except Exception:
            self.app.call_from_thread(self._safe_exit, "quit")

    def _safe_exit(self, result: str) -> None:
        if not self._exited:
            self._exited = True
            self.dismiss(result)

    def _poll(self) -> None:
        snap = self.renderer.get_snapshot()
        if snap is not None:
            self._update_ui(snap)

    def action_toggle_debug(self) -> None:
        self.debug_mode = not self.debug_mode
        self._update_debug_visibility()

    def _update_debug_visibility(self) -> None:
        panel = self.query_one("#debug-panel")
        hint = self.query_one("#hotkeys-info", Static)
        t = TEXTUAL_THEME_TOKENS[self.theme_name]
        
        if self.debug_mode:
            panel.styles.display = "block"
            hint.update(f"F8 pause  ·  F9 skip  ·  F10 quit  ·  [{t.accent}]F2 normal[/]")
        else:
            panel.styles.display = "none"
            hint.update(f"F8 pause  ·  F9 skip  ·  F10 quit  ·  [{t.accent}]F2 debug[/]")

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
        warnings_to_show = []
        if self.violations:
            warnings_to_show.append(f"[{t.warning}]Schedule violations: " + ", ".join(v.message for v in self.violations) + "[/]")
        if snap.input_path_degraded:
            warnings_to_show.append(f"[{t.warning}]Input path throttled (Filter Keys?) - playback may stutter[/]")

        if warnings_to_show:
            warn_widget.update("\n".join(warnings_to_show))
            warn_widget.styles.display = "block"
        else:
            warn_widget.update("")
            warn_widget.styles.display = "none"

        if self.debug_mode:
            stats = self.renderer.debug_stats()
            
            # Line 1: backend {healthy|stuck:N} · active keys: N
            backend_color = t.danger if stats.stuck_keys > 0 else t.success
            backend_str = f"backend [{backend_color}]{stats.backend_status}[/] · active keys: {stats.active_keys}"
            self.query_one("#debug-backend", Static).update(backend_str)
            
            # Line 2: late >2ms:N >5ms:N >10ms:N · max {x}ms · p50 {x}ms · p95 {x}ms · jitter {x}ms
            max_ms = stats.max_lateness_us / 1000.0
            lateness_str = (
                f"late >2ms:{stats.late_2ms} >5ms:{stats.late_5ms} >10ms:{stats.late_10ms} · "
                f"max {max_ms:.1f}ms · p50 {stats.p50_ms:.1f}ms · p95 {stats.p95_ms:.1f}ms · jitter {stats.jitter_ms:.1f}ms"
            )
            self.query_one("#debug-lateness", Static).update(lateness_str)
            
            # Line 3: Timing: {fps}fps ({frame_us}us) · hold/min {hold}/{min}us · {profile} {tempo}×
            if self.active_policy is not None:
                fps = getattr(self.active_policy, "fps", "N/A")
                frame_us = getattr(self.active_policy, "frame_us", "N/A")
                hold_us = getattr(self.active_policy, "hold_us", "N/A")
                min_hold = getattr(self.active_policy, "min_hold_us", "N/A")
            else:
                fps = "N/A"
                frame_us = "N/A"
                hold_us = "N/A"
                min_hold = "N/A"
            timing_str = (
                f"Timing: {fps}fps ({frame_us}us) · hold/min {hold_us}/{min_hold}us · "
                f"{self.profile_name} {self.tempo_scale:.2f}×"
            )
            self.query_one("#debug-timing", Static).update(timing_str)

class CountdownScreen(Screen[None]):
    def __init__(self, seconds: int, theme_name: str) -> None:
        super().__init__()
        self.seconds = seconds
        self.theme_name = (theme_name or "aurora").casefold()
        if self.theme_name not in TEXTUAL_THEME_TOKENS:
            self.theme_name = "aurora"
        self.remaining = seconds
        self._timer = None

    def compose(self) -> ComposeResult:
        with Container(id="playback-card"):
            yield Static("Preparing Playback", id="song-name")
            yield Static("", id="countdown-timer")
            yield Static("Bring Sky window to the foreground!", id="hotkeys-info")

    def on_mount(self) -> None:
        self.screen.add_class(f"theme-{self.theme_name}")
        user_cfg = load_config()
        bg_mode = (user_cfg.ui_background_mode or "transparent").casefold()
        self.screen.add_class(f"background-{bg_mode}")

        self.update_timer_label()
        self._timer = self.set_interval(1.0, self.tick)

    def update_timer_label(self) -> None:
        t = TEXTUAL_THEME_TOKENS[self.theme_name]
        self.query_one("#countdown-timer", Static).update(f"[bold {t.accent}]Playing in {self.remaining}...[/]")

    def tick(self) -> None:
        self.remaining -= 1
        if self.remaining <= 0:
            if self._timer:
                self._timer.stop()
            self.dismiss(None)
        else:
            self.update_timer_label()

