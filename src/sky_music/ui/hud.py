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
from sky_music.infrastructure.hotkeys import PlaybackControls
from sky_music.orchestration.native_models import (
    BackendHealth,
    PlaybackOutcome,
)
from sky_music.ui.picker_theme import ThemePreset, get_theme_preset
from sky_music.ui.playback_notices import PlaybackNoticeLedger
from sky_music.ui.playback_view_model import build_playback_hud_view
from sky_music.ui.text_render import (
    clamp_terminal_width,
    truncate_cells,
)

PLAYBACK_FINISHED = PlaybackOutcome.FINISHED
PLAYBACK_QUIT = PlaybackOutcome.QUIT
PLAYBACK_SKIPPED = PlaybackOutcome.SKIPPED

PLAYBACK_POLL_SECONDS = 0.025
PROGRESS_RENDER_INTERVAL_SECONDS = 0.10


def format_duration(seconds: float) -> str:
    seconds = max(0, int(seconds))
    minutes, sec = divmod(seconds, 60)
    hours, minutes = divmod(minutes, 60)
    if hours:
        return f"{hours}:{minutes:02}:{sec:02}"
    return f"{minutes}:{sec:02}"


def _theme_styles(preset: ThemePreset, accent_override: str | None = None) -> dict[str, Style]:
    """Build a Rich Style map from a typed *ThemePreset* design token object."""
    accent = accent_override or preset.accent
    return {
        "accent": Style.parse(accent),
        "foreground": Style.parse(preset.foreground),
        "muted": Style.parse(preset.muted),
        "success": Style.parse(preset.success),
        "warning": Style.parse(preset.warning),
        "danger": Style.parse(preset.danger),
        "divider": Style.parse(preset.divider),
        "key": Style.parse(preset.key),
        "modal_title": Style.parse(preset.modal_title),
    }

class ProgressRenderer:
    def __init__(
        self,
        controls: PlaybackControls | None = None,
        verbose: bool = False,
        hold_label: str = "hold 1.00f",
        hold_frames: float = 1.0,
        tempo_scale: float = 1.0,
        accent_hex: str | None = None,
        theme_name: str = "aurora",
        schedule_warnings: tuple[str, ...] = (),
    ) -> None:
        self.controls = controls
        self.verbose = verbose
        self.hold_label = hold_label
        self.hold_frames = hold_frames
        self.tempo_scale = tempo_scale
        self.theme_name = theme_name
        self._notice_ledger = PlaybackNoticeLedger(schedule_warnings)
        self.last_render_at: float = 0.0

        preset = get_theme_preset(theme_name)
        self._styles = _theme_styles(preset, accent_override=accent_hex)
        # Use the first gradient stop for the progress bar; fall back to accent.
        self._gradient = (
            Style.parse(preset.gradient[0]) if preset.gradient else self._styles["accent"]
        )

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

    def update_counters_batch(self, counters) -> None:
        self.max_lateness_us = counters.max_lateness_us
        self.late_2ms = counters.late_2ms
        self.late_5ms = counters.late_5ms
        self.late_10ms = counters.late_10ms

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
        sendinput_path_degraded: bool = False,
        bookkeeping_degraded: bool = False,
        wait_path_degraded: bool = False,
        wait_backend_failures: int = 0,
        wait_clock_failures: int = 0,
        recovered_zero_progress_but_late: int = 0,
        backend_health: BackendHealth | None = None,
    ) -> None:
        now = time.perf_counter()
        if not force and now - self.last_render_at < PROGRESS_RENDER_INTERVAL_SECONDS:
            return

        self.last_render_at = now
        self.input_path_degraded = bool(input_path_degraded)

        if not self.run_id:
            self.run_id = time.strftime("%Y%m%d-%H%M%S")

        terminal_width = shutil.get_terminal_size((100, 20)).columns
        width = clamp_terminal_width(terminal_width)

        styles = self._styles

        # Resolve header label & status style
        status_colors: dict[str, Style] = {
            "playing": styles["accent"],
            "paused": styles["warning"],
            "focus_lost": styles["danger"],
            "waiting_for_focus": styles["warning"],
            "refocus": styles["accent"],
            "panic": styles["warning"],
            "done": styles["accent"],
        }

        view = build_playback_hud_view(
            current_seconds=current,
            total_seconds=total,
            song_name=song_name,
            status=status,
            input_path_degraded=self.input_path_degraded,
            sendinput_path_degraded=sendinput_path_degraded,
            bookkeeping_degraded=bookkeeping_degraded,
            wait_path_degraded=wait_path_degraded,
            backend_health=backend_health,
            late_2ms=self.late_2ms,
            late_5ms=self.late_5ms,
            late_10ms=self.late_10ms,
            max_lateness_us=self.max_lateness_us,
        )
        header_label = view.status_label
        status_style = status_colors.get(status, styles["accent"])

        # Session info line
        session_line = Text.assemble(
            (header_label, Style.combine([Style(bold=True), status_style])),
            "  ·  ",
            (self.hold_label, styles["accent"]),
            "  ·  tempo ",
            (f"{self.tempo_scale:.2f}×", styles["accent"]),
            "  ·  theme ",
            (self.theme_name, styles["accent"]),
            "  ·  dispatch ",
            ("Rust native", styles["accent"]),
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

        current_time_str = format_duration(view.current_seconds)
        total_time_str = format_duration(view.total_seconds)
        remaining_str = format_duration(view.eta_seconds)
        time_text = Text(f"{current_time_str} / {total_time_str}  ·  ETA {remaining_str}", style=styles["foreground"])

        # Backend status line
        active_keys = view.backend.active_keys
        failed_releases = view.backend.stuck_keys
        keys_dropped = view.backend.keys_dropped
        chord_splits = view.backend.chord_split_events

        notice_state = self._notice_ledger.update(
            input_path_degraded=self.input_path_degraded,
            sendinput_path_degraded=sendinput_path_degraded,
            bookkeeping_degraded=bookkeeping_degraded,
            wait_path_degraded=wait_path_degraded,
            keys_dropped=keys_dropped,
            chord_split_events=chord_splits,
            chords_rejected=int(getattr(backend_health, "chords_rejected", 0) or 0),
            authored_keys_rejected=int(
                getattr(backend_health, "authored_keys_rejected", 0) or 0
            ),
            sendinput_partial_events=int(
                getattr(backend_health, "sendinput_partial_events", 0) or 0
            ),
            sendinput_zero_progress_failures=int(
                getattr(backend_health, "sendinput_zero_progress_failures", 0) or 0
            ),
            recovered_zero_progress_but_late=recovered_zero_progress_but_late,
            wait_backend_failures=wait_backend_failures,
            wait_clock_failures=wait_clock_failures,
        )

        view = build_playback_hud_view(
            current_seconds=view.current_seconds,
            total_seconds=view.total_seconds,
            song_name=view.song_name,
            status=view.status,
            input_path_degraded=self.input_path_degraded,
            sendinput_path_degraded=sendinput_path_degraded,
            bookkeeping_degraded=bookkeeping_degraded,
            wait_path_degraded=wait_path_degraded,
            backend_health=backend_health,
            late_2ms=self.late_2ms,
            late_5ms=self.late_5ms,
            late_10ms=self.late_10ms,
            max_lateness_us=self.max_lateness_us,
            notices=notice_state.notices,
        )

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

        # keys_dropped: note-on keys OS did not inject (no-retry policy). Show always in
        # verbose; in compact mode only when > 0 so a healthy run stays uncluttered.
        dropped_parts: list[str | tuple[str, Style]] = []
        if self.verbose or keys_dropped > 0:
            drop_style = (
                Style.combine([styles["danger"], Style(bold=True)])
                if keys_dropped > 0
                else styles["muted"]
            )
            dropped_parts = ["  ·  dropped: ", (str(keys_dropped), drop_style)]
            if self.verbose and chord_splits > 0:
                dropped_parts.extend(["  splits: ", (str(chord_splits), styles["warning"])])

        if status in status_descriptions:
            status_line = status_descriptions[status]
        elif self.verbose:
            status_line = Text.assemble(
                "backend ", backend_status_text,
                "  ·  late >2ms:", str(self.late_2ms),
                "  >5ms:", str(self.late_5ms),
                "  >10ms:", str(self.late_10ms),
                "  ·  active keys: ", str(active_keys),
                *dropped_parts,
            )
        else:
            status_line = Text.assemble(
                "backend ", backend_status_text,
                "  ·  late >5ms: ", str(self.late_5ms),
                "  ·  active keys: ", str(active_keys),
                *dropped_parts,
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

        panel_content.extend(
            Text(
                notice.message,
                style=styles["danger"] if notice.severity == "danger" else styles["warning"],
            )
            for notice in notice_state.notices
        )

        # Timing info (verbose)
        if self.verbose and self.active_policy is not None:
            pol = self.active_policy
            fps = resolve_game_fps(getattr(pol, "fps", None))
            frame_us = getattr(pol, "frame_us", 0) or round(1_000_000 / fps)
            frame_label = f"{frame_us}us"
            fps_label = f"{fps}fps"
            hold_info = f"Hold: {pol.hold_frames:.2f} frames  ·  Effective: {pol.hold_us / 1000:.3f} ms"
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
