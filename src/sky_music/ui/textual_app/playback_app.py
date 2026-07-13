from __future__ import annotations

import contextlib
import queue
import threading
from collections import deque
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

from rich.text import Text
from textual import events, work
from textual.app import App, ComposeResult
from textual.containers import Container
from textual.screen import Screen
from textual.widgets import Static

from sky_music.config import load_config, resolve_game_fps
from sky_music.infrastructure.hotkeys import is_hotkey_down, parse_hotkey
from sky_music.ui.hud import _hex_to_ansi, format_duration
from sky_music.ui.picker_theme import get_theme_preset
from sky_music.ui.text_render import (
    ansi_box,
    ansi_gradient_box,
    truncate_cells,
    visible_width,
)
from sky_music.ui.textual_app.theme_css import TEXTUAL_THEME_TOKENS, TextualThemeTokens

if TYPE_CHECKING:
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.domain.validation import ScheduleInvariantViolation
    from sky_music.infrastructure.backend import BackendHealth
    from sky_music.infrastructure.hotkeys import HotkeyBinding
    from sky_music.orchestration.engine import PlaybackEngine

class PlaybackCommandBridge:
    """Merges global hotkey polling with UI-originated playback commands."""

    def __init__(self, base_controls: Any | None) -> None:
        self._base_controls = base_controls
        self._commands: queue.Queue[str] = queue.Queue()

    def poll(self) -> str | None:
        try:
            return self._commands.get_nowait()
        except queue.Empty:
            pass
        if self._base_controls is None:
            return None
        return self._base_controls.poll()

    def request(self, command: str) -> None:
        if command not in {"pause", "skip", "quit", "refocus", "panic"}:
            raise ValueError(f"unsupported playback command: {command}")
        self._commands.put(command)


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
    sigma_onset_ms: float  # renamed from jitter_ms: stdev of signed onset latency
    active_keys: int
    stuck_keys: int
    backend_status: str


class SnapshotRenderer:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._snapshot_data: PlaybackSnapshot | None = None
        # Onset-only counters (key-down) — these are what the player hears
        self.max_lateness_us: int = 0
        self.late_2ms: int = 0
        self.late_5ms: int = 0
        self.late_10ms: int = 0
        # Signed onset latencies for p50/p95/σ (deque of raw signed int)
        self._latencies: deque[int] = deque(maxlen=512)
        # Release counters (key-up) — separate from onset, verbose only
        self.release_late_2ms: int = 0
        self.release_max_us: int = 0
        self.done: bool = False
        self.finish_message: str = ""

    @property
    def snapshot(self) -> PlaybackSnapshot | None:
        return self._snapshot_data

    @snapshot.setter
    def snapshot(self, value: PlaybackSnapshot | None) -> None:
        self._snapshot_data = value

    def render(
        self,
        current: float,
        total: float,
        song_name: str,
        status: str = "playing",
        force: bool = False,  # noqa: ARG002
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

    def update_counters(self, lateness_us: int, kind: str = "down") -> None:
        """Called after each SendInput dispatch. Uses signed lateness_us; onset (down) only
        updates the p50/p95/σ ring buffer. Threshold counters use max(0, ...) to avoid
        counting early arrivals as 'late'."""
        clamped = max(0, lateness_us)
        if kind == "down":
            if clamped > self.max_lateness_us:
                self.max_lateness_us = clamped
            if clamped > 2000:
                self.late_2ms += 1
            if clamped > 5000:
                self.late_5ms += 1
            if clamped > 10000:
                self.late_10ms += 1
            # Store signed latency for dispersion stats (early arrivals show as negative)
            self._latencies.append(lateness_us)
        else:
            # Release — track separately for verbose/debug use
            if clamped > self.release_max_us:
                self.release_max_us = clamped
            if clamped > 2000:
                self.release_late_2ms += 1

    @staticmethod
    def _snapshot_latencies(values: deque[int]) -> list[int]:
        """Lock-free snapshot of the latency ring buffer.

        The dispatch thread appends to ``values`` on its hot path and must stay lock-free (a writer
        lock would let this UI-thread reader's sorted()/variance stall key dispatch → jitter). Under
        the free-threaded build ``list(deque)`` can raise "deque mutated during iteration" when an
        append lands mid-iteration, so retry a few times (append is O(1); collisions are rare) and
        fall back to an empty sample set rather than crash the render. A marginally inconsistent
        window is harmless for p50/p95/σ over 512 samples.
        """
        for _ in range(5):
            try:
                return list(values)
            except RuntimeError:
                continue
        return []

    def debug_stats(self) -> DebugStats:
        samples = self._snapshot_latencies(self._latencies)
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
            sigma_onset_ms=stdev_ms,
            active_keys=active_keys,
            stuck_keys=stuck_keys,
            backend_status=backend_status,
        )

    def counters_snapshot(self) -> DebugStats:
        """Lightweight stats for non-debug display — avoids sorted() + variance."""
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
            p50_ms=0.0,
            p95_ms=0.0,
            sigma_onset_ms=0.0,
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


_ANSI_RESET = "\033[0m"
_ANSI_BOLD = "\033[1m"


class PlaybackCard(Static):
    """In-place playback surface rendered as the legacy gradient HUD box.

    Reuses ``ansi_gradient_box``/``ansi_box`` from the console HUD so the in-app
    card is visually identical to the old terminal HUD, displayed inside Textual
    via ``Text.from_ansi`` rather than printed with cursor-move escapes.
    """

    can_focus = True

    def __init__(
        self,
        *,
        theme_name: str,
        song_name: str = "",
        total_us: int = 0,
        renderer: SnapshotRenderer | None = None,
        violations: tuple[ScheduleInvariantViolation, ...] = (),
        active_policy: FrameTimingPolicy | None = None,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
        debug_mode: bool = False,
        id: str = "playback-card",
    ) -> None:
        super().__init__("", id=id)
        name = (theme_name or "aurora").casefold()
        if name not in TEXTUAL_THEME_TOKENS:
            name = "aurora"
        self.theme_name = name
        self._preset = get_theme_preset(name)
        self.song_name = song_name
        self.total_us = total_us
        self.renderer = renderer or SnapshotRenderer()
        self.violations = violations
        self.active_policy = active_policy
        self.profile_name = profile_name
        self.tempo_scale = tempo_scale
        self.debug_mode = debug_mode

        self._mode = "idle"
        self._snapshot: PlaybackSnapshot | None = None
        self._idle_message = "Preparing playback"
        self._error_title = ""
        self._error_message = ""
        self._risk_severity = ""
        self._risk_recommendations: tuple[str, ...] = ()
        self._risk_options: tuple[str, ...] = ()
        self._risk_selected = 0
        self._countdown_remaining = 0
        self._countdown_callback: Any | None = None
        self._playback_result_callback: Any | None = None
        self._timer: Any | None = None
        self._poll_timer: Any | None = None
        self._debug_hotkey: HotkeyBinding | None = None
        self._debug_was_down = False
        self._exited = False
        self.engine: PlaybackEngine | None = None
        self.command_bridge: PlaybackCommandBridge | None = None

    # ----- Textual hooks -----------------------------------------------------
    def on_mount(self) -> None:
        self.show_idle("Preparing playback")

    def on_key(self, event: events.Key) -> None:
        if self._mode == "playing":
            command = {
                "f8": "pause",
                "f9": "skip",
                "f10": "quit",
            }.get(event.key)
            if command is not None:
                self._request_playback_command(command)
                event.stop()
                return
        handler = getattr(self.app, "handle_playback_card_key", None)
        if callable(handler) and handler(event.key):
            event.stop()

    def on_unmount(self) -> None:
        self._stop_timers()
        if not self._exited and self._mode == "playing":
            self._request_playback_command("quit")

    def render(self) -> Text:
        return Text.from_ansi("\n".join(self._compose_lines()))

    def _compose_lines(self) -> list[str]:
        width = self._box_width()
        body = self._build_body(width)
        preset = self._preset
        use_gradient = (
            preset.use_gradient_border
            and self._mode in {"playing", "countdown"}
            and self._effective_status() in {"playing", "done", "refocus", "countdown"}
        )
        if use_gradient:
            lines = ansi_gradient_box(
                "SKY MUSIC HELPER",
                body,
                width=width,
                gradient=preset.gradient,
                title_color=preset.modal_title,
            )
        else:
            lines = ansi_box(
                "SKY MUSIC HELPER",
                body,
                width=width,
                border_color=_hex_to_ansi(self._border_color()),
            )
        return lines

    def _rerender(self) -> None:
        lines = self._compose_lines()
        self.styles.height = len(lines)
        self.refresh(layout=True)

    # ----- mode setters ------------------------------------------------------
    def show_idle(self, message: str) -> None:
        self._mode = "idle"
        self._idle_message = message
        self._rerender()

    def show_error(self, title: str, message: str) -> None:
        self._mode = "error"
        self._error_title = title
        self._error_message = message
        self._rerender()
        self.focus()

    def show_risk(
        self,
        severity: str,
        recommendations: tuple[str, ...],
        options: tuple[str, ...],
        selected_index: int,
    ) -> None:
        self._mode = "risk"
        self._risk_severity = severity
        self._risk_recommendations = recommendations
        self._risk_options = options
        self._risk_selected = selected_index
        self._rerender()
        self.focus()

    def start_countdown(self, seconds: int, callback: Any) -> None:
        if self._timer is not None:
            self._timer.stop()
        self._mode = "countdown"
        self._countdown_remaining = seconds
        self._countdown_callback = callback
        self._rerender()
        self._timer = self.set_interval(1.0, self._tick_countdown)
        self.focus()

    def _tick_countdown(self) -> None:
        self._countdown_remaining -= 1
        if self._countdown_remaining <= 0:
            if self._timer is not None:
                self._timer.stop()
                self._timer = None
            callback = self._countdown_callback
            self._countdown_callback = None
            if callable(callback):
                callback()
            return
        self._rerender()

    def start_playback(
        self,
        *,
        engine: PlaybackEngine,
        renderer: SnapshotRenderer,
        song_name: str,
        total_us: int,
        violations: tuple[ScheduleInvariantViolation, ...],
        active_policy: FrameTimingPolicy,
        profile_name: str,
        tempo_scale: float,
        debug_mode: bool,
        result_callback: Any,
        command_bridge: PlaybackCommandBridge | None = None,
    ) -> None:
        self.engine = engine
        self.command_bridge = command_bridge
        self.renderer = renderer
        self.song_name = song_name
        self.total_us = total_us
        self.violations = violations
        self.active_policy = active_policy
        self.profile_name = profile_name
        self.tempo_scale = tempo_scale
        self.debug_mode = debug_mode
        controls = getattr(self.app, "controls", None)
        self._debug_hotkey = getattr(controls, "toggle_debug", None) or parse_hotkey("f2")
        self._debug_was_down = False
        self._playback_result_callback = result_callback
        self._exited = False
        self._mode = "playing"
        self._snapshot = None
        self._rerender()
        self.run_engine()
        self._poll_timer = self.set_interval(0.1, self._poll)
        self.focus()

    @work(thread=True, exclusive=True)
    def run_engine(self) -> None:
        try:
            if self.engine is None:
                raise RuntimeError("Playback engine is not configured")
            result = self.engine.play()
            self.app.call_from_thread(self._safe_finish, result)
        except Exception:
            self.app.call_from_thread(self._safe_finish, "quit")

    def _safe_finish(self, result: str) -> None:
        if self._exited:
            return
        self._exited = True
        self._stop_timers()
        self.engine = None
        self.command_bridge = None
        callback = self._playback_result_callback
        if callable(callback):
            callback(result)

    def _stop_timers(self) -> None:
        for attr in ("_timer", "_poll_timer"):
            timer = getattr(self, attr, None)
            if timer is not None:
                with contextlib.suppress(Exception):
                    timer.stop()
                setattr(self, attr, None)

    def _request_playback_command(self, command: str) -> None:
        bridge = self.command_bridge
        if bridge is not None:
            bridge.request(command)

    def _poll(self) -> None:
        self._poll_debug_hotkey()
        if self.renderer is None:
            return
        snap = self.renderer.get_snapshot()
        if snap is not None:
            self._snapshot = snap
            self._rerender()

    def _poll_debug_hotkey(self) -> None:
        hotkey = self._debug_hotkey
        if hotkey is None:
            return
        is_down = is_hotkey_down(hotkey)
        if is_down and not self._debug_was_down:
            self.toggle_debug()
        self._debug_was_down = is_down

    def toggle_debug(self) -> None:
        self.debug_mode = not self.debug_mode
        self._rerender()

    # ----- rendering helpers -------------------------------------------------
    def _box_width(self) -> int:
        width = self.size.width or 0
        if width <= 0:
            try:
                width = self.app.size.width
            except Exception:
                width = 72
        return max(40, min(width, 100))

    def _effective_status(self) -> str:
        if self._mode == "playing" and self._snapshot is not None:
            return self._snapshot.status
        return self._mode

    def _border_color(self) -> str:
        preset = self._preset
        return {
            "paused": preset.warning,
            "focus_lost": preset.danger,
            "waiting_for_focus": preset.warning,
            "panic": preset.warning,
            "error": preset.danger,
            "risk": preset.warning,
        }.get(self._effective_status(), preset.accent)

    def _build_body(self, width: int) -> list[str]:
        if self._mode == "error":
            return self._error_body()
        if self._mode == "risk":
            return self._risk_body()
        if self._mode == "countdown":
            return self._countdown_body()
        if self._mode == "playing":
            return self._playing_body(width)
        return ["", self._idle_message]

    def _error_body(self) -> list[str]:
        danger = _hex_to_ansi(self._preset.danger)
        muted = _hex_to_ansi(self._preset.muted)
        return [
            f"{_ANSI_BOLD}{self._error_title}{_ANSI_RESET}",
            "",
            f"{danger}{self._error_message}{_ANSI_RESET}",
            "",
            f"{muted}Esc return{_ANSI_RESET}",
        ]

    def _risk_body(self) -> list[str]:
        preset = self._preset
        muted = _hex_to_ansi(preset.muted)
        accent = _hex_to_ansi(preset.accent)
        color = {"high": preset.danger, "medium": preset.warning, "low": preset.success}.get(
            self._risk_severity, preset.accent
        )
        body = [f"{_ANSI_BOLD}{_hex_to_ansi(color)}Risk Level: {self._risk_severity.upper()}{_ANSI_RESET}", ""]
        body.extend(f"{muted}• {rec}{_ANSI_RESET}" for rec in self._risk_recommendations)
        if self._risk_recommendations:
            body.append("")
        for index, label in enumerate(self._risk_options):
            if index == self._risk_selected:
                body.append(f"{accent}{_ANSI_BOLD}❯ {index + 1}. {label}{_ANSI_RESET}")
            else:
                body.append(f"  {index + 1}. {label}")
        body.append("")
        body.append(f"{muted}↑↓/Enter or 1-5  ·  Esc cancel{_ANSI_RESET}")
        return body

    def _countdown_body(self) -> list[str]:
        preset = self._preset
        accent = _hex_to_ansi(preset.accent)
        muted = _hex_to_ansi(preset.muted)
        return [
            f"{_ANSI_BOLD}Preparing Playback{_ANSI_RESET}",
            "",
            f"{accent}{_ANSI_BOLD}Playing in {self._countdown_remaining}...{_ANSI_RESET}",
            "",
            f"{muted}Sky will be focused for playback{_ANSI_RESET}",
        ]

    def _playing_body(self, width: int) -> list[str]:
        """Mirror ``ProgressRenderer.render`` body assembly for visual parity."""
        preset = self._preset
        accent = _hex_to_ansi(preset.accent)
        green = _hex_to_ansi(preset.success)
        yellow = _hex_to_ansi(preset.warning)
        red = _hex_to_ansi(preset.danger)
        gray = _hex_to_ansi(preset.muted)
        divider_c = _hex_to_ansi(preset.divider)
        key_c = _hex_to_ansi(preset.key)

        snap = self._snapshot
        current = snap.current if snap else 0.0
        total = snap.total if snap else (self.total_us / 1_000_000)
        status = snap.status if snap else "playing"
        degraded = snap.input_path_degraded if snap else False

        status_labels = {
            "playing": "Playing",
            "paused": "Paused",
            "focus_lost": "Focus Lost",
            "waiting_for_focus": "Waiting for Focus",
            "refocus": "Refocusing",
            "panic": "Panic Release",
            "done": "Done",
        }
        header_label = status_labels.get(status, status.replace("_", " ").title())

        session_line = (
            f"{_ANSI_BOLD}{header_label}{_ANSI_RESET}  ·  profile {accent}{self.profile_name}{_ANSI_RESET}"
            f"  ·  tempo {accent}{self.tempo_scale:.2f}×{_ANSI_RESET}  ·  theme {accent}{self.theme_name}{_ANSI_RESET}"
        )

        total_str = format_duration(total)
        current_str = format_duration(current)
        remaining_str = format_duration(max(0.0, total - current))
        time_text = f"{current_str} / {total_str}  ·  ETA {remaining_str}"

        bar_width = max(10, width - 4 - visible_width(time_text) - 2)
        fraction = current / max(total, 0.001)
        filled = min(bar_width, round(fraction * bar_width))
        bar = f"{accent}█{_ANSI_RESET}" * filled + f"{gray}░{_ANSI_RESET}" * (bar_width - filled)

        song_title_line = f"♪ {_ANSI_BOLD}{truncate_cells(self.song_name, width - 8)}{_ANSI_RESET}"
        song_progress_line = f"{bar}  {time_text}"

        divider = f"{divider_c}{'─' * (width - 4)}{_ANSI_RESET}"
        body = [session_line, divider, song_title_line, song_progress_line, divider]

        if self.violations:
            messages = ", ".join(v.message for v in self.violations)
            body.append(f"{yellow}Schedule violations: {messages}{_ANSI_RESET}")
        if degraded:
            body.append(
                f"{yellow}Input path throttled (global hook / Filter Keys?) - playback may stutter; OS-side.{_ANSI_RESET}"
            )

        if self.debug_mode:
            stats = self.renderer.debug_stats()
        else:
            stats = self.renderer.counters_snapshot()
        backend = (
            f"{red}stuck keys: {stats.stuck_keys}{_ANSI_RESET}"
            if stats.stuck_keys > 0
            else f"{green}healthy{_ANSI_RESET}"
        )

        if status == "waiting_for_focus":
            status_line = f"{yellow}Playback has not started yet. Bring Sky window to foreground.{_ANSI_RESET}"
        elif status in {"focus_lost", "paused"}:
            tone = red if status == "focus_lost" else yellow
            status_line = f"{tone}Playback is paused and tracked keys were released.{_ANSI_RESET}"
        elif self.debug_mode:
            status_line = (
                f"backend {backend}  ·  late >2ms:{stats.late_2ms}  >5ms:{stats.late_5ms}  "
                f">10ms:{stats.late_10ms}  ·  active keys: {stats.active_keys}"
            )
        else:
            status_line = f"backend {backend}  ·  late >5ms: {stats.late_5ms}  ·  active keys: {stats.active_keys}"

        if self.debug_mode:
            max_ms = stats.max_lateness_us / 1000.0
            body.append(
                f"{gray}max {max_ms:.1f}ms · p50 {stats.p50_ms:.1f}ms · "
                f"p95 {stats.p95_ms:.1f}ms · σ(onset) {stats.sigma_onset_ms:.1f}ms{_ANSI_RESET}"
            )
            if self.active_policy is not None:
                pol = self.active_policy
                fps = resolve_game_fps(getattr(pol, "fps", None))
                frame_us = getattr(pol, "frame_us", 0) or round(1_000_000 / fps)
                timing_label = f"{fps}fps ({frame_us}us)"
                body.append(f"{gray}Timing: {timing_label}  ·  hold/min: {pol.hold_us}/{pol.min_hold_us}us{_ANSI_RESET}")

        body.append(status_line)
        debug_display = self._debug_hotkey.display if self._debug_hotkey is not None else "F2"
        body.append(
            f"{key_c}{_ANSI_BOLD}F8{_ANSI_RESET} pause  ·  {key_c}{_ANSI_BOLD}F9{_ANSI_RESET} skip  ·  "
            f"{key_c}{_ANSI_BOLD}F10{_ANSI_RESET} quit  ·  "
            f"{key_c}{_ANSI_BOLD}{debug_display}{_ANSI_RESET} {'normal' if self.debug_mode else 'debug'}"
        )
        return body

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
        filled = round(fraction * bar_width)

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
    return app.run() or "quit"

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
        filled = round(fraction * bar_width)

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
                f"max {max_ms:.1f}ms · p50 {stats.p50_ms:.1f}ms · p95 {stats.p95_ms:.1f}ms · σ(onset) {stats.sigma_onset_ms:.1f}ms"
            )
            self.query_one("#debug-lateness", Static).update(lateness_str)
            
            # Line 3: Timing: {fps}fps ({frame_us}us) · hold/min {hold}/{min}us · {profile} {tempo}×
            if self.active_policy is not None:
                fps = resolve_game_fps(getattr(self.active_policy, "fps", None))
                frame_us = getattr(self.active_policy, "frame_us", 0) or round(1_000_000 / fps)
                hold_us = getattr(self.active_policy, "hold_us", 0)
                min_hold = getattr(self.active_policy, "min_hold_us", 0)
            else:
                fps = resolve_game_fps(None)
                frame_us = round(1_000_000 / fps)
                hold_us = 0
                min_hold = 0
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
