import os
import shutil
import time

from rich.console import Console, Group, RenderableType
from rich.live import Live
from rich.panel import Panel
from rich.progress import BarColumn, Progress, TaskID
from rich.style import Style
from rich.table import Table
from rich.text import Text

from sky_music.config import resolve_game_fps
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.infrastructure.backend import BackendHealth
from sky_music.infrastructure.hotkeys import PlaybackControls
from sky_music.ui.text_render import (
    clamp_terminal_width,
    truncate_cells,
)

PLAYBACK_FINISHED = "finished"
PLAYBACK_SKIPPED = "skipped"
PLAYBACK_QUIT = "quit"
PLAYBACK_POLL_SECONDS = 0.025
PROGRESS_RENDER_INTERVAL_SECONDS = 0.10


def format_duration(seconds: float) -> str:
    seconds = max(0, int(seconds))
    minutes, sec = divmod(seconds, 60)
    hours, minutes = divmod(minutes, 60)
    if hours:
        return f"{hours}:{minutes:02}:{sec:02}"
    return f"{minutes}:{sec:02}"


def _theme_styles(theme_tokens: dict[str, str]) -> dict[str, Style]:
    return {
        "accent": Style.parse(theme_tokens["accent"]),
        "foreground": Style.parse(theme_tokens["foreground"]),
        "muted": Style.parse(theme_tokens["muted"]),
        "success": Style.parse(theme_tokens["success"]),
        "warning": Style.parse(theme_tokens["warning"]),
        "danger": Style.parse(theme_tokens["danger"]),
        "divider": Style.parse(theme_tokens["divider"]),
        "key": Style.parse(theme_tokens["key"]),
        "modal_title": Style.parse(theme_tokens["modal_title"]),
        "header_lead": Style.parse(theme_tokens.get("header_lead", theme_tokens["accent"])),
    }


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

        from sky_music.ui.textual_app.theme_css import TEXTUAL_THEME_TOKENS

        theme_tokens_raw = TEXTUAL_THEME_TOKENS.get(
            theme_name.casefold(), TEXTUAL_THEME_TOKENS["aurora"]
        )
        tokens: dict[str, str] = {}
        for key in (
            "accent", "foreground", "muted", "success", "warning",
            "danger", "divider", "key", "modal_title",
        ):
            val = getattr(theme_tokens_raw, key, None)
            if val is not None:
                tokens[key] = val

        if accent_hex:
            tokens["accent"] = accent_hex

        self._styles = _theme_styles(tokens)
        self._gradient = self._styles["accent"]
        raw_gradient = getattr(theme_tokens_raw, "gradient", None)
        if raw_gradient and isinstance(raw_gradient, tuple) and len(raw_gradient) > 0:
            self._gradient = Style.parse(raw_gradient[0])

        self._console: Console | None = None
        self._live: Live | None = None
        self._progress: Progress | None = None
        self._task_id: TaskID | None = None

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
        clamped = max(0, lateness_us)
        if kind != "down":
            return
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

    def _build_controls_line(self, status: str, width: int) -> Text:
        key_style = self._styles["key"]
        muted_style = self._styles["muted"]

        if self.controls is None or not self.controls.enabled:
            return Text("hotkeys disabled", style=muted_style)

        def hint(key: str, label: str) -> Text:
            return Text.assemble((key, key_style), f" {label}")

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
        sep = Text("  ·  ", style=muted_style)
        result = Text("")
        for i, piece in enumerate(pieces):
            if i:
                result.append(sep)
            result.append(piece)
        return result

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
            self.run_id = time.strftime("%Y%m%d-%H%M%S")

        terminal_width = shutil.get_terminal_size((100, 20)).columns
        width = clamp_terminal_width(terminal_width)

        styles = self._styles

        # Resolve header label & status style
        status_labels: dict[str, str] = {
            "playing": "Playing",
            "paused": "Paused",
            "focus_lost": "Focus Lost",
            "waiting_for_focus": "Waiting for Focus",
            "refocus": "Refocusing",
            "panic": "Panic Release",
            "done": "Done",
        }
        status_colors: dict[str, Style] = {
            "playing": styles["accent"],
            "paused": styles["warning"],
            "focus_lost": styles["danger"],
            "waiting_for_focus": styles["warning"],
            "refocus": styles["accent"],
            "panic": styles["warning"],
            "done": styles["accent"],
        }

        header_label = status_labels.get(status, status.replace("_", " ").title())
        status_style = status_colors.get(status, styles["accent"])

        # Session info line
        session_line = Text.assemble(
            (header_label, Style.combine([Style(bold=True), status_style])),
            "  ·  profile ",
            (self.profile_name, styles["accent"]),
            "  ·  tempo ",
            (f"{self.tempo_scale:.2f}×", styles["accent"]),
            "  ·  theme ",
            (self.theme_name, styles["accent"]),
        )

        # Song title
        song_title = Text.assemble(
            "♪ ",
            (truncate_cells(song_name, width - 8), Style(bold=True)),
        )

        # Progress bar + time
        if self._progress is None:
            self._progress = Progress(
                BarColumn(
                    bar_width=None,
                    style=styles["muted"],
                    complete_style=self._gradient,
                    finished_style=styles["success"],
                ),
            )
            self._task_id = self._progress.add_task("playback", total=max(total, 0.001))

        total_safe = max(total, 0.001)
        if self._task_id is not None:
            self._progress.update(self._task_id, total=total_safe, completed=min(current, total_safe))

        current_time_str = format_duration(current)
        total_time_str = format_duration(total)
        remaining = max(0.0, total - current)
        remaining_str = format_duration(remaining)
        time_text = Text(f"{current_time_str} / {total_time_str}  ·  ETA {remaining_str}", style=styles["foreground"])

        # Backend status line
        active_keys = 0
        failed_releases = 0
        if backend_health is not None:
            active_keys = backend_health.active_count
            failed_releases = backend_health.failed_release_count

        if failed_releases > 0:
            backend_status_text = Text.assemble(
                ("stuck keys: ", styles["danger"]),
                (str(failed_releases), Style.combine([styles["danger"], Style(bold=True)])),
            )
        else:
            backend_status_text = Text("healthy", style=styles["success"])

        status_descriptions: dict[str, Text] = {
            "waiting_for_focus": Text("Playback has not started yet. Bring Sky window to foreground.", style=styles["warning"]),
            "focus_lost": Text("Playback is paused and tracked keys were released.", style=styles["danger"]),
            "paused": Text("Playback is paused and tracked keys were released.", style=styles["warning"]),
        }

        if status in status_descriptions:
            status_line = status_descriptions[status]
        elif self.verbose:
            status_line = Text.assemble(
                "backend ", backend_status_text,
                "  ·  late >2ms:", str(self.late_2ms),
                "  >5ms:", str(self.late_5ms),
                "  >10ms:", str(self.late_10ms),
                "  ·  active keys: ", str(active_keys),
            )
        else:
            status_line = Text.assemble(
                "backend ", backend_status_text,
                "  ·  late >5ms: ", str(self.late_5ms),
                "  ·  active keys: ", str(active_keys),
            )

        # Controls line
        controls_line = self._build_controls_line(status, width)

        # Divider
        divider = "─" * (width - 4)
        divider_text = Text(divider, style=styles["divider"])

        # Assemble panel content
        panel_content: list[RenderableType] = [
            session_line,
            divider_text,
            song_title,
        ]

        # Progress bar + time in one row
        progress_table = Table.grid(padding=(0, 0))
        progress_table.add_column(ratio=1)
        progress_table.add_column(justify="right", no_wrap=True)
        progress_table.add_row(self._progress, time_text)
        panel_content.append(progress_table)
        panel_content.append(divider_text)

        # Input path warning
        if self.input_path_degraded:
            panel_content.append(
                Text("Input path throttled (global hook / Filter Keys?) - playback may stutter; OS-side.", style=styles["warning"])
            )

        # Timing info (verbose)
        if self.verbose and self.active_policy is not None:
            pol = self.active_policy
            fps = resolve_game_fps(getattr(pol, "fps", None))
            frame_us = getattr(pol, "frame_us", 0) or round(1_000_000 / fps)
            frame_label = f"{frame_us}us"
            fps_label = f"{fps}fps"
            hold_info = f"hold/min: {pol.hold_us}/{pol.min_hold_us}us"
            timing_line = Text(
                f"Timing: {fps_label} ({frame_label})  ·  {hold_info}",
                style=styles["muted"],
            )
            panel_content.append(timing_line)

        panel_content.append(status_line)
        panel_content.append(controls_line)

        # Border style: gradient for healthy states, status color otherwise
        if status in {"playing", "done", "refocus"}:
            border_style = self._gradient
        else:
            border_style = status_style

        panel = Panel(
            Group(*panel_content),
            title="SKY MUSIC HELPER",
            title_align="left",
            border_style=border_style,
            padding=(0, 2),
        )

        if self._live is None:
            self._console = Console()
            self._live = Live(
                panel,
                console=self._console,
                refresh_per_second=10,
                vertical_overflow="visible",
            )
            self._live.start()
            self._initialized = True
        else:
            self._live.update(panel)

    def finish(self, _message: str = "") -> None:
        if self._live is not None:
            self._live.stop()
            self._live = None
        self._progress = None
        self._task_id = None
        self._console = None
        self._initialized = False
        self.last_lines_printed = 0


def clear_terminal() -> None:
    import subprocess
    subprocess.run("cls" if os.name == "nt" else "clear", shell=True)