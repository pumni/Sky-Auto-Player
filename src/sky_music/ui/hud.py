import shutil
import os
import time
from sky_music.config import resolve_game_fps
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.infrastructure.backend import BackendHealth
from sky_music.infrastructure.hotkeys import PlaybackControls
from sky_music.ui.text_render import (
    ansi_box,
    ansi_gradient_box,
    clamp_terminal_width,
    truncate_cells,
    visible_width,
)

PLAYBACK_FINISHED = "finished"
PLAYBACK_SKIPPED = "skipped"
PLAYBACK_QUIT = "quit"
PLAYBACK_POLL_SECONDS = 0.025
PROGRESS_RENDER_INTERVAL_SECONDS = 0.10


def _hex_to_ansi(hex_color: str) -> str:
    """Convert a CSS hex color (#rrggbb or #rgb) to an ANSI 24-bit fg escape.

    Returns bright-cyan as a safe fallback for malformed input so the HUD
    always renders even when given an unexpected color string.
    """
    c = hex_color.lstrip("#")
    try:
        if len(c) == 3:
            c = "".join(ch * 2 for ch in c)
        r, g, b = int(c[0:2], 16), int(c[2:4], 16), int(c[4:6], 16)
        return f"\033[38;2;{r};{g};{b}m"
    except (ValueError, IndexError):
        return "\033[96m"  # bright cyan fallback


def format_duration(seconds: float) -> str:
    seconds = max(0, int(seconds))
    minutes, sec = divmod(seconds, 60)
    hours, minutes = divmod(minutes, 60)
    if hours:
        return f"{hours}:{minutes:02}:{sec:02}"
    return f"{minutes}:{sec:02}"


class ProgressRenderer:
    def __init__(
        self,
        controls: PlaybackControls | None = None,
        verbose: bool = False,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
        accent_hex: str | None = None,
        theme_name: str = "aurora",
    ) -> None:
        self.controls = controls
        self.verbose = verbose
        self.profile_name = profile_name
        self.tempo_scale = tempo_scale
        self.theme_name = theme_name
        self.last_render_at: float = 0.0

        from sky_music.ui.picker_theme import get_theme_preset
        theme = get_theme_preset(theme_name)

        # Convert hex colors from theme to ANSI escapes
        self._accent_ansi = _hex_to_ansi(accent_hex) if accent_hex else _hex_to_ansi(theme.accent)
        self._muted_ansi = _hex_to_ansi(theme.muted)
        self._success_ansi = _hex_to_ansi(theme.success)
        self._warning_ansi = _hex_to_ansi(theme.warning)
        self._danger_ansi = _hex_to_ansi(theme.danger)
        self._foreground_ansi = _hex_to_ansi(theme.foreground)
        self._divider_ansi = _hex_to_ansi(theme.divider)
        self._key_ansi = _hex_to_ansi(theme.key)
        self._use_gradient_border = theme.use_gradient_border
        self._gradient = theme.gradient
        self._modal_title = theme.modal_title

        # Live timing counters updated by PlaybackEngine
        self.late_2ms: int = 0
        self.late_5ms: int = 0
        self.late_10ms: int = 0
        self.max_lateness_us: int = 0

        self.run_id: str = ""
        self.last_lines_printed: int = 0
        self._initialized: bool = False
        self.input_path_degraded: bool = False
        self.active_policy: FrameTimingPolicy | None = None

    def update_counters(self, lateness_us: int, kind: str = "down") -> None:
        """Called by PlaybackEngine after each key action to update live timing counters.
        Only onset (key-down) events update the main late_* counters; release events are
        tracked separately to avoid inflating the timing display with min_hold deferral."""
        clamped = max(0, lateness_us)
        if kind != "down":
            return  # release lateness is deferred by design; skip main counters
        if clamped > 10000:
            self.late_10ms += 1
            self.late_5ms += 1
            self.late_2ms += 1
        elif clamped > 5000:
            self.late_5ms += 1
            self.late_2ms += 1
        elif clamped > 2000:
            self.late_2ms += 1
        if clamped > self.max_lateness_us:
            self.max_lateness_us = clamped

    def _control_hint(self, key: str, label: str, key_color: str, bold: str, reset: str) -> str:
        return f"{key_color}{bold}{key}{reset} {label}"

    def _build_controls_line(self, status: str, width: int, key_color: str, bold: str, muted: str, reset: str) -> str:
        if self.controls is None or not self.controls.enabled:
            return f"{muted}hotkeys disabled{reset}"

        def hint(key: str, label: str) -> str:
            return self._control_hint(key, label, key_color, bold, reset)

        if status == "waiting_for_focus":
            full = [
                hint(self.controls.refocus.display, "refocus"),
                hint(self.controls.quit.display, "quit"),
                hint("D", "dry-run"),
                hint(self.controls.panic.display, "panic"),
            ]
            compact = [full[0], full[1], full[3]]
            minimal = [full[0], full[1]]
        elif status == "focus_lost":
            full = [
                hint(self.controls.refocus.display, "refocus"),
                hint(self.controls.quit.display, "quit"),
                hint(self.controls.panic.display, "panic"),
            ]
            compact = full
            minimal = [full[0], full[1]]
        elif status == "paused":
            full = [
                hint(self.controls.pause.display, "resume"),
                hint(self.controls.skip.display, "skip"),
                hint(self.controls.quit.display, "quit"),
                hint(self.controls.refocus.display, "refocus"),
                hint(self.controls.panic.display, "panic"),
            ]
            compact = [full[0], full[1], full[2], full[4]]
            minimal = [full[0], full[1], full[2]]
        else:
            full = [
                hint(self.controls.pause.display, "pause"),
                hint(self.controls.skip.display, "skip"),
                hint(self.controls.quit.display, "quit"),
                hint(self.controls.refocus.display, "refocus"),
                hint(self.controls.panic.display, "panic"),
            ]
            compact = [full[0], full[1], full[2], full[4]]
            minimal = [full[0], full[1], full[2]]

        pieces = full if width >= 90 else compact if width >= 70 else minimal
        return "  ·  ".join(pieces)

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
        now = time.perf_counter()
        if not force and now - self.last_render_at < PROGRESS_RENDER_INTERVAL_SECONDS:
            return

        self.last_render_at = now
        self.input_path_degraded = self.input_path_degraded or input_path_degraded
        
        if not self.run_id:
            self.run_id = time.strftime('%Y%m%d-%H%M%S')
            
        terminal_width = shutil.get_terminal_size((100, 20)).columns
        width = clamp_terminal_width(terminal_width)

        # ANSI Colors — fixed semantic colours
        ANSI_RESET = "\033[0m"
        ANSI_BOLD = "\033[1m"
        ANSI_ACCENT = self._accent_ansi   # theme-synced border / info colour
        ANSI_GREEN = self._success_ansi   # theme-synced success colour
        ANSI_YELLOW = self._warning_ansi  # theme-synced warning colour
        ANSI_RED = self._danger_ansi      # theme-synced error colour
        ANSI_GRAY = self._muted_ansi      # theme-synced muted/gray colour
        ANSI_DIVIDER = self._divider_ansi
        ANSI_KEY = self._key_ansi
        
        # 1. Resolve header label & status color
        if status == "playing":
            header_label = "Playing"
            status_color = ANSI_ACCENT
        elif status == "paused":
            header_label = "Paused"
            status_color = ANSI_YELLOW
        elif status == "focus_lost":
            header_label = "Focus Lost"
            status_color = ANSI_RED
        elif status == "waiting_for_focus":
            header_label = "Waiting for Focus"
            status_color = ANSI_YELLOW
        elif status == "refocus":
            header_label = "Refocusing"
            status_color = ANSI_ACCENT
        elif status == "panic":
            header_label = "Panic Release"
            status_color = ANSI_YELLOW  # panic release is warning color
        elif status == "done":
            header_label = "Done"
            status_color = ANSI_ACCENT
        else:
            header_label = status.replace("_", " ").title()
            status_color = ANSI_ACCENT

        # Header session status line
        session_line = f"{ANSI_BOLD}{header_label}{ANSI_RESET}  ·  profile {ANSI_ACCENT}{self.profile_name}{ANSI_RESET}  ·  tempo {ANSI_ACCENT}{self.tempo_scale:.2f}×{ANSI_RESET}  ·  theme {ANSI_ACCENT}{self.theme_name}{ANSI_RESET}"

        # 2. Song progress with remaining time (ETA)
        total_time_str = format_duration(total)
        current_time_str = format_duration(current)
        remaining = max(0.0, total - current)
        remaining_str = format_duration(remaining)
        time_text = f"{current_time_str} / {total_time_str}  ·  ETA {remaining_str}"

        # The inner width is width - 4
        bar_width = max(10, width - 4 - visible_width(time_text) - 2)
        fraction = current / max(total, 0.001)
        filled = min(bar_width, round(fraction * bar_width))
        bar = f"{ANSI_ACCENT}█{ANSI_RESET}" * filled + f"{ANSI_GRAY}░{ANSI_RESET}" * (bar_width - filled)

        song_title_line = f"♪ {ANSI_BOLD}{truncate_cells(song_name, width - 8)}{ANSI_RESET}"
        song_progress_line = f"{bar}  {time_text}"

        # 3. Backend status
        active_keys = 0
        failed_releases = 0
        if backend_health is not None:
            active_keys = backend_health.active_count
            failed_releases = backend_health.failed_release_count
            
        backend_status = f"{ANSI_RED}stuck keys: {failed_releases}{ANSI_RESET}" if failed_releases > 0 else f"{ANSI_GREEN}healthy{ANSI_RESET}"

        if status == "waiting_for_focus":
            status_line = f"{ANSI_YELLOW}Playback has not started yet. Bring Sky window to foreground.{ANSI_RESET}"
        elif status == "focus_lost":
            status_line = f"{ANSI_RED}Playback is paused and tracked keys were released.{ANSI_RESET}"
        elif status == "paused":
            status_line = f"{ANSI_YELLOW}Playback is paused and tracked keys were released.{ANSI_RESET}"
        else:
            if self.verbose:
                status_line = f"backend {backend_status}  ·  late >2ms:{self.late_2ms}  >5ms:{self.late_5ms}  >10ms:{self.late_10ms}  ·  active keys: {active_keys}"
            else:
                status_line = f"backend {backend_status}  ·  late >5ms: {self.late_5ms}  ·  active keys: {active_keys}"

        # 4. Controls line
        controls_line = self._build_controls_line(status, width, ANSI_KEY, ANSI_BOLD, ANSI_GRAY, ANSI_RESET)

        # Assemble the single HUD card lines
        divider = f"{ANSI_DIVIDER}{'─' * (width - 4)}{ANSI_RESET}"
        hud_body_lines = [
            session_line,
            divider,
            song_title_line,
            song_progress_line,
            divider,
        ]

        # Insert input path warning if degraded
        if self.input_path_degraded:
            hud_body_lines.append(
                f"{ANSI_YELLOW}Input path throttled (global hook / Filter Keys?) - playback may stutter; OS-side.{ANSI_RESET}"
            )

        # Insert timing info if verbose
        if self.verbose and self.active_policy is not None:
            pol = self.active_policy
            fps = resolve_game_fps(getattr(pol, "fps", None))
            frame_us = getattr(pol, "frame_us", 0) or round(1_000_000 / fps)
            frame_label = f"{frame_us}us"
            fps_label = f"{fps}fps"
            hold_info = f"hold/min: {pol.hold_us}/{pol.min_hold_us}us"
            timing_line = f"{ANSI_GRAY}Timing: {fps_label} ({frame_label})  ·  {hold_info}{ANSI_RESET}"
            hud_body_lines.append(timing_line)

        hud_body_lines.append(status_line)
        hud_body_lines.append(controls_line)

        # If healthy state and theme gradient is enabled, draw gradient border
        if self._use_gradient_border and status in {"playing", "done", "refocus"}:
            hud_lines = ansi_gradient_box(
                "SKY MUSIC HELPER",
                hud_body_lines,
                width=width,
                gradient=self._gradient,
                title_color=self._modal_title,
            )
        else:
            hud_lines = ansi_box("SKY MUSIC HELPER", hud_body_lines, width=width, border_color=status_color)
        
        if self._initialized and self.last_lines_printed > 0:
            print(f"\033[{self.last_lines_printed}A", end="", flush=True)
            
        self._initialized = True
        self.last_lines_printed = len(hud_lines)
        
        output = "\n".join(hud_lines) + "\n"
        print(output, end="", flush=True)

    def finish(self, message: str = "") -> None:
        """Erase the live HUD from the terminal. The caller is responsible for
        redrawing (e.g. picker loop) — no status message is printed here."""
        if self._initialized and self.last_lines_printed > 0:
            print(f"\033[{self.last_lines_printed}A", end="", flush=True)
            for _ in range(self.last_lines_printed):
                print("\r\033[K")
            print(f"\033[{self.last_lines_printed}A", end="", flush=True)
        self._initialized = False
        self.last_lines_printed = 0

def clear_terminal() -> None:
    import subprocess
    subprocess.run('cls' if os.name == 'nt' else 'clear', shell=True)
