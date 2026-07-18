"""Picker screen — song selection, filtering, and configuration."""

from __future__ import annotations

import contextlib
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Any, Protocol, cast

from rapidfuzz import fuzz, process
from rich.text import Text
from textual import events
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Container
from textual.message import Message
from textual.reactive import reactive
from textual.screen import Screen
from textual.widgets import DataTable, Input

from sky_music.config import (
    AppConfig,
    canonical_profile_name,
    load_config,
    persist_calibration_defaults,
    persist_default_fps,
    persist_default_profile,
    persist_default_tempo,
    resolve_game_fps,
    save_config,
)
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.infrastructure.background import BackgroundScope
from sky_music.ui.picker import (
    FPS_OPTIONS,
    PROFILES_INFO,
    TEMPO_OPTIONS,
    SongPickerResult,
)
from sky_music.ui.picker_helpers import get_song_choices, save_theme
from sky_music.ui.picker_metadata import (
    clear_metadata_cache,
    peek_cached_song_ui_metadata,
)
from sky_music.ui.picker_theme import (
    THEME_PRESETS,
    pad_text,
    remove_accents,
)
from sky_music.ui.textual_app.components.footers import CustomFooter
from sky_music.ui.textual_app.display_widgets import DetailPanel, GradientHeader
from sky_music.ui.textual_app.keymap import COMMANDS
from sky_music.ui.textual_app.messages import PickerActionRequested
from sky_music.ui.textual_app.modals import (
    CommandModal,
    InfoModal,
    OptionModal,
    PickerOption,
)
from sky_music.ui.textual_app.renderers import (
    _metadata_cells,
    _risk_cell,
    _title_cell,
    build_detail_text,
    build_empty_detail_text,
)
from sky_music.ui.textual_app.theme_css import (
    APP_CSS,
    TEXTUAL_THEME_TOKENS,
    TextualThemeTokens,
)
from sky_music.ui.textual_app.workers import MetadataCoordinator, MetadataHandle

if TYPE_CHECKING:
    from textual.widgets._data_table import RowKey


class PickerAppHost(Protocol):
    """Type contract for whatever App hosts a PickerScreen.

    The picker drives the App via these callbacks. Declaring them here (instead
    of letting the picker reach into ``self.app._on_picker_screen_*`` private
    methods) keeps Pyright happy without forcing a circular import between
    ``app`` and ``screens.picker``, and makes future renames surface as
    type errors — not as silently dropped events.
    """

    @property
    def profile_name(self) -> str: ...
    @profile_name.setter
    def profile_name(self, value: str) -> None: ...

    @property
    def tempo_scale(self) -> float: ...
    @tempo_scale.setter
    def tempo_scale(self, value: float) -> None: ...

    @property
    def fps(self) -> int: ...
    @fps.setter
    def fps(self, value: int) -> None: ...

    @property
    def dry_run(self) -> bool: ...
    @dry_run.setter
    def dry_run(self, value: bool) -> None: ...

    @property
    def verbose_hud(self) -> bool: ...
    @verbose_hud.setter
    def verbose_hud(self, value: bool) -> None: ...

    @property
    def telemetry_enabled(self) -> bool: ...
    @telemetry_enabled.setter
    def telemetry_enabled(self, value: bool) -> None: ...

    def on_picker_confirm(self, result: SongPickerResult) -> None: ...
    def on_picker_cancel(self) -> None: ...
    def on_picker_check_for_update(self) -> None: ...
    def on_picker_open_update_settings(self) -> None: ...
    def on_picker_snapshot_calibration_state(self, choice: CalibrationChoice | None) -> None: ...
    def on_picker_profile_changed(self, profile_name: str) -> None: ...
    def on_picker_tempo_changed(self, tempo_scale: float) -> None: ...
    def on_picker_fps_changed(self, fps: int) -> None: ...
    def on_picker_theme_changed(self, theme_name: str, background_mode: str) -> None: ...
    def on_picker_dry_run_changed(self, dry_run: bool) -> None: ...
    def on_picker_verbose_hud_changed(self, verbose_hud: bool) -> None: ...
    def on_picker_telemetry_enabled_changed(self, telemetry_enabled: bool) -> None: ...
    def handle_playback_card_key(self, key: str) -> bool: ...
    @property
    def playback_mode(self) -> str: ...
    def action_cancel(self) -> None: ...
    def notify(
        self,
        message: str,
        *,
        title: str = "",
        severity: str = "information",
        timeout: float = 3.0,
    ) -> None: ...
    def check_for_updates_worker(self, force: bool = False) -> None: ...

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




@dataclass(frozen=True, slots=True)
class CalibrationChoice:
    profile_name: str
    tempo_scale: float
    fps: int


@dataclass(frozen=True, slots=True)
class PendingRiskDecision:
    decision: str
    label: str


class SearchInput(Input):
    """Custom search input that shifts focus back to the song table on escape key."""
    def on_key(self, event: events.Key) -> None:
        if event.key == "escape":
            event.stop()
            try:
                table = self.screen.query_one("#songs", SongTable)
                table.focus()
            except Exception:
                pass


class PickerScreen(Screen[SongPickerResult]):
    """Main song picker UI that can be pushed to the screen stack."""

    CSS = APP_CSS

    AUTO_FOCUS = None  # PickerScreen handles focus in on_mount

    BINDINGS = [
        Binding("escape", "cancel", "Cancel", priority=True, show=False),
        Binding("q", "cancel", "Quit", show=False),
        Binding("enter", "confirm", "Play", show=False),
        Binding("/", "open_commands", "Commands", show=False),
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

    search_query: reactive[str] = reactive("", init=False)  # type: ignore[override]

    # ── Events → App ─────────────────────────────────────────────────
    # PickerScreen is instantiated and held by SkyPickerApp but never pushed
    # onto App's screen stack — the app composes its own widgets and uses the
    # screen as a logic/state holder. That means Textual's message bubbling
    # from ``self.post_message`` will not reach the App's ``_on_*`` handlers.
    # Instead we call the App callbacks directly here.
    #
    # To keep this type-safe (Pyright) without forcing an import cycle, the
    # public callback surface the App exposes is declared as a Protocol below;
    # ``self.app`` is cast to it at every call site. This is honest about the
    # design — the App is the picker's host — and the protocol is the
    # contract: any future App refactor that breaks it will fail Pyright
    # rather than mysteriously dropping events at runtime.

    class Confirm(Message):
        """Posted when user selects a song to play."""
        def __init__(self, result: SongPickerResult) -> None:
            super().__init__()
            self.result = result

    class Cancel(Message):
        """Posted when user cancels/exits the picker."""
        pass

    class CheckForUpdate(Message):
        """Posted when user requests a manual update check."""
        pass

    class ProfileChanged(Message):
        """Posted after profile change."""
        def __init__(self, profile_name: str) -> None:
            super().__init__()
            self.profile_name = profile_name

    class TempoChanged(Message):
        """Posted after tempo change."""
        def __init__(self, tempo_scale: float) -> None:
            super().__init__()
            self.tempo_scale = tempo_scale

    class FpsChanged(Message):
        """Posted after FPS change."""
        def __init__(self, fps: int) -> None:
            super().__init__()
            self.fps = fps

    class ThemeChanged(Message):
        """Posted after theme is applied."""
        def __init__(self, theme_name: str, background_mode: str) -> None:
            super().__init__()
            self.theme_name = theme_name
            self.background_mode = background_mode

    class SnapshotCalibrationState(Message):
        """Posted after calibration applies — carries updated picker state."""
        def __init__(self, choice: CalibrationChoice | None) -> None:
            super().__init__()
            self.choice = choice

    def __init__(
        self,
        *,
        name: str | None = "picker",
        id: str | None = "picker",
        choices: list[SongChoice] | None = None,
        theme_name: str | None = None,
        background_mode: str | None = None,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
        fps: int = 30,
        dry_run: bool = False,
        scan_code_mode: str = "physical",
        cfg: AppConfig | None = None,
        verbose_hud: bool = False,
        telemetry_enabled: bool = False,
        dispatch_lead_us: int = 0,
    ) -> None:
        super().__init__(name=name, id=id)
        self.profile_name = profile_name
        self.tempo_scale = tempo_scale
        self.dry_run = dry_run
        self.scan_code_mode = scan_code_mode
        self.cfg = cfg or load_config()
        self.fps = fps
        self.verbose_hud = verbose_hud
        self.telemetry_enabled = telemetry_enabled
        self.active_theme = self._normalize_theme_name(theme_name or self.cfg.theme)
        self.background_mode = self._normalize_background_mode(background_mode or self.cfg.ui_background_mode)
        self.preview_visible = True
        self.show_notes = True
        self.show_risk = True
        self.show_suggested = True
        self.dispatch_lead_us = dispatch_lead_us
        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )
        self._provided_choices = choices
        self.choices: list[SongChoice] = []
        self.filtered: list[SongChoice] = []
        self._marked_row_key: object | None = None
        self.picker_scope = BackgroundScope(phase="picker")
        self.metadata: MetadataHandle = cast(MetadataHandle, self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg)))
        self._search_timer = None
        self._quiesced = False
        self._row_meta_sig: dict[str, tuple[str, str, str, str]] = {}
        self._detail_sig: tuple[object, ...] | None = None

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

    def run_worker(self, *args: Any, **kwargs: Any) -> Any:
        return self.app.run_worker(*args, **kwargs)

    def call_from_thread(self, callback: Any, *args: Any, **kwargs: Any) -> Any:
        return self.app.call_from_thread(callback, *args, **kwargs)

    def _apply_theme_class(self) -> None:
        for name in THEME_PRESETS:
            self.remove_class(f"theme-{name}")
        for mode in ("transparent", "painted"):
            self.remove_class(f"background-{mode}")
        self.add_class(self._theme_class)
        self.add_class(f"background-{self.background_mode}")
        t = self._theme_tokens
        try:
            self.query_one("#appbar", GradientHeader).set_theme(
                t.gradient, t.foreground, t.detail, t.foreground, lead=t.header_lead
            )
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[picker] failed to apply header theme")
        try:
            self.query_one(CustomFooter).set_theme(t.key, t.muted)
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[picker] failed to apply footer theme")
        try:
            from sky_music.ui.textual_app.playback_app import PlaybackCard
            self.query_one("#playback-card", PlaybackCard).styles.display = "none"
        except Exception:
            pass
        
        total = len(self.choices)
        noun = "song" if total == 1 else "songs"
        tagline = f"precision music player  ♪ {total} {noun}"
        try:
            self.query_one("#appbar", GradientHeader).set_tagline(tagline)
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[picker] failed to set header tagline")

    def compose(self) -> ComposeResult:
        with Container(id="root"):
            yield GradientHeader("\u266a Sky Player", "precision music player", id="appbar")
            search = SearchInput(placeholder="Search songs\u2026", id="search")
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

            from sky_music.ui.textual_app.playback_app import PlaybackCard
            yield PlaybackCard(theme_name=self.active_theme, id="playback-card")

            yield CustomFooter()

    def on_mount(self) -> None:
        self._apply_theme_class()
        if self._provided_choices is not None:
            self.choices = list(self._provided_choices)
        else:
            paths = get_song_choices(force_refresh=True)
            self.choices = [
                SongChoice(path=path, search_key=remove_accents(path.stem).casefold())
                for path in paths
            ]
        self.filtered = rank_song_choices(self.choices, self.search_query)
        self._render_status()
        self._render_table()
        self._render_detail()
        self.set_focus(self.app.query_one("#songs", SongTable))
        if self._provided_choices is not None:
            paths = [choice.path for choice in self.choices]
        else:
            paths = get_song_choices(force_refresh=False)
            if not paths:
                paths = [choice.path for choice in self.choices]
        self.metadata.refresh(paths)
        self._update_header_tagline()
        self.call_after_refresh(self._apply_responsive_columns)

    def on_resize(self, _event: events.Resize) -> None:
        self.call_after_refresh(self._apply_responsive_columns)

    def on_picker_action_requested(self, event: PickerActionRequested) -> None:
        event.stop()
        action = event.action
        if action == "open_commands":
            self.action_open_commands()
        elif action == "confirm":
            self.action_confirm()
        elif action == "cancel":
            self.action_cancel()

    def _apply_responsive_columns(self) -> None:
        try:
            table = self.app.query_one("#songs", SongTable)
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

            table_width = table.size.width
            if table_width > 0:
                visible_other_count = 2
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

                overhead = 3 + 2 + other_cols_width + (visible_other_count * 2)
                dynamic_title_width = max(20, table_width - overhead)

                title_col = next((c for c in table.ordered_columns if c.key.value == "title"), None)
                if title_col is not None:
                    title_col.width = dynamic_title_width
                    table.clear_cached_dimensions()
                    table.refresh()
        except Exception:
            pass

    def on_unmount(self) -> None:
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
            with contextlib.suppress(Exception):
                resources_list = [
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
            TelemetryLogger.last_picker_cleanup = {
                "ok": False,
                "resources": resources_list,
                "error": str(exc),
            }
            raise exc

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id != "search":
            return
        self.search_query = event.value  # type: ignore[assignment]
        if "pytest" in sys.modules or "unittest" in sys.modules:
            if self._search_timer is not None:
                self._search_timer.stop()
                self._search_timer = None
            self._perform_search()
        else:
            if self._search_timer is not None:
                self._search_timer.stop()
            self._search_timer = self.set_timer(0.15, self._perform_search)

    def get_metadata_priority_paths(self) -> list[Path]:
        try:
            table = self.app.query_one("#songs", SongTable)
            if not self.filtered:
                return []
            y_min = table.scroll_y
            y_max = y_min + table.size.height
            return [
                self.filtered[i].path
                for i in range(int(y_min), min(int(y_max), len(self.filtered)))
            ]
        except Exception:
            # fallback
            return [c.path for c in self.filtered[:40]]

    def _perform_search(self) -> None:
        self._search_timer = None
        self.filtered = rank_song_choices(self.choices, self.search_query)
        self._render_status()
        self._render_table(reset_cursor=True)
        self._render_detail()

    def on_key(self, event: events.Key) -> None:
        if cast(PickerAppHost, self.app).handle_playback_card_key(event.key):
            event.stop()
            return
        if event.key == "enter":
            event.stop()
            self.action_confirm()
        elif event.key == "escape":
            event.stop()
            search = self.app.query_one("#search", Input)
            if search.has_focus:
                self._focus_table()
            else:
                self.action_cancel()
        elif event.key == "up":
            search = self.app.query_one("#search", Input)
            if search.has_focus:
                event.stop()
                table = self.app.query_one("#songs", SongTable)
                table.action_cursor_up()
        elif event.key == "down":
            search = self.app.query_one("#search", Input)
            if search.has_focus:
                event.stop()
                table = self.app.query_one("#songs", SongTable)
                table.action_cursor_down()
        elif event.key == "q":
            search = self.app.query_one("#search", Input)
            if not search.value and not search.has_focus:
                event.stop()
                self.action_cancel()

    def on_data_table_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        self._set_marker(event.row_key)
        self._render_detail()

    def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        event.stop()
        row_key_value = event.row_key.value
        assert row_key_value is not None
        self.action_confirm(song_path=Path(row_key_value))

    def _set_marker(self, row_key: object | None) -> None:
        table = self.app.query_one("#songs", SongTable)
        t = self._theme_tokens
        if self._marked_row_key is not None:
            try:
                table.update_cell(cast(RowKey, self._marked_row_key), "marker", t.song_icon)
            except Exception:
                from sky_music.platform.win32 import inputs
                inputs.debug_log("[picker] failed to clear marker")
        if row_key is not None:
            try:
                table.update_cell(cast(RowKey, row_key), "marker", t.pointer)
            except Exception:
                from sky_music.platform.win32 import inputs
                inputs.debug_log("[picker] failed to set marker")
        self._marked_row_key = row_key

    def _sync_marker(self) -> None:
        table = self.app.query_one("#songs", SongTable)
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
        total = len(self.choices)
        noun = "song" if total == 1 else "songs"
        tagline = f"precision music player  \u266a {total} {noun}"
        try:
            self.app.query_one("#appbar", GradientHeader).set_tagline(tagline)
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[picker] failed to update header tagline")

    def _render_status(self) -> None:
        fps_str = f"{self.fps}fps"
        parts = [self.profile_name, f"{self.tempo_scale:.2f}\u00d7", fps_str, self.active_theme]
        if self.dry_run:
            parts.append("dry-run")
        if self.verbose_hud:
            parts.append("hud on")
        if self.telemetry_enabled:
            parts.append("tele")
        chips = " \u2502 ".join(parts)
        try:
            self.app.query_one("#appbar", GradientHeader).set_status(chips)
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[picker] failed to set status")
        try:
            self.app.query_one(CustomFooter).refresh()
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[picker] failed to refresh footer")
        table = self.app.query_one("#songs", SongTable)
        table.border_subtitle = f"{len(self.filtered)}/{len(self.choices)}"

    def _render_table(self, *, reset_cursor: bool = False) -> None:
        table = self.app.query_one("#songs", SongTable)
        previous_row = 0 if reset_cursor else table.cursor_row
        normalized_query = remove_accents(self.search_query).casefold().strip()
        match_style = f"bold {self._theme_tokens.match}"
        muted = self._theme_tokens.muted
        song_icon = self._theme_tokens.song_icon
        table.clear()
        self._marked_row_key = None
        self._row_meta_sig.clear()
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

            table.add_row(*row_cells, key=str(choice.path))  # type: ignore[arg-type]

        if self.filtered:
            table.move_cursor(row=min(previous_row, len(self.filtered) - 1), column=0)
            self._sync_marker()

    def refresh_metadata_rows(self) -> None:
        table = self.app.query_one("#songs", SongTable)
        muted = self._theme_tokens.muted
        for choice in self.filtered:
            row_key = str(choice.path)
            try:
                metadata = peek_cached_song_ui_metadata(choice.path, self.session, self.cfg)
                if metadata is not None:
                    duration, notes, risk, suggested = _metadata_cells(metadata)
                    sig = (duration, notes, risk, suggested)
                    if self._row_meta_sig.get(row_key) == sig:
                        continue
                    self._row_meta_sig[row_key] = sig
                    table.update_cell(row_key, "time", duration)
                    if self.show_notes:
                        table.update_cell(row_key, "notes", notes)
                    if self.show_risk:
                        table.update_cell(row_key, "risk", str(_risk_cell(risk, muted, self._theme_tokens)))
                    if self.show_suggested:
                        table.update_cell(row_key, "suggested", suggested)
            except Exception:
                pass
        self._render_detail()

    def _render_detail(self) -> None:
        detail = self.app.query_one("#detail", DetailPanel)
        t = self._theme_tokens
        if not self.preview_visible:
            sig = ("hidden",)
            if self._detail_sig != sig:
                self._detail_sig = sig
                detail.update(Text("Details hidden", style=t.muted))
            return

        selected = self._selected_choice()
        if selected is None:
            sig = ("empty", bool(self.choices), self.search_query)
            if self._detail_sig != sig:
                self._detail_sig = sig
                detail.update(build_empty_detail_text(t, bool(self.choices), self.search_query))
            return

        metadata = peek_cached_song_ui_metadata(selected.path, self.session, self.cfg)
        if metadata is not None:
            sig = (str(selected.path), metadata.analyzed, getattr(metadata, "risk", ""), metadata.note_count)
        else:
            sig = (str(selected.path), False, "", 0)
            
        if self._detail_sig == sig:
            return
        self._detail_sig = sig
        
        detail.update(build_detail_text(selected.path, metadata, t))

    def _selected_choice(self) -> SongChoice | None:
        if not self.filtered:
            return None
        table = self.app.query_one("#songs", SongTable)
        index = max(0, min(table.cursor_row, len(self.filtered) - 1))
        return self.filtered[index]

    def _hide_detail_and_table(self) -> None:
        # Hide search and detail panel — they are not useful during playback
        # and freeing their rows gives the song table more room above the card.
        # CustomFooter is hidden because the PlaybackCard provides its own
        # controls hint row.
        for selector in ("#search", "#detail", CustomFooter):
            try:
                w = self.app.query_one(selector)
                w.disabled = True
                w.styles.display = "none"
            except Exception:
                from sky_music.platform.win32 import inputs
                inputs.debug_log(f"[picker] failed to hide {selector}")
        # Song table: keep VISIBLE so the user can see what is playing and
        # what comes next, but disable interaction (focus + key bindings).
        # The Screen.playback-active CSS class dims the table visually.
        try:
            songs = self.app.query_one("#songs")
            songs.disabled = True
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[picker] failed to disable song table")

    def _show_detail_and_table(self) -> None:
        for selector in ("#detail", "#songs", "#search", CustomFooter):
            try:
                w = self.app.query_one(selector)
                w.disabled = False
                w.styles.display = "block"
            except Exception:
                from sky_music.platform.win32 import inputs
                inputs.debug_log(f"[picker] failed to show {selector}")
        self._render_detail()
        self._focus_table()

    def quiesce(self) -> None:
        self._quiesced = True
        self.picker_scope.close_all(wait=True)

    def rearm(self) -> None:
        self._quiesced = False
        self.picker_scope = BackgroundScope(phase="picker")
        self.metadata = cast(MetadataHandle, self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg)))
        self.metadata.refresh([choice.path for choice in self.choices])
        self._focus_table()

    def action_confirm(self, song_path: Path | None = None) -> None:
        # Re-entrancy guard: ``enter`` and a row ``RowSelected`` event can
        # both fire ``action_confirm`` for the same keypress (Textual dispatches
        # the key to the App's ``on_key`` *and* lets the focused DataTable emit
        # RowSelected on Enter). Without this guard the playback plan would
        # start twice — once via ``App.on_key`` → ``action_confirm()`` and
        # again via ``App.on_data_table_row_selected`` →
        # ``action_confirm(song_path=...)`` — which the focus-guard test
        # catches as a duplicate ``Win32SkyFocusGuard().focus()`` call. The
        # flag is reset by ``App._restore_picker_after_playback``.
        if getattr(self.app, "_transitioning_to_playback", False):
            return
        if self._search_timer is not None:
            with contextlib.suppress(Exception):
                self._search_timer.stop()
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

        cast(PickerAppHost, self.app).on_picker_confirm(picker_result)

    def action_cancel(self) -> None:
        from textual.widgets import Input

        from sky_music.ui.textual_app.app_state import PlaybackMode
        search = self.query_one("#search", Input)
        if search.has_focus:
            self._focus_table()
            return
        app = cast(PickerAppHost, self.app)
        if app.playback_mode != PlaybackMode.PICKER:
            app.action_cancel()

    def _replace_metadata_coordinator(self) -> None:
        self.picker_scope.retire(self.metadata)
        self.metadata.cancel()
        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )
        self.metadata = cast(MetadataHandle, self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg)))
        self._render_status()
        self._render_table()
        self._render_detail()
        self.metadata.refresh([choice.path for choice in self.choices])
        self._focus_table()

    def _focus_table(self) -> None:
        # Only call app.set_focus when this picker screen is still the
        # active screen.  Otherwise app.set_focus routes to the *top*
        # screen (e.g. a modal pushed after resume) and steals its focus.
        if self is not self.app.screen:
            return
        self.app.set_focus(self.app.query_one("#songs", SongTable))

    def action_open_profile(self) -> None:
        options = [PickerOption(name, f"{name} - {desc}") for name, desc in PROFILES_INFO]
        from sky_music.ui.timing_guidance import PROFILE_MODAL_INFO
        self.app.push_screen(
            OptionModal("Timing Profile", options, info_text=PROFILE_MODAL_INFO, theme_name=self.active_theme),
            self._apply_profile,
        )

    def _apply_profile(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        self.profile_name = canonical_profile_name(str(value))
        persist_default_profile(self.cfg, self.profile_name)
        self._replace_metadata_coordinator()
        cast(PickerAppHost, self.app).on_picker_profile_changed(self.profile_name)

    def action_open_tempo(self) -> None:
        options = [PickerOption(value, f"{value:.2f}x - {desc}") for value, desc in TEMPO_OPTIONS]
        self.app.push_screen(OptionModal("Tempo", options, theme_name=self.active_theme), self._apply_tempo)

    def _apply_tempo(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        assert value is not None
        self.tempo_scale = cast(float, value)
        persist_default_tempo(self.cfg, self.tempo_scale)
        self._replace_metadata_coordinator()
        cast(PickerAppHost, self.app).on_picker_tempo_changed(self.tempo_scale)

    def action_open_fps(self) -> None:
        options = [
            PickerOption(value, f"{value} - {desc}")
            for value, desc in FPS_OPTIONS
        ]
        from sky_music.ui.timing_guidance import FPS_MODAL_INFO
        self.app.push_screen(
            OptionModal("FPS", options, info_text=FPS_MODAL_INFO, theme_name=self.active_theme),
            self._apply_fps,
        )

    def _apply_fps(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        assert value is not None
        self.fps = resolve_game_fps(cast(int, value))
        persist_default_fps(self.cfg, self.fps)
        self._replace_metadata_coordinator()
        cast(PickerAppHost, self.app).on_picker_fps_changed(self.fps)

    def action_open_theme(self) -> None:
        options = [PickerOption(name, name) for name in THEME_PRESETS]
        self.app.push_screen(OptionModal("Theme", options, theme_name=self.active_theme), self._apply_theme)

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
        cast(PickerAppHost, self.app).on_picker_theme_changed(self.active_theme, self.background_mode)

    def action_open_commands(self) -> None:
        # Defer so the command runs *after* CommandModal dismiss + pop_screen
        # complete.  Otherwise push_screen inside _run_command races with
        # dismiss's own pop_screen and the newly pushed screen is popped.
        def _on_result(value: object | None) -> None:
            self.call_after_refresh(self._run_command, value)

        self.app.push_screen(
            CommandModal("Commands", COMMANDS, theme_name=self.active_theme),
            _on_result,
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
        elif command == "calibrate_latency":
            self.action_calibrate_input_latency()
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
        elif command == "update":
            self.action_check_for_update()
        elif command == "update_settings":
            self.action_open_update_settings()

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
                    ("\u2191\u2193", "Navigate", "Move selection"),
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

        sections.extend(
            (group_name, command_groups[group_name])
            for group_name in ("View", "Playback", "Interface", "Library", "System")
            if command_groups[group_name]
        )

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

        self.app.push_screen(
            InfoModal(
                "Sky Player Keyboard Shortcuts",
                content,
                theme_name=self.active_theme,
            )
        )

    def action_calibrate_input_latency(self) -> None:
        from sky_music.platform.win32 import inputs
        if inputs.get_sky_window() is not None:
            self.app.push_screen(
                InfoModal(
                    "Calibration Blocked",
                    "Error: The game (Sky) is currently running.\n\nPlease close the game entirely before running input calibration.",
                    theme_name=self.active_theme,
                )
            )
            return

        options = [
            PickerOption("yes", "Start calibration"),
            PickerOption("no", "Cancel"),
        ]
        text = (
            "This will measure your precise hardware keyboard latency.\n\n"
            "1. A separate Windows window will open.\n"
            "2. Keep that window focused (click/tap it if needed).\n"
            "3. The app will simulate 200 keypresses to measure latency.\n"
            "4. Cache is saved to .cache/input_latency.json.\n\n"
            "Would you like to proceed?"
        )
        
        def _on_confirm(choice: object | None) -> None:
            if choice == "yes":
                self.run_worker(self._run_latency_calibration_worker, exclusive=True)

        self.app.push_screen(
            OptionModal(
                "Input Latency Calibration",
                options,
                info_text=text,
                theme_name=self.active_theme,
            ),
            _on_confirm
        )

    async def _run_latency_calibration_worker(self) -> None:
        import asyncio

        from sky_music.platform.win32.calibration import calibrate_input_latency_harness
        
        try:
            loop = asyncio.get_running_loop()
            res = await loop.run_in_executor(None, calibrate_input_latency_harness)
            
            self.app.push_screen(
                InfoModal(
                    "Calibration Complete",
                    f"Sampled Down Latency (us): p50={res['down_us']['p50']}, p90={res['down_us']['p90']}, p99={res['down_us']['p99']}\n"
                    f"Sampled Up Latency   (us): p50={res['up_us']['p50']}, p90={res['up_us']['p90']}, p99={res['up_us']['p99']}\n\n"
                    "Calibration saved to .cache/input_latency.json successfully!",
                    theme_name=self.active_theme,
                )
            )
        except Exception as exc:
            self.app.push_screen(
                InfoModal(
                    "Calibration Failed",
                    f"Error running calibration:\n{exc}",
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
            self.app.push_screen(
                InfoModal(
                    "Calibration Error",
                    "No telemetry summary found in logs.\nRun playback with telemetry enabled first.",
                    theme_name=self.active_theme,
                )
            )
            return
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
        self.app.push_screen(
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
            cast(PickerAppHost, self.app).on_picker_snapshot_calibration_state(None)
            return
        persist_calibration_defaults(
            self.cfg,
            profile_name=value.profile_name,
            tempo_scale=value.tempo_scale,
            fps=value.fps,
        )
        self.profile_name = canonical_profile_name(value.profile_name)
        self.tempo_scale = value.tempo_scale
        self.fps = resolve_game_fps(value.fps)
        self._replace_metadata_coordinator()
        cast(PickerAppHost, self.app).on_picker_snapshot_calibration_state(value)

    def action_toggle_dry_run(self) -> None:
        self.dry_run = not self.dry_run
        cast(PickerAppHost, self.app).on_picker_dry_run_changed(self.dry_run)
        self._render_status()
        self._focus_table()

    def action_toggle_hud(self) -> None:
        self.verbose_hud = not self.verbose_hud
        cast(PickerAppHost, self.app).on_picker_verbose_hud_changed(self.verbose_hud)
        self.cfg.verbose_hud = self.verbose_hud
        save_config(self.cfg)
        self._render_status()
        self._focus_table()

    def action_toggle_telemetry(self) -> None:
        self.telemetry_enabled = not self.telemetry_enabled
        cast(PickerAppHost, self.app).on_picker_telemetry_enabled_changed(self.telemetry_enabled)
        self.cfg.telemetry_enabled_by_default = self.telemetry_enabled
        save_config(self.cfg)
        self._render_status()
        self._focus_table()

    def action_reload_songs(self) -> None:
        if self._search_timer is not None:
            with contextlib.suppress(Exception):
                self._search_timer.stop()
            self._search_timer = None

        clear_metadata_cache()
        paths = get_song_choices(force_refresh=True)
        self.choices = [
            SongChoice(path=path, search_key=remove_accents(path.stem).casefold())
            for path in paths
        ]
        self.filtered = rank_song_choices(self.choices, self.search_query)
        self._render_status()
        self._update_header_tagline()
        self._render_table(reset_cursor=True)
        self._render_detail()
        self.metadata.refresh(paths)

    def action_check_for_update(self) -> None:
        cast(PickerAppHost, self.app).on_picker_check_for_update()

    def action_open_update_settings(self) -> None:
        from sky_music.config import (
            persist_update_auto_apply,
            persist_update_auto_check,
            persist_update_skip_version,
        )
        from sky_music.ui.textual_app.modals import UpdateSettingsModal

        app = cast(PickerAppHost, self.app)

        def _on_auto_check(value: bool) -> None:
            persist_update_auto_check(self.cfg, value)
            app.notify(
                "Auto-update check enabled." if value else "Auto-update check disabled.",
                severity="information",
                timeout=4,
            )

        def _on_auto_apply(value: bool) -> None:
            persist_update_auto_apply(self.cfg, value)
            if value:
                app.notify(
                    "Auto-apply enabled — newer releases will be downloaded and"
                    " installed automatically on the next check.",
                    severity="warning",
                    timeout=6,
                )
            else:
                app.notify("Auto-apply disabled.", severity="information", timeout=4)

        def _on_settings_result(result: object) -> None:
            if result == "check_now":
                app.check_for_updates_worker(force=True)

        modal = UpdateSettingsModal(
            auto_check=self.cfg.update.auto_check,
            auto_apply=self.cfg.update.auto_apply,
            on_auto_check=_on_auto_check,
            on_auto_apply=_on_auto_apply,
            skip_version=self.cfg.update.skip_version,
            check_interval_s=self.cfg.update.check_interval_s,
            last_check_ts=self.cfg.update.last_check_ts,
            theme_name=self.active_theme,
        )
        modal._on_clear_skip = lambda: persist_update_skip_version(self.cfg, "")

        self.app.push_screen(modal, _on_settings_result)