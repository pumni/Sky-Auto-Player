"""Picker screen — song selection, filtering, and configuration."""

from __future__ import annotations

import contextlib
import sys
from concurrent.futures import ThreadPoolExecutor
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
    load_config,
    persist_calibration_defaults,
    persist_default_fps,
    persist_default_hold_frames,
    persist_default_tempo,
    resolve_game_fps,
    save_config,
)
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.infrastructure.background import BackgroundScope, ExecutorResource
from sky_music.ui.picker import (
    FPS_OPTIONS,
    HOLD_OPTIONS,
    TEMPO_OPTIONS,
    SongPickerResult,
)
from sky_music.ui.picker_helpers import save_theme
from sky_music.ui.picker_metadata import (
    clear_metadata_cache,
    invalidate_policy_metadata,
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
    build_detail_text,
    build_empty_detail_text,
)
from sky_music.ui.textual_app.theme_css import (
    APP_CSS,
    TEXTUAL_THEME_TOKENS,
    TextualThemeTokens,
)
from sky_music.ui.textual_app.workers import MetadataCoordinator, MetadataHandle


class PickerAppHost(Protocol):
    """Type contract for whatever App hosts a PickerScreen.

    The picker drives the App via these callbacks. Declaring them here (instead
    of letting the picker reach into ``self.app._on_picker_screen_*`` private
    methods) keeps Pyright happy without forcing a circular import between
    ``app`` and ``screens.picker``, and makes future renames surface as
    type errors — not as silently dropped events.
    """

    @property
    def hold_frames(self) -> float: ...
    @hold_frames.setter
    def hold_frames(self, value: float) -> None: ...

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
    def on_picker_hold_frames_changed(self, hold_frames: float) -> None: ...
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


class SongTable(DataTable[Any]):
    """DataTable wrapper for song picker rows."""

    class ViewportChanged(Message):
        pass

    def watch_scroll_y(self, old_value: float, new_value: float) -> None:
        # Note: DataTable does not define watch_scroll_y itself, but we can intercept it
        # because scroll_y is a reactive property on ScrollableContainer.
        if hasattr(super(), "watch_scroll_y"):
            super().watch_scroll_y(old_value, new_value)  # type: ignore
        self.post_message(self.ViewportChanged())

@dataclass(frozen=True, slots=True)
class CalibrationChoice:
    hold_frames: float
    tempo_scale: float
    fps: int


@dataclass
class CatalogScanned(Message):
    choices: list[SongChoice]
    generation: int


@dataclass(frozen=True, slots=True)
class MetadataPrioritySnapshot:
    selected: list[Path]
    visible: list[Path]
    overscan: list[Path]
    filtered: list[Path]
    
    def ordered_paths(self) -> list[Path]:
        priority: list[Path] = []
        seen: set[Path] = set()
        
        for paths_list in (self.selected, self.visible, self.overscan, self.filtered):
            for p in paths_list:
                if p not in seen:
                    priority.append(p)
                    seen.add(p)
                    
        return priority


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
        Binding("p", "open_hold", "Hold", priority=True, show=False),
        Binding("t", "open_tempo", "Tempo", priority=True, show=False),
        Binding("f", "open_fps", "FPS", priority=True, show=False),
        Binding("y", "open_theme", "Theme", priority=True, show=False),
        Binding("v", "toggle_preview", "Details", priority=True, show=False),
        Binding("d", "toggle_dry_run", "Dry-run", priority=True, show=False),
        Binding("h", "toggle_hud", "HUD", priority=True, show=False),
        Binding("f3", "toggle_telemetry", "Telemetry", priority=True, show=False),
        Binding("ctrl+r", "reload_songs", "Reload", priority=True, show=False),
        Binding("c", "calibrate_input_latency", "Input calibration", priority=True, show=False),
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

    class HoldFramesChanged(Message):
        """Posted after hold selection change."""
        def __init__(self, hold_frames: float) -> None:
            super().__init__()
            self.hold_frames = hold_frames

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
        hold_frames: float = 1.0,
        tempo_scale: float = 1.0,
        fps: int = 60,
        dry_run: bool = False,
        scan_code_mode: str = "physical",
        cfg: AppConfig | None = None,
        verbose_hud: bool = False,
        telemetry_enabled: bool = False,
    ) -> None:
        super().__init__(name=name, id=id)
        self.hold_frames = hold_frames
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
        self.session = PlaybackSessionContext(
            hold_frames=self.hold_frames,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )
        self._provided_choices = choices
        self.choices: list[SongChoice] = []
        self.filtered: list[SongChoice] = []
        self._catalog_generation = 0
        self._create_picker_resources()
        self._search_timer = None
        self._detail_timer = None
        self._quiesced = False
        self._row_meta_sig: dict[str, tuple[str, str, str, str]] = {}
        self._detail_sig: tuple[object, ...] | None = None
        self._search_query: str = ""
        self._priority_paths: MetadataPrioritySnapshot = MetadataPrioritySnapshot([], [], [], [])

    def _create_picker_resources(self) -> None:
        self.picker_scope = BackgroundScope(phase="picker")
        self._catalog_executor = ExecutorResource(
            name="textual-picker-catalog", 
            phase="picker-catalog", 
            executor=ThreadPoolExecutor(max_workers=1, thread_name_prefix="CatalogScanner")
        )
        self.picker_scope.register(self._catalog_executor)
        self.metadata: MetadataHandle = cast(MetadataHandle, self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg)))

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
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[picker] failed to apply header theme")
        try:
            self.query_one(CustomFooter).set_theme(t.key, t.muted)
        except Exception:
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[picker] failed to apply footer theme")
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
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[picker] failed to set header tagline")

    def compose(self) -> ComposeResult:
        with Container(id="root"):
            yield GradientHeader("\u266a Sky Auto Player", "precision music player", id="appbar")
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
        
        # Start empty to guarantee immediate first frame
        self.choices = list(self._provided_choices) if self._provided_choices is not None else []
        self.filtered = []
        
        self._render_status()
        self._render_table()
        self._render_detail()
        self.set_focus(self.app.query_one("#songs", SongTable))
        self._update_header_tagline()
        
        # Defer all expensive operations (filesystem, SQLite, parsing) until after first paint
        self.call_after_refresh(self._deferred_startup)

    def _deferred_startup(self) -> None:
        self._catalog_generation += 1
        self._catalog_executor.submit(self._scan_catalog_worker, self._catalog_generation)

    def _scan_catalog_worker(self, generation: int, force_refresh: bool = False) -> None:
        from sky_music.ui.picker_helpers import get_song_choices
        from sky_music.ui.picker_theme import remove_accents
        
        if self._quiesced or self._catalog_generation != generation:
            return
            
        if self._provided_choices is None:
            paths = get_song_choices(force_refresh=force_refresh)
            new_choices = [
                SongChoice(path=path, search_key=remove_accents(path.stem).casefold())
                for path in paths
            ]
        else:
            new_choices = list(self._provided_choices)
            
        if self._quiesced or self._catalog_generation != generation:
            return
            
        self.post_message(CatalogScanned(choices=new_choices, generation=generation))

    def on_catalog_scanned(self, message: CatalogScanned) -> None:
        if self._quiesced or self._catalog_generation != message.generation:
            return
        self._apply_catalog_choices(message.choices)

    def _apply_catalog_choices(self, choices: list[SongChoice]) -> None:
        self.choices = choices
        self.filtered = rank_song_choices(self.choices, self.search_query)
        self._render_table()
        self._render_detail()
        self._update_header_tagline()
        self._render_status()
        
        paths_to_refresh = [choice.path for choice in self.choices]
        self.metadata.refresh(paths_to_refresh)
        
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
        if self._detail_timer is not None:
            with contextlib.suppress(Exception):
                self._detail_timer.stop()
            self._detail_timer = None
        try:
            self.quiesce()
            from sky_music.platform.win32 import window_target
            if getattr(window_target, "PLAYBACK_DEBUG", False):
                for snap in self.picker_scope.snapshots():
                    window_target.debug_log(
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
            from sky_music.platform.win32 import window_target
            window_target.debug_log(f"[background] Cleanup error in Textual picker unmount: {exc}")
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

    def get_metadata_priority_paths(self) -> MetadataPrioritySnapshot:
        """Return snapshot of paths prioritized by relevance."""
        return self._priority_paths

    def _visible_row_indices(self) -> range | None:
        """Indices of ``self.filtered`` currently on screen (with header margin)."""
        try:
            table = self.app.query_one("#songs", SongTable)
        except Exception:
            return None
        if not self.filtered:
            return None
        height = table.size.height
        if height <= 0:
            return None
        y_min = max(0, int(table.scroll_y))
        y_max = min(len(self.filtered), y_min + height + 1)
        if y_min >= y_max:
            return None
        return range(y_min, y_max)

    def _update_priority_paths(self) -> None:
        if not self.filtered:
            self._priority_paths = MetadataPrioritySnapshot([], [], [], [])
            return
        
        try:
            table = self.app.query_one("#songs", SongTable)
        except Exception:
            self._priority_paths = MetadataPrioritySnapshot([], [], [], [c.path for c in self.filtered])
            return
            
        cursor_row = max(0, min(table.cursor_row, len(self.filtered) - 1))
        height = max(10, table.size.height)
        y_min = max(0, int(table.scroll_y))
        y_max = min(len(self.filtered), y_min + height + 1)
        
        overscan = 50
        o_min = max(0, y_min - overscan)
        o_max = min(len(self.filtered), y_max + overscan)
        
        selected = [self.filtered[cursor_row].path] if 0 <= cursor_row < len(self.filtered) else []
        visible = [self.filtered[i].path for i in range(y_min, y_max) if 0 <= i < len(self.filtered)]
        overscan_paths = [self.filtered[i].path for i in range(o_min, o_max) if 0 <= i < len(self.filtered)]
        filtered = [c.path for c in self.filtered]
                
        self._priority_paths = MetadataPrioritySnapshot(selected, visible, overscan_paths, filtered)

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

    def on_song_table_viewport_changed(self, _event: SongTable.ViewportChanged) -> None:
        self._update_priority_paths()
        self._refresh_visible_rows()
        self._schedule_detail_render()

    def on_data_table_row_highlighted(self, _event: DataTable.RowHighlighted) -> None:
        # Per-scroll path. Keep it cheap: refresh metadata cells for the
        # visible window and defer the detail-panel rebuild to a debounce
        # timer so fast scrolling never rebuilds the panel per frame.
        self._update_priority_paths()
        # Fix 4.4: refresh entire visible window — not just the highlighted
        # row — so rows that scrolled into view after metadata completed are
        # not stuck at placeholder values.
        self._refresh_visible_rows()
        self._schedule_detail_render()

    def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        event.stop()
        row_key_value = event.row_key.value
        assert row_key_value is not None
        self.action_confirm(song_path=Path(row_key_value))

    def _refresh_row_metadata(self, row_key: object) -> bool:
        """Refresh metadata cells for a single row if its rendered content changed."""
        try:
            key = str(getattr(row_key, "value", row_key))
        except Exception:
            return False
        try:
            metadata = peek_cached_song_ui_metadata(Path(key), self.session, self.cfg)
            if metadata is None:
                return False
            duration, notes, risk, suggested = _metadata_cells(metadata)
            sig = (duration, notes, risk, suggested)
            if self._row_meta_sig.get(key) == sig:
                return False
            self._row_meta_sig[key] = sig
            table = self.app.query_one("#songs", SongTable)
            table.update_cell(key, "time", duration)
            if self.show_notes:
                table.update_cell(key, "notes", notes)
            if self.show_risk:
                # Fix 4.5: pass Rich Text directly — str() strips bold colour styling
                table.update_cell(key, "risk", _risk_cell(risk, self._theme_tokens.muted, self._theme_tokens))
            if self.show_suggested:
                table.update_cell(key, "suggested", suggested)
            return True
        except Exception:
            return False

    def _refresh_visible_rows(self) -> None:
        """Refresh metadata cells for all currently visible rows.

        Called on every scroll event so rows that move into the viewport
        after background metadata completes pick up their values immediately.
        """
        indices = self._visible_row_indices()
        if indices is None:
            return
        for i in indices:
            if i < len(self.filtered):
                self._refresh_row_metadata(self.filtered[i].path)

    def _schedule_detail_render(self) -> None:
        if self._detail_timer is not None:
            with contextlib.suppress(Exception):
                self._detail_timer.stop()
        self._detail_timer = self.set_timer(0.06, self._render_detail_debounced)

    def _render_detail_debounced(self) -> None:
        self._detail_timer = None
        self._render_detail()

    def on_screen_resume(self, _event: events.ScreenResume) -> None:
        self.call_after_refresh(self._focus_table)

    def _update_header_tagline(self) -> None:
        total = len(self.choices)
        noun = "song" if total == 1 else "songs"
        tagline = f"precision music player  \u266a {total} {noun}"
        try:
            self.app.query_one("#appbar", GradientHeader).set_tagline(tagline)
        except Exception:
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[picker] failed to update header tagline")

    def _render_status(self) -> None:
        fps_str = f"{self.fps}fps"
        parts = [f"hold {self.hold_frames:.2f}f", f"{self.tempo_scale:.2f}\u00d7", fps_str, self.active_theme]
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
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[picker] failed to set status")
        try:
            self.app.query_one(CustomFooter).refresh()
        except Exception:
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[picker] failed to refresh footer")
        table = self.app.query_one("#songs", SongTable)
        table.border_subtitle = f"{len(self.filtered)}/{len(self.choices)}"

    def _title_cell(self, choice: SongChoice) -> Text:
        t = self._theme_tokens
        title = Text(choice.path.stem, style=t.foreground)
        query = remove_accents(self.search_query).casefold().strip()
        if query:
            norm_title = remove_accents(choice.path.stem).casefold()
            start = norm_title.find(query)
            if start >= 0:
                title.stylize(f"bold {t.accent}", start, start + len(query))
        return title

    def _render_table(self, *, reset_cursor: bool = False) -> None:
        table = self.app.query_one("#songs", SongTable)
        previous_row = 0 if reset_cursor else table.cursor_row
        
        if getattr(self, "_render_timer", None) is not None:
            self._render_timer.stop()  # type: ignore[attr-defined]
            self._render_timer = None
            
        table.clear()
        self._row_meta_sig.clear()

        with self.app.batch_update():
            for choice in self.filtered:
                row_cells = ["", self._title_cell(choice), ""]
                if self.show_notes:
                    row_cells.append("")
                if self.show_risk:
                    row_cells.append("")
                if self.show_suggested:
                    row_cells.append("")
                table.add_row(*row_cells, key=str(choice.path))  # type: ignore[arg-type]

        if self.filtered:
            table.move_cursor(row=min(previous_row, len(self.filtered) - 1), column=0)
            
        self.refresh_metadata_rows()

    def refresh_metadata_rows(self) -> None:
        """Refresh metadata cells for the visible rows only.

        Runs on the UI thread via the background coordinator, so it is limited
        to the on-screen window (+1 header margin) instead of the whole
        library; rows that scroll into view are refreshed by
        ``on_data_table_row_highlighted``.
        """
        indices = self._visible_row_indices()
        if indices is None:
            return
        changed = False
        for index in indices:
            if self._refresh_row_metadata(self.filtered[index].path):
                changed = True
        if changed:
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
            # Fix 4.8: include every rendered field in sig so changes to warnings
            # or recommendations don't leave a stale detail panel.
            sig = (
                str(selected.path),
                metadata.analyzed,
                metadata.risk,
                metadata.note_count,
                metadata.recommended_hold_frames,
                metadata.recommended_tempo_scale,
                metadata.warnings,
                metadata.duration_seconds,
                metadata.average_notes_per_second,
                metadata.peak_notes_per_second_1s,
                metadata.chords_count,
                metadata.min_note_gap_ms,
                metadata.min_same_key_gap_ms,
            )
        else:
            sig = (str(selected.path), False, "", 0, "", 0.0, (), 0.0, 0.0, 0.0, 0, 0.0, 0.0)
            
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
                from sky_music.platform.win32 import window_target
                window_target.debug_log(f"[picker] failed to hide {selector}")
        # Song table: keep VISIBLE so the user can see what is playing and
        # what comes next, but disable interaction (focus + key bindings).
        # The Screen.playback-active CSS class dims the table visually.
        try:
            songs = self.app.query_one("#songs")
            songs.disabled = True
        except Exception:
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[picker] failed to disable song table")

    def _show_detail_and_table(self) -> None:
        for selector in ("#detail", "#songs", "#search", CustomFooter):
            try:
                w = self.app.query_one(selector)
                w.disabled = False
                w.styles.display = "block"
            except Exception:
                from sky_music.platform.win32 import window_target
                window_target.debug_log(f"[picker] failed to show {selector}")
        self._render_detail()
        self._focus_table()

    def quiesce(self) -> Any:
        self._quiesced = True
        return self.picker_scope.close_all(wait=True)

    def rearm(self) -> None:
        self._quiesced = False
        self._create_picker_resources()
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
            hold_frames=self.hold_frames,
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
            hold_frames=self.hold_frames,
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

    def action_open_hold(self) -> None:
        options = [PickerOption(value, f"{value:.2f} frames — {desc.split('—', 1)[-1].strip()}") for value, desc in HOLD_OPTIONS]
        from sky_music.ui.timing_guidance import HOLD_MODAL_INFO
        self.app.push_screen(
            OptionModal("Hold Duration", options, info_text=HOLD_MODAL_INFO, theme_name=self.active_theme),
            self._apply_hold,
        )

    def _apply_hold(self, value: object | None) -> None:
        if value is None:
            self._focus_table()
            return
        self.hold_frames = float(cast(float, value))
        persist_default_hold_frames(self.cfg, self.hold_frames)
        self._replace_metadata_coordinator()
        cast(PickerAppHost, self.app).on_picker_hold_frames_changed(self.hold_frames)

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
        elif command == "hold":
            self.action_open_hold()
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
                "Sky Auto Player Keyboard Shortcuts",
                content,
                theme_name=self.active_theme,
            )
        )

    def action_calibrate_input_latency(self) -> None:
        if getattr(self.app, "calibration_active", False):
            return
        from sky_music.platform.win32 import window_target
        if window_target.get_sky_window() is not None:
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
            "This measures host-side injected Raw Input delivery, not Sky polling, rendering, or audio onset.\n\n"
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

        from sky_music.platform.win32.native_calibration import (
            run_published_native_calibration,
        )
        from sky_music.ui.textual_app.modals import CalibrationProgressModal

        if TYPE_CHECKING:
            from sky_music.ui.textual_app.app import SkyPickerApp

        app = cast("SkyPickerApp", self.app)

        if app.calibration_active:
            return

        progress_modal = CalibrationProgressModal(theme_name=self.active_theme)
        app.calibration_active = True
        self.app.push_screen(progress_modal)

        published = None
        calibration_error = None

        try:
            loop = asyncio.get_running_loop()
            published = await loop.run_in_executor(
                None,
                run_published_native_calibration,
            )
        except Exception as exc:
            calibration_error = exc
        finally:
            if progress_modal in self.app.screen_stack:
                with contextlib.suppress(Exception):
                    self.app.pop_screen()
            app.calibration_active = False



        if calibration_error is not None:
            self.app.push_screen(
                InfoModal(
                    "Calibration Failed",
                    f"Error running calibration:\n{calibration_error}",
                    theme_name=self.active_theme,
                )
            )
            return

        assert published is not None

        # Invalidate session/policy-dependent picker metadata so the next render
        # uses the new device_cache margin.
        invalidate_policy_metadata()
        self._replace_metadata_coordinator()

        self.app.push_screen(
            InfoModal(
                "Input Latency Calibration Complete",
                f"Device margin: {published.margin_us} \u00b5s\n"
                f"Source: {published.source}\n"
                f"Cache: {published.cache_path}\n\n"
                f"Down latency (\u00b5s): p50={published.down_us.p50}, "
                f"p90={published.down_us.p90}, p99={published.down_us.p99}\n"
                f"Up latency   (\u00b5s): p50={published.up_us.p50}, "
                f"p90={published.up_us.p90}, p99={published.up_us.p99}\n\n"
                f"Evidence: {published.evidence_kind} (SendInput \u2192 app-owned WM_INPUT).",
                theme_name=self.active_theme,
            )
        )


    def action_open_calibration(self) -> None:
        from sky_music.orchestration.calibration import (
            calibrate_timing,
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
        rec = calibrate_timing(inp)
        t = self._theme_tokens
        accent = t.accent
        info_lines = [
            f"[bold {accent}]Hold:[/]      {rec.hold_frames:.2f} frames",
            f"[bold {accent}]Tempo:[/]     {rec.tempo_scale:.2f}x",
            f"[bold {accent}]Effective:[/] {rec.recommended_hold_us / 1000:.1f}ms",
            f"[bold {accent}]Severity:[/]  {rec.severity.upper()}",
            "",
            f"[bold {accent}]Reason:[/]    {rec.reason}",
        ]
        options = [
            PickerOption(
                CalibrationChoice(rec.hold_frames, rec.tempo_scale, inp.fps),
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
            hold_frames=value.hold_frames,
            tempo_scale=value.tempo_scale,
            fps=value.fps,
        )
        self.hold_frames = value.hold_frames
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
        self.choices = []
        self.filtered = []
        self._render_table(reset_cursor=True)
        self._render_detail()
        self._update_header_tagline()
        self._render_status()
        self._catalog_generation += 1
        self._catalog_executor.submit(self._scan_catalog_worker, self._catalog_generation, force_refresh=True)
    def action_check_for_update(self) -> None:
        cast(PickerAppHost, self.app).on_picker_check_for_update()

    def action_open_update_settings(self) -> None:
        from sky_music.config import (
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

        def _on_settings_result(result: object) -> None:
            if result == "check_now":
                app.check_for_updates_worker(force=True)

        modal = UpdateSettingsModal(
            auto_check=self.cfg.update.auto_check,
            on_auto_check=_on_auto_check,
            skip_version=self.cfg.update.skip_version,
            check_interval_s=self.cfg.update.check_interval_s,
            last_check_ts=self.cfg.update.last_check_ts,
            theme_name=self.active_theme,
        )
        modal._on_clear_skip = lambda: persist_update_skip_version(self.cfg, "")

        self.app.push_screen(modal, _on_settings_result)
