"""Sky Player App container — orchestration hub for Textual picker and playback."""

from __future__ import annotations

import contextlib
import sys
from dataclasses import replace
from pathlib import Path
from typing import TYPE_CHECKING, Any, cast

from textual import events, work
from textual.app import App
from textual.screen import Screen

from sky_music import __version__ as VERSION
from sky_music.config import (
    AppConfig,
    RtPriorityMode,
    canonical_profile_name,
    load_config,
    resolve_game_fps,
)
from sky_music.domain.session_context import (
    PlaybackSessionContext,
)
from sky_music.infrastructure.background import BackgroundCleanupError, BackgroundScope
from sky_music.infrastructure.focus import Win32SkyFocusGuard
from sky_music.ui.picker import (
    SongPickerResult,
)
from sky_music.ui.picker_helpers import get_song_choices, save_theme
from sky_music.ui.picker_theme import remove_accents
from sky_music.ui.textual_app.app_state import PlaybackMode
from sky_music.ui.textual_app.display_widgets import GradientHeader
from sky_music.ui.textual_app.keymap import COMMANDS
from sky_music.ui.textual_app.playback_app import (
    PlaybackCard,
    PlaybackCommandBridge,
    SnapshotRenderer,
)
from sky_music.ui.textual_app.playback_controller import (
    PlaybackError,
    PlaybackPlan,
    prepare_playback,
    rebuild_with,
)
from sky_music.ui.textual_app.screens.picker import (
    PendingRiskDecision,
    PickerScreen,
    SongChoice,
    SongTable,
)
from sky_music.ui.textual_app.theme_css import (
    APP_CSS,
    TEXTUAL_THEME_TOKENS,
    TextualThemeTokens,
)
from sky_music.ui.textual_app.widgets import CustomFooter
from sky_music.ui.textual_app.workers import MetadataCoordinator, MetadataHandle

if TYPE_CHECKING:
    from sky_music.infrastructure.hotkeys import PlaybackControls
    from sky_music.ui.textual_app.screens.picker import CalibrationChoice


class SkyPickerApp(App[SongPickerResult | None]):
    """Song picker & playback app — thin container with shared chrome."""

    
    ansi_color = True  # type: ignore[assignment]

    AUTO_FOCUS = None  # on_mount handles focus explicitly

    CSS = APP_CSS

    BINDINGS = [
        ("q", "quit", "Quit"),
        ("escape", "cancel", "Cancel"),
        ("enter", "confirm", "Play"),
        ("/", "open_commands", "Commands"),
    ]

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
        dispatch_lead_us: int = 0,
    ) -> None:
        super().__init__()
        self.unified_mode = unified_mode
        self.controls = controls
        self.countdown_seconds = countdown_seconds
        self.cfg = cfg or load_config()
        self.scan_code_mode = scan_code_mode
        self.dispatch_lead_us = dispatch_lead_us

        self.theme_name: str
        self.active_theme: str
        self.background_mode: str
        self.profile_name: str
        self.tempo_scale: float
        self.fps: int
        self.dry_run: bool
        self.verbose_hud: bool
        self.telemetry_enabled: bool

        self._init_params(
            theme_name=theme_name,
            background_mode=background_mode,
            initial_profile=initial_profile,
            initial_tempo=initial_tempo,
            initial_fps=initial_fps,
            initial_dry_run=initial_dry_run,
        )

        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )

        # Playback state machine
        self.playback_mode = PlaybackMode.PICKER
        self._risk_decisions: tuple[PendingRiskDecision, ...] = ()
        self._risk_index = 0
        self._risk_plan: PlaybackPlan | None = None
        self._risk_picker_result: SongPickerResult | None = None
        self._transitioning_to_playback = False
        self._active_playback_commands: PlaybackCommandBridge | None = None
        self._shutting_down_playback = False

        # Song choices (shared with PickerScreen) + initial preload
        self._choices: list[SongChoice] = []
        self._pre_load_choices()

        self._picker: PickerScreen | None = None
        self.picker_scope = BackgroundScope(phase="picker")
        self.metadata: MetadataHandle | None
        if not self.unified_mode:
            self.metadata = cast(MetadataHandle, self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg)))
        else:
            self.metadata = None

        self._update_available_version: str | None = None
        self._version_indicator_applied = False

    def _init_params(
        self,
        *,
        theme_name: str | None,
        background_mode: str | None,
        initial_profile: str,
        initial_tempo: float,
        initial_fps: int | None,
        initial_dry_run: bool,
    ) -> None:
        self.profile_name = canonical_profile_name(initial_profile)
        self.tempo_scale = initial_tempo
        self.dry_run = initial_dry_run
        self.fps = resolve_game_fps(initial_fps if initial_fps is not None else self.cfg.game_fps)
        self.verbose_hud = self.cfg.verbose_hud
        self.telemetry_enabled = self.cfg.telemetry_enabled_by_default
        self.active_theme = self._normalize_theme_name(theme_name or self.cfg.theme)
        self.background_mode = self._normalize_background_mode(
            background_mode or self.cfg.ui_background_mode
        )
        self.theme_name = self.active_theme  # semantic alias

    @staticmethod
    def _normalize_theme_name(theme_name: str | None) -> str:
        from sky_music.ui.picker_theme import THEME_PRESETS
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

    def _pre_load_choices(self) -> None:
        paths = get_song_choices(force_refresh=True)
        self._choices = [
            SongChoice(path=path, search_key=remove_accents(path.stem).casefold())
            for path in paths
        ]

    def get_default_screen(self) -> Screen[SongPickerResult | None]:
        self._pre_load_choices()
        if hasattr(self, "metadata") and self.metadata is not None:
            self.metadata.refresh([c.path for c in self._choices])
        self._picker = PickerScreen(
            name="picker",
            choices=self._choices,
            theme_name=self.active_theme,
            background_mode=self.background_mode,
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            dry_run=self.dry_run,
            scan_code_mode=self.scan_code_mode,
            cfg=self.cfg,
            verbose_hud=self.verbose_hud,
            telemetry_enabled=self.telemetry_enabled,
        )
        return cast(Screen[SongPickerResult | None], self._picker)



    def on_mount(self) -> None:
        self._check_post_update_flag()
        self._set_version_indicator()
        self._restore_pending_update_indicator()
        # Auto-check after a short quiet window so the launch is not
        # immediately sent to the network — improves perceived responsiveness
        # and avoids a network hit on metered connections the instant the
        # picker is interactive. The 24h throttle in ``should_auto_check``
        # still gates the actual fetch.
        self.set_timer(3.0, self.check_for_updates_worker)

    def _set_version_indicator(self) -> None:
        """Show current version in the app bar header."""
        with contextlib.suppress(Exception):
            self.query_one("#appbar", GradientHeader).set_version(f"v{VERSION}")

    def _restore_pending_update_indicator(self) -> None:
        """If a previous session left a pending update marker in config (user
        dismissed the modal without skipping), re-apply the ``↑`` highlight
        on the app bar so the user remembers an update is available.

        Does NOT push the modal on cold start — the user themselves initiates
        the apply via ``u`` (Check for Update) when they are ready.
        """
        pending = (self.cfg.update.pending_update_version or "").strip()
        if not pending:
            return
        # Skip: the pending version may have been installed already (e.g. the
        # user manually upgraded); compare against the running version to
        # avoid a stale arrow. ``is_newer`` only treats strictly greater as
        # newer, so equal or older pending → clear the marker.
        from sky_music.domain.update_checker import is_newer
        if not is_newer(pending, VERSION):
            from sky_music.config import persist_pending_update_version
            persist_pending_update_version(self.cfg, "")
            return
        self._update_available_version = pending
        try:
            self.query_one("#appbar", GradientHeader).set_version(
                f"v{VERSION} \u2191", highlight=True, highlight_color=self._theme_tokens.accent
            )
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[app] failed to restore pending update indicator")

    # ── Test-compat delegates → PickerScreen ──────────────────────────

    @property
    def choices(self) -> list[SongChoice]:
        picker = self._find_picker_screen()
        if picker is not None:
            return picker.choices
        return self._choices

    @choices.setter
    def choices(self, value: list[SongChoice]) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.choices = value
        self._choices = value

    def _render_status(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._render_status()

    @property
    def filtered(self) -> list[SongChoice]:
        picker = self._find_picker_screen()
        if picker is not None:
            return picker.filtered
        return []

    @property
    def search_value(self) -> str:
        # Public alias for the current search box text — kept distinct from
        # ``App.query`` (the DOM query selector method) to avoid shadowing the
        # Textual base API. Tests and external callers read/write the picker's
        # search string through this property; internally, we delegate to
        # ``PickerScreen.search_query`` (a Textual ``reactive``).
        picker = self._find_picker_screen()
        if picker is not None:
            return picker.search_query
        return ""

    @search_value.setter
    def search_value(self, value: str) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.search_query = value  # type: ignore[assignment]

    @property
    def _search_timer(self):
        picker = self._find_picker_screen()
        if picker is not None:
            return picker._search_timer
        return None

    @_search_timer.setter
    def _search_timer(self, value) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._search_timer = value

    @property
    def preview_visible(self) -> bool:
        picker = self._find_picker_screen()
        if picker is not None:
            return picker.preview_visible
        return True

    @property
    def show_notes(self) -> bool:
        picker = self._find_picker_screen()
        if picker is not None:
            return picker.show_notes
        return True

    @property
    def show_risk(self) -> bool:
        picker = self._find_picker_screen()
        if picker is not None:
            return picker.show_risk
        return True

    @property
    def show_suggested(self) -> bool:
        picker = self._find_picker_screen()
        if picker is not None:
            return picker.show_suggested
        return True

    @property
    def _marked_row_key(self):
        picker = self._find_picker_screen()
        if picker is not None:
            return picker._marked_row_key
        return None

    @_marked_row_key.setter
    def _marked_row_key(self, value: object | None) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._marked_row_key = value

    def _run_command(self, value: object | None) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._run_command(value)

    def action_open_tempo(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_open_tempo()

    def action_open_fps(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_open_fps()

    def action_open_theme(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_open_theme()

    def action_open_help(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_open_help()

    def action_open_calibration(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_open_calibration()

    def action_toggle_preview(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_toggle_preview()

    def action_toggle_dry_run(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_toggle_dry_run()

    def action_toggle_hud(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_toggle_hud()

    def action_toggle_telemetry(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_toggle_telemetry()

    def action_reload_songs(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_reload_songs()

    def _focus_table(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._focus_table()

    def _sync_marker(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._sync_marker()

    def _update_header_tagline(self) -> None:
        """Sync the header tagline to reflect the current total song count."""
        total = len(self._choices)
        noun = "song" if total == 1 else "songs"
        tagline = f"precision music player  ♪ {total} {noun}"
        try:
            self.query_one("#appbar", GradientHeader).set_tagline(tagline)
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[app] failed to set header tagline")

    def _perform_search(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._perform_search()

    def _apply_responsive_columns(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._apply_responsive_columns()

    def refresh_metadata_rows(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.refresh_metadata_rows()

    # ── Picker callbacks (implement PickerAppHost protocol) ─────────
    # PickerScreen calls these public methods (typed by PickerAppHost in
    # ``screens.picker``) instead of reaching into ``self.app._on_*``. The
    # App is the picker's host: the picker emits user-intent events and the
    # App applies them to its own state. Keeping the surface typed as a
    # Protocol means future renames surface as Pyright errors, not as
    # silently dropped events at runtime.

    def on_picker_confirm(self, result: SongPickerResult) -> None:
        # Re-entrancy guard so a duplicate confirm (from App.on_key and the
        # DataTable RowSelected event both firing for the same Enter press)
        # does not start playback twice. Reset in ``_restore_picker_after_playback``.
        if getattr(self, "_transitioning_to_playback", False):
            return
        self._transitioning_to_playback = True
        if not self.unified_mode:
            self.exit(result)
        else:
            self.start_playback_workflow(result)

    def on_picker_cancel(self) -> None:
        self.action_cancel()

    def on_picker_check_for_update(self) -> None:
        self.check_for_updates_worker(force=True)

    def on_picker_open_update_settings(self) -> None:
        self._open_update_settings_modal()

    def _open_update_settings_modal(self) -> None:
        """Push the ``UpdateSettingsModal`` bound to the current config values.

        The modal calls the persistence callbacks in real time as toggles
        happen, so changes survive a restart. ``check_now`` dismisses the
        modal and triggers an immediate forced check.
        """
        from sky_music.config import (
            persist_update_auto_apply,
            persist_update_auto_check,
            persist_update_skip_version,
        )
        from sky_music.ui.textual_app.modals import UpdateSettingsModal

        def _on_auto_check(value: bool) -> None:
            persist_update_auto_check(self.cfg, value)
            if not value:
                self.notify("Auto-update check disabled.", severity="information", timeout=4)
            else:
                self.notify("Auto-update check enabled.", severity="information", timeout=4)

        def _on_auto_apply(value: bool) -> None:
            persist_update_auto_apply(self.cfg, value)
            if value:
                self.notify(
                    "Auto-apply enabled — newer releases will be downloaded and"
                    " installed automatically on the next check.",
                    severity="warning",
                    timeout=6,
                )
            else:
                self.notify("Auto-apply disabled — you'll be asked each time.", severity="information", timeout=4)

        def _on_clear_skip() -> None:
            persist_update_skip_version(self.cfg, "")
            self.notify("Skip-version cleared.", severity="information", timeout=4)

        def _on_settings_result(result: object) -> None:
            if result == "check_now":
                self.check_for_updates_worker(force=True)

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
        modal._on_clear_skip = _on_clear_skip
        self.push_screen(modal, _on_settings_result)

    def on_picker_snapshot_calibration_state(self, choice: CalibrationChoice | None) -> None:
        self._calibration_snapshot: CalibrationChoice | None = choice
        if choice is not None:
            self._apply_calibration_choice(choice)

    def on_picker_profile_changed(self, profile_name: str) -> None:
        self.profile_name = canonical_profile_name(profile_name)
        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )

    def on_picker_tempo_changed(self, tempo_scale: float) -> None:
        self.tempo_scale = tempo_scale
        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )

    def on_picker_fps_changed(self, fps: int) -> None:
        self.fps = resolve_game_fps(fps)
        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )

    def on_picker_theme_changed(self, theme_name: str, background_mode: str) -> None:  # noqa: ARG002
        # ``background_mode`` is part of the picker→host contract but the host
        # currently derives its own background mode from cfg/theme; keep the
        # parameter positional so future hosts can opt to react to it.
        self.active_theme = self._normalize_theme_name(theme_name)
        save_theme(self.active_theme)
        self.cfg.theme = self.active_theme
        self._apply_chrome_theme()

    def _apply_chrome_theme(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker._apply_theme_class()

    def on_picker_dry_run_changed(self, dry_run: bool) -> None:
        # Mirror picker state into the App so playback setup (which reads
        # ``self.dry_run``) sees user-driven toggles from the command palette.
        self.dry_run = dry_run

    def on_picker_verbose_hud_changed(self, verbose_hud: bool) -> None:
        self.verbose_hud = verbose_hud
        self.cfg.verbose_hud = verbose_hud

    def on_picker_telemetry_enabled_changed(self, telemetry_enabled: bool) -> None:
        self.telemetry_enabled = telemetry_enabled
        self.cfg.telemetry_enabled_by_default = telemetry_enabled

    def _apply_calibration_choice(self, choice: CalibrationChoice) -> None:
        from sky_music.config import persist_calibration_defaults
        persist_calibration_defaults(
            self.cfg,
            profile_name=choice.profile_name,
            tempo_scale=choice.tempo_scale,
            fps=choice.fps,
        )
        self.profile_name = canonical_profile_name(choice.profile_name)
        self.tempo_scale = choice.tempo_scale
        self.fps = resolve_game_fps(choice.fps)
        self.session = PlaybackSessionContext(
            profile_name=self.profile_name,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )

    # ── App-level action stubs (delegated from PickerScreen via Message) ─

    def action_cancel(self) -> None:
        if self.playback_mode in (PlaybackMode.ERROR, PlaybackMode.RISK):
            self._restore_picker_after_playback()
            return
        if self.playback_mode == PlaybackMode.COUNTDOWN:
            self._restore_picker_after_playback()
            return
        if self.playback_mode == PlaybackMode.PLAYING:
            self._shutting_down_playback = True
            bridge = self._active_playback_commands
            if bridge is not None:
                bridge.request("quit")
                return
        if self.playback_mode == PlaybackMode.PICKER:
            picker = self._find_picker_screen()
            if picker is not None:
                picker.action_cancel()
            return
        self.exit(None)

    def action_confirm(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_confirm()

    def action_open_commands(self) -> None:
        # Delegate to current picker screen if active
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_open_commands()
        else:
            from sky_music.ui.textual_app.modals import CommandModal
            self.push_screen(CommandModal("Commands", COMMANDS, theme_name=self.active_theme), self._on_commands_result)

    def _on_commands_result(self, value: object | None) -> None:
        if value is None:
            return
        picker = self._find_picker_screen()
        if picker is not None:
            self.call_after_refresh(picker._run_command, value)

    def action_open_profile(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_open_profile()
        else:
            from sky_music.ui.picker import PROFILES_INFO
            from sky_music.ui.textual_app.modals import OptionModal, PickerOption
            options = [PickerOption(name, f"{name} - {desc}") for name, desc in PROFILES_INFO]
            self.push_screen(OptionModal("Timing Profile", options, theme_name=self.active_theme), self._on_profile_selected)

    def _on_profile_selected(self, value: object | None) -> None:
        if value is not None:
            self.profile_name = canonical_profile_name(str(value))
            self.session = PlaybackSessionContext(
                profile_name=self.profile_name,
                tempo_scale=self.tempo_scale,
                fps=self.fps,
                scan_code_mode=self.scan_code_mode,
            )
            picker = self._find_picker_screen()
            if picker is not None:
                picker.action_open_profile()

    def _find_picker_screen(self) -> PickerScreen | None:
        return self._picker

    # ── Event Handlers ──────────────────────────────────────────────

    def on_key(self, event: events.Key) -> None:
        if self.handle_playback_card_key(event.key):
            event.stop()
            return

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
                ],
            }
        except Exception as exc:
            from sky_music.platform.win32 import inputs
            inputs.debug_log(f"[background] Cleanup error in Textual picker unmount: {exc}")
            from sky_music.orchestration.telemetry import TelemetryLogger
            resources_list: list[dict[str, Any]] = []
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

    def on_screen_resume(self, _event: events.ScreenResume) -> None:
        self.call_after_refresh(self._focus_table)

    def _set_marker(self, row_key: object | None) -> None:
        table = self.query_one("#songs", SongTable)
        t = self._theme_tokens
        if TYPE_CHECKING:
            from textual.widgets._data_table import RowKey as _RowKey
        else:
            _RowKey = Any
        if self._marked_row_key is not None:
            try:
                table.update_cell(cast(_RowKey, self._marked_row_key), "marker", t.song_icon)
            except Exception:
                from sky_music.platform.win32 import inputs
                inputs.debug_log("[app] failed to clear marker")
        if row_key is not None:
            try:
                table.update_cell(cast(_RowKey, row_key), "marker", t.pointer)
            except Exception:
                from sky_music.platform.win32 import inputs
                inputs.debug_log("[app] failed to set marker")
        self._marked_row_key = row_key

    # ── Playback Lifecycle ────────────────────────────────────────────

    def start_playback_workflow(self, picker_result: SongPickerResult) -> None:
        is_dry_run = picker_result.action == "dry_run"
        session = PlaybackSessionContext(
            profile_name=picker_result.profile_name,
            tempo_scale=picker_result.tempo_scale,
            fps=picker_result.fps,
            scan_code_mode=self.scan_code_mode,
        )
        res = prepare_playback(picker_result.song_path, session, self.cfg, is_dry_run=is_dry_run)

        if isinstance(res, PlaybackError):
            self._show_playback_error("Playback Error", res.message)
            return

        if res.risk_report.severity != "low":
            self._risk_plan = res
            self._risk_picker_result = picker_result
            self._risk_decisions = (
                PendingRiskDecision("proceed", "Proceed with current settings"),
                PendingRiskDecision(
                    "switch_profile",
                    f"Switch to recommended '{res.risk_report.suggested_profile}' profile",
                ),
                PendingRiskDecision(
                    "scale_tempo", f"Scale tempo down to {res.risk_report.suggested_tempo_scale:.2f}x"
                ),
                PendingRiskDecision("dry_run", "Dry-run first (simulate, no keystrokes)"),
                PendingRiskDecision("cancel", "Cancel and return to picker"),
            )
            self._risk_index = 0
            self._render_risk_card(res)
        else:
            self.execute_playback_plan(res, picker_result)

    def execute_playback_plan(self, plan: PlaybackPlan, picker_result: SongPickerResult) -> None:
        from sky_music.orchestration.telemetry import TelemetryLogger

        picker = self._find_picker_screen()
        try:
            if picker is not None:
                picker.quiesce()
            close_result = self.picker_scope.close_all(wait=True)
        except BackgroundCleanupError as e:
            try:
                snaps = e.result.snapshots if e.result else []
                snapshots_list = [
                    {
                        "name": snap.name, "phase": snap.phase, "state": snap.state,
                        "closed": snap.closed, "pending_count": snap.pending_count,
                        "running_count": snap.running_count,
                    }
                    for snap in snaps
                ]
                TelemetryLogger.last_picker_cleanup = {"ok": False, "error": str(e), "resources": snapshots_list}
            except Exception:
                TelemetryLogger.last_picker_cleanup = {"ok": False, "error": str(e), "resources": []}
            if picker is not None:
                picker.rearm()
            self._show_playback_error("Cleanup Error", f"Failed to stop background workers: {e}")
            return

        # Record cleanup telemetry (mirrors on_unmount behavior)
        try:
            snaps = close_result.snapshots if hasattr(close_result, 'snapshots') else []
            snapshots_list = [
                {
                    "name": snap.name, "phase": snap.phase, "state": snap.state,
                    "closed": snap.closed, "pending_count": snap.pending_count,
                    "running_count": snap.running_count,
                }
                for snap in snaps
            ]
            TelemetryLogger.last_picker_cleanup = {"ok": True, "resources": snapshots_list}
        except Exception:
            TelemetryLogger.last_picker_cleanup = {"ok": True, "resources": []}

        _last_cleanup = TelemetryLogger.last_picker_cleanup
        if _last_cleanup is not None and _picker_cleanup_failed(_last_cleanup):
            error_msg = _last_cleanup.get("error", "Unknown error during picker cleanup")
            if picker is not None:
                picker.rearm()
            self._show_playback_error("Cleanup Error", f"Failed to stop background workers: {error_msg}")
            return

        from sky_music.infrastructure.backend import DryRunBackend, WinSendInputBackend
        from sky_music.orchestration.engine import PlaybackEngine

        is_dry_run = picker_result.action == "dry_run"
        backend = DryRunBackend() if is_dry_run else WinSendInputBackend()

        renderer = SnapshotRenderer()

        main_mod = _get_main_module()
        if main_mod:
            telemetry_enabled = (
                main_mod.RUNTIME_STATE.telemetry_csv_enabled
                or self.cfg.telemetry_enabled_by_default
                or is_dry_run
            )
            use_dispatch_thread = main_mod.RUNTIME_STATE.use_dispatch_thread
            input_path_warn_us = (
                self.cfg.input_path_warn_us if main_mod.RUNTIME_STATE.check_input_path else 0
            )
            enable_timer_guard = main_mod.RUNTIME_STATE.enable_timer_guard
            enable_waitable_timer = main_mod.RUNTIME_STATE.enable_waitable_timer
            enable_gc_pause = main_mod.RUNTIME_STATE.enable_gc_pause
            enable_switch_interval_tuning = main_mod.RUNTIME_STATE.enable_switch_interval_tuning
            enable_adaptive_lead = main_mod.RUNTIME_STATE.enable_adaptive_lead
            enable_adaptive_spin = getattr(main_mod.RUNTIME_STATE, "enable_adaptive_spin", False)
            enable_event_wait = getattr(main_mod.RUNTIME_STATE, "enable_event_wait", False)
            enable_epoch_rebase = getattr(main_mod.RUNTIME_STATE, "enable_epoch_rebase", True)
            rt_priority_mode = cast(RtPriorityMode, getattr(main_mod.RUNTIME_STATE, "rt_priority_mode", "auto"))
            spin_floor_us = getattr(main_mod.RUNTIME_STATE, "spin_floor_us", None) or 700
        else:
            telemetry_enabled = self.cfg.telemetry_enabled_by_default or is_dry_run
            use_dispatch_thread = self.cfg.use_dispatch_thread
            input_path_warn_us = self.cfg.input_path_warn_us
            enable_timer_guard = True
            enable_waitable_timer = True
            enable_gc_pause = True
            enable_switch_interval_tuning = True
            enable_adaptive_lead = getattr(self.cfg, "enable_adaptive_lead", True)
            enable_adaptive_spin = getattr(self.cfg, "enable_adaptive_spin", True)
            enable_event_wait = True
            enable_epoch_rebase = True
            rt_priority_mode = cast(RtPriorityMode, getattr(self.cfg, "rt_priority_mode", "auto"))
            spin_floor_us = 700

        command_bridge = PlaybackCommandBridge(self.controls)
        self._active_playback_commands = command_bridge
        self._shutting_down_playback = False

        engine = PlaybackEngine(
            song=plan.song,
            actions=plan.actions,
            backend=backend,
            controls=command_bridge,
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
            enable_switch_interval_tuning=enable_switch_interval_tuning,
            enable_adaptive_lead=enable_adaptive_lead,
            enable_adaptive_spin=enable_adaptive_spin,
            enable_event_wait=enable_event_wait,
            enable_epoch_rebase=enable_epoch_rebase,
            rt_priority_mode=rt_priority_mode,
            dispatch_lead_us=self.dispatch_lead_us,
            spin_floor_us=spin_floor_us,
        )
        engine.telemetry.record_schedule_metadata(plan.sched_meta)

        def handle_playback_result(result: Any) -> None:
            if result == "quit":
                self._active_playback_commands = None
                self._shutting_down_playback = False
                self.exit(None)
                return
            if picker is not None:
                picker.rearm()
            if not self.unified_mode:
                self.picker_scope = BackgroundScope(phase="picker")
                self.metadata = cast(MetadataHandle, self.picker_scope.register(MetadataCoordinator(self, self.session, self.cfg)))
                self.metadata.refresh([choice.path for choice in self._choices])
            self._focus_table()
            self._restore_picker_after_playback()
            self.update_session_state(picker_result)

        def run_playback() -> None:
            from sky_music.ui.timing_guidance import fps_play_advisory
            _fps = getattr(plan.active_policy, "fps", 60)
            _short = getattr(plan.sched_meta, "sub_60fps_frame_notes", 0)
            _advisory = fps_play_advisory(fps=_fps, short_note_count=_short)
            if _advisory:
                self.notify(_advisory, severity="warning", timeout=8)
            card = self._show_playback_card(PlaybackMode.PLAYING)
            card.start_playback(
                engine=engine,
                renderer=renderer,
                song_name=plan.song.name,
                total_us=plan.sched_meta.playback_duration_us,
                violations=plan.violations,
                active_policy=plan.active_policy,
                profile_name=plan.session.display_profile_label(),
                tempo_scale=plan.session.tempo_scale,
                debug_mode=self.verbose_hud,
                result_callback=handle_playback_result,
                command_bridge=command_bridge,
            )

        if not is_dry_run:
            Win32SkyFocusGuard().focus()
        if not is_dry_run and self.countdown_seconds > 0:
            card = self._show_playback_card(PlaybackMode.COUNTDOWN)
            card.start_countdown(self.countdown_seconds, run_playback)
        else:
            run_playback()

    def update_session_state(self, picker_result: SongPickerResult) -> None:
        main_mod = _get_main_module()
        if not main_mod:
            raise RuntimeError("Could not resolve main module to update runtime state.")

        from sky_music.config import persist_playback_defaults
        from sky_music.domain.session_context import (
            PlaybackSessionContext,
            merge_session_with_overrides,
        )
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
        main_mod.RUNTIME_STATE.dry_run = picker_result.action == "dry_run"

        persist_playback_defaults(
            user_cfg,
            profile_name=picker_result.profile_name,
            tempo_scale=picker_result.tempo_scale,
            fps=picker_result.fps,
        )

    # ── Playback Card Management (inline state machine) ──────────────

    def _show_playback_card(self, mode: PlaybackMode) -> PlaybackCard:
        self.playback_mode = mode
        picker = self._find_picker_screen()
        if picker is not None:
            picker._hide_detail_and_table()
        footer = self.query_one(CustomFooter)
        card = self.query_one("#playback-card", PlaybackCard)
        footer.styles.display = "none"
        card.styles.display = "block"
        card.focus()
        return card

    def _restore_picker_after_playback(self) -> None:
        self.playback_mode = PlaybackMode.PICKER
        self._risk_decisions = ()
        self._risk_index = 0
        self._risk_plan = None
        self._risk_picker_result = None
        self._transitioning_to_playback = False
        self._active_playback_commands = None
        self._shutting_down_playback = False
        self.query_one("#playback-card", PlaybackCard).styles.display = "none"
        self.query_one(CustomFooter).styles.display = "block"
        picker = self._find_picker_screen()
        if picker is not None:
            picker._show_detail_and_table()

    def _show_playback_error(self, title: str, message: str) -> None:
        card = self._show_playback_card(PlaybackMode.ERROR)
        card.show_error(title, message)

    def _handle_risk_decision_by_index(self, index: int) -> None:
        if not 0 <= index < len(self._risk_decisions):
            return
        self._handle_risk_decision(self._risk_decisions[index].decision)

    def _move_risk_selection(self, delta: int) -> None:
        if not self._risk_decisions or self._risk_plan is None:
            return
        self._risk_index = (self._risk_index + delta) % len(self._risk_decisions)
        self._render_risk_card(self._risk_plan)

    def _render_risk_card(self, plan: PlaybackPlan) -> None:
        card = self._show_playback_card(PlaybackMode.RISK)
        card.show_risk(
            plan.risk_report.severity,
            tuple(plan.risk_report.recommendations),
            tuple(decision.label for decision in self._risk_decisions),
            self._risk_index,
        )

    def _handle_risk_decision(self, decision: str | None) -> None:
        plan = self._risk_plan
        picker_result = self._risk_picker_result
        if plan is None or picker_result is None:
            self._restore_picker_after_playback()
            return

        if decision == "proceed":
            self.execute_playback_plan(plan, picker_result)
        elif decision in {"switch_profile", "scale_tempo", "dry_run"}:
            rebuild_kwargs: dict[str, Any]
            if decision == "switch_profile":
                rebuild_kwargs = {"profile": plan.risk_report.suggested_profile}
            elif decision == "scale_tempo":
                rebuild_kwargs = {"tempo": plan.risk_report.suggested_tempo_scale}
            else:
                rebuild_kwargs = {"is_dry_run": True}

            result = rebuild_with(plan, cfg=self.cfg, **rebuild_kwargs)
            if isinstance(result, PlaybackError):
                self._show_playback_error("Rebuild Error", result.message)
                return

            # Apply new plan (profile/tempo/dry-run already baked in)
            if rebuild_kwargs.get("is_dry_run"):
                picker_result = replace(picker_result, action="dry_run")
            self._risk_plan = result
            self.execute_playback_plan(result, picker_result)
        elif decision == "cancel":
            self._restore_picker_after_playback()
        else:
            self._restore_picker_after_playback()

    def handle_playback_card_key(self, key: str) -> bool:
        if self.playback_mode == PlaybackMode.RISK:
            if key == "up":
                self._move_risk_selection(-1)
                return True
            if key == "down":
                self._move_risk_selection(1)
                return True
            if key in {"1", "2", "3", "4", "5"}:
                self._handle_risk_decision_by_index(int(key) - 1)
                return True
            if key == "enter":
                self._handle_risk_decision_by_index(self._risk_index)
                return True
            if key == "escape":
                self._handle_risk_decision("cancel")
                return True
        if self.playback_mode == PlaybackMode.PLAYING and key in {"up", "down", "enter"}:
            return True
        if self.playback_mode in {PlaybackMode.ERROR, PlaybackMode.COUNTDOWN}:
            if key == "escape":
                self._restore_picker_after_playback()
                return True
            if key in {"up", "down", "enter"}:
                return True
        return False

    # ── Update Service ────────────────────────────────────────────────

    def _check_post_update_flag(self) -> None:
        import shutil

        from sky_music.infrastructure.update_installer import (
            find_old_backups,
            install_dir_for_frozen,
            post_update_flag_path,
        )
        with contextlib.suppress(Exception):
            flag = post_update_flag_path(install_dir_for_frozen())
            if flag.exists():
                self.notify(
                    f"Sky Player successfully updated to v{VERSION}!",
                    severity="information",
                    timeout=5,
                )
                flag.unlink()
                # Sweep ``.old.{guid}`` backups left by past atomic swaps.
                # Best-effort: ignore_errors so a single stale / locked dir
                # does not block cleanup of the rest.
                install_dir = install_dir_for_frozen()
                for backup in find_old_backups(install_dir):
                    shutil.rmtree(backup, ignore_errors=True)

    @work(thread=True)
    def check_for_updates_worker(self, force: bool = False) -> None:
        # ``--no-update`` (RUNTIME_STATE.update_disabled) suppresses the
        # automatic launch check only; the manual ``force`` path from the
        # ``u`` key still works so the user can check on demand.
        import main as main_mod
        from sky_music.orchestration.update_service import (
            check_for_update,
            record_successful_check,
            should_auto_check,
        )
        update_disabled = bool(getattr(main_mod.RUNTIME_STATE, "update_disabled", False))
        if not force and update_disabled:
            return
        if not force and not should_auto_check(self.cfg):
            return

        result = check_for_update(self.cfg, current_version=VERSION)
        if result.error is None:
            record_successful_check(self.cfg)
            if result.update is None and self.cfg.update.pending_update_version:
                from sky_music.config import persist_pending_update_version
                persist_pending_update_version(self.cfg, "")
                self.call_from_thread(self._clear_pending_update_indicator)
        else:
            from sky_music.orchestration.update_service import record_check_error
            record_check_error(self.cfg)
            if force:
                # Manual check fails visibly: surface the error and let the
                # short-backoff gate schedule an automatic retry later.
                self.call_from_thread(
                    self.notify,
                    f"Update check failed: {result.error}",
                    severity="error",
                    timeout=6,
                )

        if result.update is not None:
            self.call_from_thread(self._prompt_update, result)
        elif result.error is None and force:
            self.call_from_thread(
                self.notify,
                f"Sky Player v{VERSION} is up to date.",
                severity="information",
                timeout=4,
            )

    def _prompt_update(self, result: Any) -> None:
        if result.update is None:
            return

        from sky_music.config import persist_pending_update_version

        self._update_available_version = result.update.latest_version
        persist_pending_update_version(self.cfg, result.update.latest_version)
        try:
            self.query_one("#appbar", GradientHeader).set_version(
                f"v{VERSION} \u2191", highlight=True, highlight_color=self._theme_tokens.accent
            )
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[app] failed to set update version indicator")

        if self.cfg.update.auto_apply:
            if self.playback_mode != PlaybackMode.PICKER:
                self.notify(
                    f"Update v{result.update.latest_version} available — "
                    "exit playback to apply on next restart.",
                    severity="warning",
                    timeout=6,
                )
                return
            self.notify(
                f"Downloading v{result.update.latest_version}...",
                severity="information",
                timeout=5,
            )
            self.download_and_apply_update_worker(result.update)
            return

        self.notify(
            f"Update v{result.update.latest_version} available! (press Esc to dismiss)",
            severity="information",
            timeout=6,
        )
        from sky_music.ui.textual_app.modals import UpdateModal
        modal = UpdateModal(
            latest_version=result.update.latest_version,
            current_version=result.current_version,
            release_notes=getattr(result.update, "release_notes", "") or "",
            published_at=getattr(result.update, "published_at", "") or "",
            theme_name=self.active_theme,
        )
        self.push_screen(modal, lambda res: self._handle_update_response(res, result.update))

    def _clear_pending_update_indicator(self) -> None:
        self._update_available_version = None
        try:
            self.query_one("#appbar", GradientHeader).set_version(f"v{VERSION}")
        except Exception:
            from sky_music.platform.win32 import inputs
            inputs.debug_log("[app] failed to clear update indicator")

    def _handle_update_response(self, response: str | None, release: Any) -> None:
        from sky_music.orchestration.update_service import record_skip
        if response == "skip":
            record_skip(self.cfg, release.latest_version)
            self.notify(f"Skipped version {release.latest_version}", timeout=3)
        elif response == "download":
            self.notify("Downloading update... Please wait.", severity="information", timeout=5)
            self.download_and_apply_update_worker(release)
        elif response == "github":
            self._open_update_url(release)

    def _open_update_url(self, release: Any) -> None:
        url = getattr(release, "html_url", "") or ""
        if not url:
            self.notify("No release page available.", severity="error", timeout=4)
            return
        import webbrowser
        webbrowser.open(url)
        self.notify(f"Download page opened in browser: {url}", timeout=8)

    def _apply_staged(self, staged: Any, install_dir: Path | None) -> None:
        """Apply a staged update on the UI thread and relaunch.

        ``apply_staged_update`` calls ``sys.exit(0)`` after launching the
        detached apply batch, so any code after it is unreachable. Wrap the
        call so install-side errors surface to the user as a notification
        rather than a silent crash.
        """
        from sky_music.infrastructure.update_installer import UpdateInstallerError
        from sky_music.orchestration.update_service import apply_staged_update
        try:
            apply_staged_update(staged, install_dir=install_dir)
        except UpdateInstallerError as exc:
            self.notify(f"Update failed: {exc}", severity="error", timeout=6)
        except Exception as exc:
            from sky_music.platform.win32 import inputs
            inputs.debug_log(f"[app] apply_staged_update raised: {exc!r}")
            self.notify(f"Update failed: {exc}", severity="error", timeout=6)

    @work(thread=True, exclusive=True)
    def download_and_apply_update_worker(self, release: Any) -> None:
        from sky_music.orchestration.update_service import download_and_verify_update
        from sky_music.ui.textual_app.modals import UpdateProgressModal

        if self.playback_mode != PlaybackMode.PICKER:
            self.call_from_thread(
                self.notify,
                "Update deferred — exit playback first, then re-run Check for Update.",
                severity="warning",
                timeout=6,
            )
            return

        install_dir: Path | None = None
        if getattr(sys, "frozen", False):
            from sky_music.infrastructure.update_installer import install_dir_for_frozen
            install_dir = install_dir_for_frozen()

        latest_version = getattr(release, "latest_version", "?")
        # Push the progress modal synchronously on the UI thread; the worker
        # keeps a handle so the progress callback can update it via
        # call_from_thread. Once pushed, the worker proceeds to download.
        modal_holder: dict[str, UpdateProgressModal | None] = {"modal": None}

        def _blocked_close_hint() -> None:
            # User pressed Esc/Enter mid-download — explain why it's gated.
            with contextlib.suppress(Exception):
                self.notify(
                    "Update in progress — please wait for it to finish.",
                    severity="warning",
                    timeout=3,
                )

        def _push_modal() -> None:
            modal = UpdateProgressModal(
                latest_version=latest_version,
                current_version=VERSION,
                theme_name=self.active_theme,
            )
            modal.on_blocked_close_attempt = _blocked_close_hint
            modal_holder["modal"] = modal
            self.push_screen(modal, lambda _res: _on_modal_dismissed())

        def _on_modal_dismissed() -> None:
            # Drop the worker's reference so later callbacks are no-ops; the
            # modal itself is also defensively guarded by ``_closed`` in
            # ``update_progress`` / ``set_status``.
            modal_holder["modal"] = None

        self.call_from_thread(_push_modal)

        def _progress(downloaded: int, total: int | None) -> None:
            modal = modal_holder["modal"]
            if modal is None:
                return
            self.call_from_thread(modal.update_progress, downloaded, total)

        outcome = download_and_verify_update(release, install_dir=install_dir, progress=_progress)
        if outcome.staged:
            self.call_from_thread(self._apply_staged, outcome.staged, install_dir)
        else:
            def _show_failure(modal: UpdateProgressModal | None, error: str) -> None:
                if modal is not None:
                    # Re-arm close + show the failure reason; the user can now
                    # dismiss with Esc/Enter. The success path exits the
                    # process before ever reaching this branch.
                    modal.allow_close = True
                    modal.set_status(error, severity="error")
                else:
                    self.notify(f"Update failed: {error}", severity="error", timeout=6)
            self.call_from_thread(_show_failure, modal_holder["modal"], outcome.error or "unknown error")


# ── Helpers ─────────────────────────────────────────────────────────

def _get_main_module():
    import sys as _sys
    main_mod = _sys.modules.get("__main__")
    if main_mod and hasattr(main_mod, "RUNTIME_STATE"):
        return main_mod
    try:
        import main
        return main
    except ImportError:
        return None


def _picker_cleanup_failed(cleanup: dict | None) -> bool:
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
    from sky_music.orchestration.telemetry import TelemetryLogger

    app = SkyPickerApp(
        theme_name=theme_name,
        background_mode=background_mode,
        initial_profile=initial_profile,
        initial_tempo=initial_tempo,
        initial_fps=initial_fps,
        initial_dry_run=initial_dry_run,
        scan_code_mode=scan_code_mode,
    )
    TelemetryLogger.last_picker_cleanup = None
    result = app.run()

    _last_cleanup = TelemetryLogger.last_picker_cleanup
    if _last_cleanup is not None and _picker_cleanup_failed(_last_cleanup):
        error = _last_cleanup.get("error", "unknown error")
        raise RuntimeError(f"picker background worker cleanup failed before playback: {error}")
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
    dispatch_lead_us: int = 0,
) -> int:
    from sky_music.orchestration.telemetry import TelemetryLogger

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
        dispatch_lead_us=dispatch_lead_us,
    )
    TelemetryLogger.last_picker_cleanup = None

    app.run()

    _last_cleanup = TelemetryLogger.last_picker_cleanup
    if _last_cleanup is not None and _picker_cleanup_failed(_last_cleanup):
        error = _last_cleanup.get("error", "unknown error")
        raise RuntimeError(f"picker background worker cleanup failed: {error}")
    return 0


if __name__ == "__main__":
    choose_song_interactively_textual()