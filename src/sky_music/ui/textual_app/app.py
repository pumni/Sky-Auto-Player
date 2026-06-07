"""Textual song picker backend."""

from __future__ import annotations

try:
    import importlib.metadata
    VERSION = importlib.metadata.version("sky-player")
except Exception:
    VERSION = "0.1.0"

from dataclasses import dataclass
from pathlib import Path
from typing import Any, TYPE_CHECKING

from rapidfuzz import fuzz, process
from rich.text import Text
from textual import events
from textual.app import App, ComposeResult
from sky_music.ui.textual_app.playback_app import CountdownScreen, PlaybackScreen, SnapshotRenderer
from sky_music.ui.textual_app.playback_controller import prepare_playback, rebuild_with, PlaybackPlan, PlaybackError
from textual.binding import Binding
from textual.containers import Container
from textual.reactive import reactive
from textual.widgets import DataTable, Input

if TYPE_CHECKING:
    from sky_music.infrastructure.hotkeys import PlaybackControls

from sky_music.config import (
    AppConfig,
    canonical_profile_name,
    load_config,
    persist_calibration_defaults,
    persist_default_fps,
    persist_default_profile,
    persist_default_tempo,
    save_config,
)
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.ui.picker import FPS_OPTIONS, PROFILES_INFO, TEMPO_OPTIONS, SongPickerResult
from sky_music.ui.picker_helpers import get_song_choices, save_theme
from sky_music.ui.picker_metadata import (
    clear_metadata_cache,
    peek_cached_song_ui_metadata,
)
from sky_music.ui.picker_theme import (
    THEME_PRESETS,
    remove_accents,
    pad_text,
)
from sky_music.ui.textual_app.keymap import COMMANDS
from sky_music.ui.textual_app.display_widgets import DetailPanel, GradientHeader
from sky_music.ui.textual_app.modals import CommandModal, InfoModal, OptionModal, PickerOption
from sky_music.ui.textual_app.theme_css import APP_CSS, TEXTUAL_THEME_TOKENS, TextualThemeTokens
from sky_music.ui.textual_app.widgets import CustomFooter
from sky_music.ui.textual_app.workers import MetadataCoordinator
from sky_music.infrastructure.background import BackgroundScope
from sky_music.ui.textual_app.renderers import (
    _title_cell,
    _risk_cell,
    _metadata_cells,
    build_empty_detail_text,
    build_detail_text,
)

FUZZY_SCORE_CUTOFF = 60.0


@dataclass(frozen=True, slots=True)
class SongChoice:
    path: Path
    search_key: str


def rank_song_choices(
    choices: list[SongChoice],
    query: str,
    *,
    score_cutoff: float = FUZZY_SCORE_CUTOFF,
) -> list[SongChoice]:
    normalized = remove_accents(query).casefold().strip()
    if not normalized:
        return list(choices)

    if len(normalized) == 1:
        return [choice for choice in choices if normalized in choice.search_key]

    choices_by_index = {index: choice.search_key for index, choice in enumerate(choices)}
    matches = process.extract(
        normalized,
        choices_by_index,
        scorer=fuzz.WRatio,
        score_cutoff=score_cutoff,
        limit=None,
    )

    scores: dict[int, float] = {int(index): float(score) for _key, score, index in matches}
    for index, choice in enumerate(choices):
        if normalized in choice.search_key:
            scores[index] = max(scores.get(index, 0.0), 100.0)

    ranked_indices = sorted(scores, key=lambda index: (-scores[index], index))
    return [choices[index] for index in ranked_indices]


class SongTable(DataTable[str]):
    """DataTable wrapper for song picker rows."""

    BINDINGS = [
        # Secondary actions — functional but hidden from the footer bar to reduce
        # visual clutter. Users discover them via the Commands modal (/).
        Binding("p", "open_profile", "Profile", priority=True, show=False),
        Binding("t", "open_tempo", "Tempo", priority=True, show=False),
        Binding("f", "open_fps", "FPS", priority=True, show=False),
        Binding("y", "open_theme", "Theme", priority=True, show=False),
        Binding("v", "toggle_preview", "Details", priority=True, show=False),
        Binding("d", "toggle_dry_run", "Dry-run", priority=True, show=False),
        Binding("h", "toggle_hud", "HUD", priority=True, show=False),
        Binding("f3", "toggle_telemetry", "Telemetry", priority=True, show=False),
        Binding("ctrl+r", "reload_songs", "Reload", priority=True, show=False),
    ]

    def action_open_commands(self) -> None:
        self.app.action_open_commands()

    def action_open_profile(self) -> None:
        self.app.action_open_profile()

    def action_open_tempo(self) -> None:
        self.app.action_open_tempo()

    def action_open_fps(self) -> None:
        self.app.action_open_fps()

    def action_open_theme(self) -> None:
        self.app.action_open_theme()

    def action_toggle_preview(self) -> None:
        self.app.action_toggle_preview()

    def action_toggle_dry_run(self) -> None:
        self.app.action_toggle_dry_run()

    def action_toggle_hud(self) -> None:
        self.app.action_toggle_hud()

    def action_toggle_telemetry(self) -> None:
        self.app.action_toggle_telemetry()

    def action_reload_songs(self) -> None:
        self.app.action_reload_songs()


@dataclass(frozen=True, slots=True)
class CalibrationChoice:
    profile_name: str
    tempo_scale: float
    fps: int


class SearchInput(Input):
    """Custom search input that shifts focus back to the song table on escape key."""
    def on_key(self, event: events.Key) -> None:
        if event.key == "escape":
            event.stop()
            try:
                table = self.app.query_one("#songs", SongTable)
                table.focus()
            except Exception:
                pass


class SkyPickerApp(App[SongPickerResult | None]):
    """Textual picker app."""

    # Use the terminal's own default background (no painted app surface),
    # so the picker blends into the user's terminal theme like a native CLI.
    ansi_color = True

    CSS = APP_CSS + "\n" + """
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
#countdown-timer {
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

    BINDINGS = [
        ("q", "cancel", "Quit"),
        ("escape", "cancel", "Cancel"),
        ("enter", "confirm", "Play"),
        ("/", "open_commands", "Commands"),
    ]

    query: reactive[str] = reactive("", init=False)

    def __init__(
        self,
        *,
        theme_name: str | None = None,
        background_mode: str | None = None,
        initial_profile: str = "balanced",
        initial_tempo: float = 1.0,
        initial_fps: int | None = None,
        initial_dry_run: bool = False,
        scan_code_mode: str = "physical",
        cfg: AppConfig | None = None,
        unified_mode: bool = False,
        controls: PlaybackControls | None = None,
        countdown_seconds: int = 3,
    ) -> None:
        super().__init__()
        self.unified_mode = unified_mode
        self.controls = controls
        self.countdown_seconds = countdown_seconds
        self.theme_name = theme_name
        self.profile_name = canonical_profile_name(initial_profile)
        self.tempo_scale = initial_tempo
        self.fps = initial_fps
        self.dry_run = initial_dry_run
        self.scan_code_mode = scan_code_mode
        self.cfg = cfg or load_config()
        self.verbose_hud = self.cfg.verbose_hud
        self.telemetry_enabled = self.cfg.telemetry_enabled_by_default
        self.active_theme = self._normalize_theme_name(theme_name or self.cfg.theme)
        self.background_mode = self._normalize_background_mode(background_mode or self.cfg.ui_background_mode)
        self.preview_visible = True
        self.show_notes = True
        self.show_risk = True
        self.show_suggested = True
        self._transitioning_to_playback = False
        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )
        self.choices: list[SongChoice] = []
        self.filtered: list[SongChoice] = []
        self._marked_row_key: object | None = None
        self.picker_scope = BackgroundScope(phase="picker")
        self.metadata = self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg))
        self._search_timer = None

    @staticmethod
    def _normalize_theme_name(theme_name: str | None) -> str:
        requested = (theme_name or "aurora").casefold()
        if requested in THEME_PRESETS:
            return requested
        return "aurora"

    @staticmethod
    def _normalize_background_mode(background_mode: str | None) -> str:
        requested = (background_mode or "transparent").casefold()
        if requested in {"transparent", "painted"}:
            return requested
        return "transparent"

    @property
    def _theme_tokens(self) -> TextualThemeTokens:
        return TEXTUAL_THEME_TOKENS[self.active_theme]

    @property
    def _theme_class(self) -> str:
        return f"theme-{self.active_theme}"

    def _apply_theme_class(self) -> None:
        for name in THEME_PRESETS:
            self.screen.remove_class(f"theme-{name}")
        for mode in ("transparent", "painted"):
            self.screen.remove_class(f"background-{mode}")
        self.screen.add_class(self._theme_class)
        self.screen.add_class(f"background-{self.background_mode}")
        t = self._theme_tokens
        try:
            self.query_one("#appbar", GradientHeader).set_theme(
                t.gradient, t.foreground, t.detail, t.foreground, lead=t.header_lead
            )
        except Exception:
            pass
        try:
            self.query_one(CustomFooter).set_theme(t.key, t.muted)
        except Exception:
            pass

    def compose(self) -> ComposeResult:
        with Container(id="root"):
            yield GradientHeader("♪ Sky Player", "precision music player", version=f"v{VERSION}", id="appbar")  # tagline updated in on_mount
            search = SearchInput(placeholder="Search songs…", id="search")
            search.border_title = "Search"
            yield search
            table = SongTable(id="songs", cursor_type="row")
            table.border_title = "Songs"
            table.add_column(" ", key="marker", width=2)
            table.add_column("Title", key="title", width=42)
            table.add_column("Time", key="time", width=8)
            table.add_column("Notes", key="notes", width=8)
            table.add_column("Risk", key="risk", width=8)
            table.add_column("Suggested", key="suggested", width=16)
            yield table
            detail = DetailPanel(id="detail")
            detail.border_title = "Details"
            yield detail
            yield CustomFooter()

    def on_mount(self) -> None:
        self._apply_theme_class()
        paths = get_song_choices(force_refresh=True)
        self.choices = [
            SongChoice(path=path, search_key=remove_accents(path.stem).casefold())
            for path in paths
        ]
        self.filtered = rank_song_choices(self.choices, self.query)
        self._render_status()
        self._render_table()
        self._render_detail()
        self.set_focus(self.query_one("#songs", SongTable))
        self.metadata.refresh(paths)
        # Update tagline with total song count once songs are loaded
        self._update_header_tagline()
        # Initialize responsive columns on start
        self.call_after_refresh(self._apply_responsive_columns)

    def on_resize(self, event: events.Resize) -> None:
        self.call_after_refresh(self._apply_responsive_columns)

    def _apply_responsive_columns(self) -> None:
        try:
            table = self.query_one("#songs", SongTable)
            width = self.size.width
            if width >= 90:
                new_show_notes = True
                new_show_risk = True
                new_show_suggested = True
            elif width >= 80:
                new_show_notes = False
                new_show_risk = True
                new_show_suggested = True
            elif width >= 72:
                new_show_notes = True
                new_show_risk = True
                new_show_suggested = False
            elif width >= 64:
                new_show_notes = False
                new_show_risk = True
                new_show_suggested = False
            else:
                new_show_notes = False
                new_show_risk = False
                new_show_suggested = False

            if (
                new_show_notes != self.show_notes
                or new_show_risk != self.show_risk
                or new_show_suggested != self.show_suggested
                or len(table.columns) == 0
            ):
                self.show_notes = new_show_notes
                self.show_risk = new_show_risk
                self.show_suggested = new_show_suggested

                table.clear(columns=True)
                table.add_column(" ", key="marker", width=2)
                table.add_column("Title", key="title", width=42)
                table.add_column("Time", key="time", width=8)
                if self.show_notes:
                    table.add_column("Notes", key="notes", width=8)
                if self.show_risk:
                    table.add_column("Risk", key="risk", width=8)
                if self.show_suggested:
                    table.add_column("Suggested", key="suggested", width=16)

                self._render_table()

            # Recalculate title column width dynamically to take up remaining space
            table_width = table.size.width
            if table_width > 0:
                visible_other_count = 2  # marker and time
                other_cols_width = 2 + 8
                if self.show_notes:
                    visible_other_count += 1
                    other_cols_width += 8
                if self.show_risk:
                    visible_other_count += 1
                    other_cols_width += 8
                if self.show_suggested:
                    visible_other_count += 1
                    other_cols_width += 16
                
                # Scrollbar is 1 cell wide, borders are 2 cells wide (left + right), title padding is 2 cells wide
                overhead = 3 + 2 + other_cols_width + (visible_other_count * 2)
                dynamic_title_width = max(20, table_width - overhead)
                
                # Update column width
                title_col = next((c for c in table.ordered_columns if c.key.value == "title"), None)
                if title_col is not None:
                    title_col.width = dynamic_title_width
                    table.clear_cached_dimensions()
                    table.refresh()
        except Exception:
            pass

    def on_unmount(self) -> None:
        # Once the picker exits into playback, metadata work must not keep competing with the
        # real-time dispatch thread.  Profile/fps changes still use the non-waiting close path via
        # _replace_metadata_coordinator(), but app shutdown waits for the active job to quiesce.
        try:
            self.picker_scope.close_all(wait=True)
            from sky_music.platform.win32 import inputs
            if getattr(inputs, "PLAYBACK_DEBUG", False):
                for snap in self.picker_scope.snapshots():
                    inputs.debug_log(
                        f"[background] picker resource {snap.name} closed={snap.closed} "
                        f"pending={snap.pending_count} running={snap.running_count}"
                    )
            self.picker_scope.assert_closed()
            from sky_music.orchestration.telemetry import TelemetryLogger
            TelemetryLogger.last_picker_cleanup = {
                "ok": True,
                "resources": [
                    {
                        "name": snap.name,
                        "phase": snap.phase,
                        "state": snap.state,
                        "closed": snap.closed,
                        "pending_count": snap.pending_count,
                        "running_count": snap.running_count,
                    }
                    for snap in self.picker_scope.snapshots()
                ]
            }
        except Exception as exc:
            from sky_music.platform.win32 import inputs
            inputs.debug_log(f"[background] Cleanup error in Textual picker unmount: {exc}")
            from sky_music.orchestration.telemetry import TelemetryLogger
            resources_list = []
            try:
                for snap in self.picker_scope.snapshots():
                    resources_list.append({
                        "name": snap.name,
                        "phase": snap.phase,
                        "state": snap.state,
                        "closed": snap.closed,
                        "pending_count": snap.pending_count,
                        "running_count": snap.running_count,
                    })
            except Exception:
                pass
            TelemetryLogger.last_picker_cleanup = {
                "ok": False,
                "resources": resources_list,
                "error": str(exc),
            }
            raise exc

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id != "search":
            return
        self.query = event.value
        import sys
        if "pytest" in sys.modules or "unittest" in sys.modules:
            if self._search_timer is not None:
                try:
                    self._search_timer.stop()
                except Exception:
                    pass
                self._search_timer = None
            self._perform_search()
        else:
            if self._search_timer is not None:
                try:
                    self._search_timer.stop()
                except Exception:
                    pass
            self._search_timer = self.set_timer(0.15, self._perform_search)

    def _perform_search(self) -> None:
        self._search_timer = None
        self.filtered = rank_song_choices(self.choices, self.query)
        self._render_status()
        self._render_table(reset_cursor=True)
        self._render_detail()

    def on_key(self, event: events.Key) -> None:
        if event.key == "enter":
            event.stop()
            self.action_confirm()
        elif event.key == "escape":
            event.stop()
            search = self.query_one("#search", Input)
            if search.has_focus:
                self._focus_table()
            else:
                self.action_cancel()
        elif event.key == "up":
            search = self.query_one("#search", Input)
            if search.has_focus:
                event.stop()
                table = self.query_one("#songs", SongTable)
                table.action_cursor_up()
        elif event.key == "down":
            search = self.query_one("#search", Input)
            if search.has_focus:
                event.stop()
                table = self.query_one("#songs", SongTable)
                table.action_cursor_down()
        elif event.key == "q":
            search = self.query_one("#search", Input)
            if not search.value and not search.has_focus:
                event.stop()
                self.action_cancel()

    def on_data_table_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        self._set_marker(event.row_key)
        self._render_detail()

    def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        event.stop()
        self.action_confirm(song_path=Path(event.row_key.value))

    def _set_marker(self, row_key: object | None) -> None:
        table = self.query_one("#songs", SongTable)
        t = self._theme_tokens
        if self._marked_row_key is not None:
            try:
                table.update_cell(self._marked_row_key, "marker", t.song_icon)
            except Exception:
                pass
        if row_key is not None:
            try:
                table.update_cell(row_key, "marker", t.pointer)
            except Exception:
                pass
        self._marked_row_key = row_key

    def _sync_marker(self) -> None:
        table = self.query_one("#songs", SongTable)
        if not self.filtered:
            return
        try:
            row_key = table.coordinate_to_cell_key(table.cursor_coordinate).row_key
        except Exception:
            return
        self._set_marker(row_key)

    def on_screen_resume(self, _event: events.ScreenResume) -> None:
        self.call_after_refresh(self._focus_table)

    def _update_header_tagline(self) -> None:
        """Sync the header tagline to reflect the current total song count."""
        total = len(self.choices)
        noun = "song" if total == 1 else "songs"
        tagline = f"precision music player  ♪ {total} {noun}"
        try:
            self.query_one("#appbar", GradientHeader).set_tagline(tagline)
        except Exception:
            pass

    def _render_status(self) -> None:
        fps = "auto" if self.fps is None else f"{self.fps}fps"
        # Core session params — always shown.
        parts = [self.profile_name, f"{self.tempo_scale:.2f}×", fps, self.active_theme]
        # Only append non-default flags so the status bar stays uncluttered.
        # "dry-run" is a meaningful deviation; show it prominently.
        if self.dry_run:
            parts.append("dry-run")
        # Show "hud off" when the HUD is disabled to avoid confusion.
        if not self.verbose_hud:
            parts.append("hud off")
        if self.telemetry_enabled:
            parts.append("tele")
        # Use │ (box-drawing pipe) as separator — matches Claude Code / Codex CLI aesthetic
        chips = " │ ".join(parts)
        try:
            self.query_one("#appbar", GradientHeader).set_status(chips)
        except Exception:
            pass
        try:
            self.query_one(CustomFooter).refresh()
        except Exception:
            pass
        table = self.query_one("#songs", SongTable)
        table.border_subtitle = f"{len(self.filtered)}/{len(self.choices)}"

    def _render_table(self, *, reset_cursor: bool = False) -> None:
        table = self.query_one("#songs", SongTable)
        previous_row = 0 if reset_cursor else table.cursor_row
        normalized_query = remove_accents(self.query).casefold().strip()
        match_style = f"bold {self._theme_tokens.match}"
        muted = self._theme_tokens.muted
        song_icon = self._theme_tokens.song_icon
        table.clear()
        self._marked_row_key = None
        for choice in self.filtered:
            metadata = peek_cached_song_ui_metadata(choice.path, self.session, self.cfg)
            duration, notes, risk, suggested = _metadata_cells(metadata)
            
            row_cells = [
                song_icon,
                _title_cell(choice.path.stem, normalized_query, match_style),
                duration,
            ]
            if self.show_notes:
                row_cells.append(notes)
            if self.show_risk:
                row_cells.append(_risk_cell(risk, muted, self._theme_tokens))
            if self.show_suggested:
                row_cells.append(suggested)

            table.add_row(*row_cells, key=str(choice.path))
            
        if self.filtered:
            table.move_cursor(row=min(previous_row, len(self.filtered) - 1), column=0)
            self._sync_marker()

    def refresh_metadata_rows(self) -> None:
        table = self.query_one("#songs", SongTable)
        muted = self._theme_tokens.muted
        for choice in self.filtered:
            row_key = str(choice.path)
            try:
                metadata = peek_cached_song_ui_metadata(choice.path, self.session, self.cfg)
                if metadata is not None:
                    duration, notes, risk, suggested = _metadata_cells(metadata)
                    table.update_cell(row_key, "time", duration)
                    if self.show_notes:
                        table.update_cell(row_key, "notes", notes)
                    if self.show_risk:
                        table.update_cell(row_key, "risk", _risk_cell(risk, muted, self._theme_tokens))
                    if self.show_suggested:
                        table.update_cell(row_key, "suggested", suggested)
            except Exception:
                pass
        self._render_detail()

    def _render_detail(self) -> None:
        detail = self.query_one("#detail", DetailPanel)
        t = self._theme_tokens
        if not self.preview_visible:
            detail.update(Text("Details hidden", style=t.muted))
            return

        selected = self._selected_choice()
        if selected is None:
            detail.update(build_empty_detail_text(t, bool(self.choices), self.query))
            return

        metadata = peek_cached_song_ui_metadata(selected.path, self.session, self.cfg)
        detail.update(build_detail_text(selected.path, metadata, t))

    def _selected_choice(self) -> SongChoice | None:
        if not self.filtered:
            return None
        table = self.query_one("#songs", SongTable)
        index = max(0, min(table.cursor_row, len(self.filtered) - 1))
        return self.filtered[index]

    def action_confirm(self, song_path: Path | None = None) -> None:
        if getattr(self, "_transitioning_to_playback", False):
            return
        self._transitioning_to_playback = True

        if getattr(self, "_search_timer", None) is not None:
            try:
                self._search_timer.stop()
            except Exception:
                pass
            self._search_timer = None
            self._perform_search()

        if song_path is not None:
            selected_path = song_path
        else:
            selected = self._selected_choice()
            if selected is None:
                return
            selected_path = selected.path

        picker_result = SongPickerResult(
            song_path=selected_path,
            action="dry_run" if self.dry_run else "play",
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            verbose_hud=self.verbose_hud,
            telemetry_enabled=self.telemetry_enabled,
        )

        if not self.unified_mode:
            self.exit(picker_result)
        else:
            self.start_playback_workflow(picker_result)

    def action_cancel(self) -> None:
        self.exit(None)

    def _replace_metadata_coordinator(self) -> None:
        self.picker_scope.retire(self.metadata)
        self.metadata.cancel()
        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )
        self.metadata = self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg))
        self._render_status()
        self._render_table()
        self._render_detail()
        self.metadata.refresh([choice.path for choice in self.choices])
        self._focus_table()

    def _focus_table(self) -> None:
        self.set_focus(self.query_one("#songs", SongTable))

    def action_open_profile(self) -> None:
        options = [PickerOption(name, f"{name} - {desc}") for name, desc in PROFILES_INFO]
        self.push_screen(OptionModal("Timing Profile", options, theme_name=self.active_theme), self._apply_profile)

    def _apply_profile(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        self.profile_name = canonical_profile_name(str(value))
        persist_default_profile(self.cfg, self.profile_name)
        self._replace_metadata_coordinator()

    def action_open_tempo(self) -> None:
        options = [PickerOption(value, f"{value:.2f}x - {desc}") for value, desc in TEMPO_OPTIONS]
        self.push_screen(OptionModal("Tempo", options, theme_name=self.active_theme), self._apply_tempo)

    def _apply_tempo(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        self.tempo_scale = float(value)
        persist_default_tempo(self.cfg, self.tempo_scale)
        self._replace_metadata_coordinator()

    def action_open_fps(self) -> None:
        options = [
            PickerOption("auto" if value is None else value, f"{'Auto' if value is None else value} - {desc}")
            for value, desc in FPS_OPTIONS
        ]
        self.push_screen(OptionModal("FPS", options, theme_name=self.active_theme), self._apply_fps)

    def _apply_fps(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        self.fps = None if value == "auto" else int(value)
        persist_default_fps(self.cfg, self.fps)
        self._replace_metadata_coordinator()

    def action_open_theme(self) -> None:
        options = [PickerOption(name, name) for name in THEME_PRESETS]
        self.push_screen(OptionModal("Theme", options, theme_name=self.active_theme), self._apply_theme)

    def _apply_theme(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        self.active_theme = self._normalize_theme_name(str(value))
        save_theme(self.active_theme)
        self.cfg.theme = self.active_theme
        self._apply_theme_class()
        self._render_status()
        self._render_table()
        self._render_detail()
        self._focus_table()

    def action_open_commands(self) -> None:
        self.push_screen(
            CommandModal("Commands", COMMANDS, theme_name=self.active_theme),
            self._run_command,
        )

    def _run_command(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        command = str(value)
        if command == "preview":
            self.preview_visible = True
            self._render_detail()
        elif command == "profile":
            self.action_open_profile()
        elif command == "tempo":
            self.action_open_tempo()
        elif command == "fps":
            self.action_open_fps()
        elif command == "calibration":
            self.action_open_calibration()
        elif command == "dry_run":
            self.action_toggle_dry_run()
        elif command == "hud":
            self.action_toggle_hud()
        elif command == "telemetry":
            self.action_toggle_telemetry()
        elif command == "reload":
            self.action_reload_songs()
        elif command == "theme":
            self.action_open_theme()
        elif command == "help":
            self.action_open_help()

    def action_toggle_preview(self) -> None:
        self.preview_visible = not self.preview_visible
        self._render_detail()
        self._focus_table()

    def action_open_help(self) -> None:
        t = self._theme_tokens
        key_width = 10
        label_width = 22

        sections: list[tuple[str, list[tuple[str, str, str]]]] = [
            (
                "Navigation",
                [
                    ("/", "Commands", "Open command palette"),
                    ("Enter", "Play", "Play selected song"),
                    ("↑↓", "Navigate", "Move selection"),
                    ("Esc / q", "Cancel", "Close picker"),
                ],
            )
        ]

        command_groups: dict[str, list[tuple[str, str, str]]] = {
            "View": [],
            "Playback": [],
            "Interface": [],
            "Library": [],
            "System": [],
        }
        for cmd in COMMANDS:
            if cmd.id == "help":
                command_groups["System"].append((cmd.key, cmd.label, "Open this help modal"))
            elif cmd.group in command_groups:
                command_groups[cmd.group].append((cmd.key, cmd.label, cmd.description))

        for group_name in ("View", "Playback", "Interface", "Library", "System"):
            if command_groups[group_name]:
                sections.append((group_name, command_groups[group_name]))

        content = Text()
        for index, (section_name, items) in enumerate(sections):
            if not items:
                continue
            if index:
                content.append("\n")
            content.append(section_name, style=f"bold {t.key}")
            for key, label, description in items:
                content.append("\n  ")
                content.append(pad_text(key, key_width), style=f"bold {t.accent}")
                content.append(pad_text(label, label_width), style=t.foreground)
                content.append(description, style=t.muted)
            content.append("\n")

        self.push_screen(
            InfoModal(
                "Sky Player Keyboard Shortcuts",
                content,
                theme_name=self.active_theme,
            )
        )

    def action_open_calibration(self) -> None:
        from sky_music.orchestration.calibration import (
            calibrate_profile,
            calibration_input_from_summary,
            load_latest_telemetry_summary,
        )

        summary = load_latest_telemetry_summary()
        if summary is None:
            self.push_screen(
                InfoModal(
                    "Calibration Error",
                    "No telemetry summary found in logs.\nRun playback with telemetry enabled first.",
                    theme_name=self.active_theme,
                )
            )
            return
        else:
            inp = calibration_input_from_summary(summary)
            rec = calibrate_profile(inp)
            t = self._theme_tokens
            accent = t.accent
            info_lines = [
                f"[bold {accent}]Profile:[/]   {rec.profile_name}",
                f"[bold {accent}]Tempo:[/]     {rec.tempo_scale:.2f}x",
                f"[bold {accent}]Hold:[/]      {rec.hold_us / 1000:.1f}ms",
                f"[bold {accent}]Severity:[/]  {rec.severity.upper()}",
                "",
                f"[bold {accent}]Reason:[/]    {rec.reason}",
            ]
            options = [
                PickerOption(
                    CalibrationChoice(rec.profile_name, rec.tempo_scale, inp.fps),
                    "Apply Recommendation",
                ),
                PickerOption(None, "Close"),
            ]
            self.push_screen(
                OptionModal(
                    "Calibration Recommendation",
                    options,
                    info_text="\n".join(info_lines),
                    theme_name=self.active_theme,
                ),
                self._apply_calibration,
            )

    def _apply_calibration(self, value: object | None) -> None:
        if not isinstance(value, CalibrationChoice):
            self._focus_table()
            return
        persist_calibration_defaults(
            self.cfg,
            profile_name=value.profile_name,
            tempo_scale=value.tempo_scale,
            fps=value.fps,
        )
        self.profile_name = canonical_profile_name(value.profile_name)
        self.tempo_scale = value.tempo_scale
        self.fps = value.fps if value.fps > 0 else None
        self._replace_metadata_coordinator()

    def action_toggle_dry_run(self) -> None:
        self.dry_run = not self.dry_run
        self._render_status()
        self._focus_table()

    def action_toggle_hud(self) -> None:
        self.verbose_hud = not self.verbose_hud
        self.cfg.verbose_hud = self.verbose_hud
        save_config(self.cfg)
        self._render_status()
        self._focus_table()

    def action_toggle_telemetry(self) -> None:
        self.telemetry_enabled = not self.telemetry_enabled
        self.cfg.telemetry_enabled_by_default = self.telemetry_enabled
        save_config(self.cfg)
        self._render_status()
        self._focus_table()

    def action_reload_songs(self) -> None:
        if getattr(self, "_search_timer", None) is not None:
            try:
                self._search_timer.stop()
            except Exception:
                pass
            self._search_timer = None

        clear_metadata_cache()
        paths = get_song_choices(force_refresh=True)
        self.choices = [
            SongChoice(path=path, search_key=remove_accents(path.stem).casefold())
            for path in paths
        ]
        self.filtered = rank_song_choices(self.choices, self.query)
        self._render_status()
        self._update_header_tagline()
        self._render_table(reset_cursor=True)
        self._render_detail()
        self.metadata.refresh(paths)

    def quiesce(self) -> None:
        try:
            self.picker_scope.close_all(wait=True)
            self.picker_scope.assert_closed()
            from sky_music.orchestration.telemetry import TelemetryLogger
            TelemetryLogger.last_picker_cleanup = {
                "ok": True,
                "resources": [
                    {
                        "name": snap.name,
                        "phase": snap.phase,
                        "state": snap.state,
                        "closed": snap.closed,
                        "pending_count": snap.pending_count,
                        "running_count": snap.running_count,
                    }
                    for snap in self.picker_scope.snapshots()
                ]
            }
        except Exception as exc:
            from sky_music.orchestration.telemetry import TelemetryLogger
            TelemetryLogger.last_picker_cleanup = {
                "ok": False,
                "error": str(exc),
            }

    def rearm(self) -> None:
        self.picker_scope = BackgroundScope(phase="picker")
        self.metadata = self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg))
        self.metadata.refresh([choice.path for choice in self.choices])
        self._focus_table()

    def start_playback_workflow(self, picker_result: SongPickerResult) -> None:
        is_dry_run = (picker_result.action == "dry_run")
        session = PlaybackSessionContext(
            profile_name=picker_result.profile_name,
            tempo_scale=picker_result.tempo_scale,
            fps=picker_result.fps,
            scan_code_mode=self.scan_code_mode,
        )
        res = prepare_playback(picker_result.song_path, session, self.cfg, is_dry_run=is_dry_run)

        if isinstance(res, PlaybackError):
            def reset_flag(_: Any = None) -> None:
                self._transitioning_to_playback = False
            self.call_after_refresh(self.push_screen, InfoModal("Playback Error", f"[bold {self._theme_tokens.danger}]{res.message}[/]", theme_name=self.active_theme), reset_flag)
            return

        if res.risk_report.severity != "low":
            from sky_music.ui.textual_app.modals import OptionModal, PickerOption

            danger_color = {
                "high": self._theme_tokens.danger,
                "medium": self._theme_tokens.warning,
                "low": self._theme_tokens.success,
            }.get(res.risk_report.severity, self._theme_tokens.accent)

            info_text = f"[bold {danger_color}]Risk Level: {res.risk_report.severity.upper()}[/]\n\n"
            info_text += "\n".join(f"• {rec}" for rec in res.risk_report.recommendations)

            options = [
                PickerOption("proceed", "Proceed with current settings"),
                PickerOption("switch_profile", f"Switch to recommended '{res.risk_report.suggested_profile}' profile"),
                PickerOption("scale_tempo", f"Scale tempo down to {res.risk_report.suggested_tempo_scale:.2f}x"),
                PickerOption("dry_run", "Dry-run first (simulate, no keystrokes)"),
                PickerOption("cancel", "Cancel and return to picker"),
            ]

            from dataclasses import replace

            def handle_risk_decision(decision: Any) -> None:
                if decision == "proceed":
                    self.execute_playback_plan(res, picker_result)
                elif decision == "switch_profile":
                    rebuilt = rebuild_with(res, profile=res.risk_report.suggested_profile, is_dry_run=is_dry_run)
                    if isinstance(rebuilt, PlaybackError):
                        def reset_flag(_: Any = None) -> None:
                            self._transitioning_to_playback = False
                        self.call_after_refresh(self.push_screen, InfoModal("Playback Error", f"[bold {self._theme_tokens.danger}]{rebuilt.message}[/]", theme_name=self.active_theme), reset_flag)
                    else:
                        updated_picker_result = replace(picker_result, profile_name=res.risk_report.suggested_profile)
                        try:
                            user_cfg = load_config()
                            persist_default_profile(user_cfg, res.risk_report.suggested_profile)
                        except Exception:
                            pass
                        self.execute_playback_plan(rebuilt, updated_picker_result)
                elif decision == "scale_tempo":
                    new_tempo = res.risk_report.suggested_tempo_scale
                    rebuilt = rebuild_with(res, tempo=new_tempo, is_dry_run=is_dry_run)
                    if isinstance(rebuilt, PlaybackError):
                        def reset_flag(_: Any = None) -> None:
                            self._transitioning_to_playback = False
                        self.call_after_refresh(self.push_screen, InfoModal("Playback Error", f"[bold {self._theme_tokens.danger}]{rebuilt.message}[/]", theme_name=self.active_theme), reset_flag)
                    else:
                        updated_picker_result = replace(picker_result, tempo_scale=new_tempo)
                        self.execute_playback_plan(rebuilt, updated_picker_result)
                elif decision == "dry_run":
                    rebuilt = rebuild_with(res, is_dry_run=True)
                    if isinstance(rebuilt, PlaybackError):
                        def reset_flag(_: Any = None) -> None:
                            self._transitioning_to_playback = False
                        self.call_after_refresh(self.push_screen, InfoModal("Playback Error", f"[bold {self._theme_tokens.danger}]{rebuilt.message}[/]", theme_name=self.active_theme), reset_flag)
                    else:
                        updated_picker_result = replace(picker_result, action="dry_run")
                        self.execute_playback_plan(rebuilt, updated_picker_result)
                elif decision == "cancel" or decision is None:
                    self._transitioning_to_playback = False

            self.push_screen(
                OptionModal("Playback Risk Warnings", options, info_text=info_text, theme_name=self.active_theme),
                handle_risk_decision
            )
        else:
            self.execute_playback_plan(res, picker_result)

    def execute_playback_plan(self, plan: PlaybackPlan, picker_result: SongPickerResult) -> None:
        self.quiesce()

        from sky_music.orchestration.telemetry import TelemetryLogger
        if _picker_cleanup_failed(TelemetryLogger.last_picker_cleanup):
            error_msg = TelemetryLogger.last_picker_cleanup.get("error", "Unknown error during picker cleanup")
            self.rearm()
            def reset_flag(_: Any = None) -> None:
                self._transitioning_to_playback = False
            self.call_after_refresh(self.push_screen, InfoModal("Cleanup Error", f"[bold {self._theme_tokens.danger}]Failed to stop background workers: {error_msg}[/]", theme_name=self.active_theme), reset_flag)
            return

        from sky_music.orchestration.engine import PlaybackEngine
        from sky_music.infrastructure.backend import DryRunBackend, WinSendInputBackend

        is_dry_run = (picker_result.action == "dry_run")
        backend = DryRunBackend() if is_dry_run else WinSendInputBackend()

        renderer = SnapshotRenderer()

        main_mod = _get_main_module()
        if main_mod:
            telemetry_enabled = main_mod.RUNTIME_STATE.telemetry_csv_enabled or self.cfg.telemetry_enabled_by_default or main_mod.PLAYBACK_DEBUG or is_dry_run
            use_dispatch_thread = main_mod.RUNTIME_STATE.use_dispatch_thread
            input_path_warn_us = self.cfg.input_path_warn_us if main_mod.RUNTIME_STATE.check_input_path else 0
            enable_timer_guard = main_mod.RUNTIME_STATE.enable_timer_guard
            enable_waitable_timer = main_mod.RUNTIME_STATE.enable_waitable_timer
            enable_gc_pause = main_mod.RUNTIME_STATE.enable_gc_pause
        else:
            # Fallback to config/defaults
            telemetry_enabled = self.cfg.telemetry_enabled_by_default or is_dry_run
            use_dispatch_thread = self.cfg.use_dispatch_thread
            input_path_warn_us = self.cfg.input_path_warn_us
            enable_timer_guard = True
            enable_waitable_timer = True
            enable_gc_pause = True

        engine = PlaybackEngine(
            song=plan.song,
            actions=plan.actions,
            backend=backend,
            controls=self.controls,
            renderer=renderer,
            telemetry_enabled=telemetry_enabled,
            require_focus=not is_dry_run,
            profile_name=plan.session.display_profile_label(),
            tempo_scale=plan.session.tempo_scale,
            sleep_policy=plan.active_sleep_policy,
            focus_restore_grace_us=plan.active_policy.focus_restore_grace_us,
            fps=getattr(plan.active_policy, "fps", None),
            min_hold_us=int(plan.active_policy.min_hold_us),
            same_key_conflict_policy=plan.active_policy.same_key_conflict_policy,
            use_dispatch_thread=use_dispatch_thread,
            input_path_warn_us=input_path_warn_us,
            enable_timer_guard=enable_timer_guard,
            enable_waitable_timer=enable_waitable_timer,
            enable_gc_pause=enable_gc_pause,
        )
        engine.telemetry.record_schedule_metadata(plan.sched_meta)

        def run_playback() -> None:
            playback_screen = PlaybackScreen(
                engine=engine,
                renderer=renderer,
                theme_name=self.active_theme,
                song_name=plan.song.name,
                total_us=plan.sched_meta.playback_duration_us,
                violations=plan.violations,
                active_policy=plan.active_policy,
                profile_name=plan.session.display_profile_label(),
                tempo_scale=plan.session.tempo_scale,
                debug_mode=self.verbose_hud,
            )

            def handle_playback_result(result: Any) -> None:
                self.rearm()
                self._transitioning_to_playback = False
                if result == "quit":
                    self.exit(0)
                else:
                    self.update_session_state(picker_result)

            self.push_screen(playback_screen, handle_playback_result)

        if not is_dry_run and self.countdown_seconds > 0:
            countdown_screen = CountdownScreen(self.countdown_seconds, self.active_theme)
            self.push_screen(countdown_screen, lambda _: run_playback())
        else:
            run_playback()

    def update_session_state(self, picker_result: SongPickerResult) -> None:
        main_mod = _get_main_module()
        if not main_mod:
            raise RuntimeError("Could not resolve main module to update runtime state.")

        from sky_music.config import persist_playback_defaults
        from sky_music.domain.session_context import merge_session_with_overrides, PlaybackSessionContext
        user_cfg = load_config()
        updated_session = merge_session_with_overrides(
            main_mod.RUNTIME_STATE.session or PlaybackSessionContext.balanced(
                tempo_scale=main_mod.RUNTIME_STATE.tempo_scale,
                scan_code_mode=main_mod.RUNTIME_STATE.scan_code_mode,
            ),
            profile=picker_result.profile_name,
            tempo=picker_result.tempo_scale,
            fps=picker_result.fps,
        )
        main_mod.RUNTIME_STATE.apply_session(updated_session, user_cfg)
        main_mod.RUNTIME_STATE.dry_run = (picker_result.action == "dry_run")
        main_mod._sync_legacy_runtime_globals()

        persist_playback_defaults(
            user_cfg,
            profile_name=picker_result.profile_name,
            tempo_scale=picker_result.tempo_scale,
            fps=picker_result.fps,
        )


def _get_main_module():
    import sys
    main_mod = sys.modules.get('__main__')
    if main_mod and hasattr(main_mod, "RUNTIME_STATE"):
        return main_mod
    try:
        import main
        return main
    except ImportError:
        return None


def _picker_cleanup_failed(cleanup: dict | None) -> bool:
    """True when the recorded picker cleanup proves a worker could not be stopped.

    A missing record (``None``) is treated as clean: the unmount path always records a result, so
    ``None`` only happens when no picker actually ran. Only an explicit ``ok=False`` is an abort.
    """
    return bool(cleanup is not None and not cleanup.get("ok", False))


def choose_song_interactively_textual(
    theme_name: str | None = None,
    background_mode: str | None = None,
    initial_profile: str = "balanced",
    initial_tempo: float = 1.0,
    initial_fps: int | None = None,
    initial_dry_run: bool = False,
    scan_code_mode: str = "physical",
) -> SongPickerResult | None:
    app = SkyPickerApp(
        theme_name=theme_name,
        background_mode=background_mode,
        initial_profile=initial_profile,
        initial_tempo=initial_tempo,
        initial_fps=initial_fps,
        initial_dry_run=initial_dry_run,
        scan_code_mode=scan_code_mode,
    )
    # Reset before the run so a stale record from an earlier picker session cannot mask (or fake)
    # this run's cleanup outcome.  on_unmount() records ok=True/False as it tears the scope down.
    from sky_music.orchestration.telemetry import TelemetryLogger

    TelemetryLogger.last_picker_cleanup = None
    result = app.run()

    # Deterministic abort regardless of whether Textual propagates the on_unmount exception:
    if _picker_cleanup_failed(TelemetryLogger.last_picker_cleanup):
        error = TelemetryLogger.last_picker_cleanup.get("error", "unknown error")
        raise RuntimeError(
            f"picker background worker cleanup failed before playback: {error}"
        )
    return result


def run_sky_app_unified(
    theme_name: str | None = None,
    background_mode: str | None = None,
    initial_profile: str = "balanced",
    initial_tempo: float = 1.0,
    initial_fps: int | None = None,
    initial_dry_run: bool = False,
    scan_code_mode: str = "physical",
    controls: PlaybackControls | None = None,
    countdown_seconds: int = 3,
) -> int:
    app = SkyPickerApp(
        theme_name=theme_name,
        background_mode=background_mode,
        initial_profile=initial_profile,
        initial_tempo=initial_tempo,
        initial_fps=initial_fps,
        initial_dry_run=initial_dry_run,
        scan_code_mode=scan_code_mode,
        unified_mode=True,
        controls=controls,
        countdown_seconds=countdown_seconds,
    )
    from sky_music.orchestration.telemetry import TelemetryLogger
    TelemetryLogger.last_picker_cleanup = None

    app.run()

    if _picker_cleanup_failed(TelemetryLogger.last_picker_cleanup):
        error = TelemetryLogger.last_picker_cleanup.get("error", "unknown error")
        raise RuntimeError(
            f"picker background worker cleanup failed: {error}"
        )
    return 0


if __name__ == "__main__":
    choose_song_interactively_textual()
