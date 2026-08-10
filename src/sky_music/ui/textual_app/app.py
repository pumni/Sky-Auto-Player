"""Sky Auto Player App container — orchestration hub for Textual picker and playback."""

from __future__ import annotations

import contextlib
import webbrowser
from dataclasses import replace
from typing import TYPE_CHECKING, Any, cast

from textual import events, work
from textual.app import App
from textual.screen import Screen

from sky_music import __version__ as VERSION
from sky_music.config import (
    AppConfig,
    load_config,
    resolve_game_fps,
)
from sky_music.domain.session_context import (
    PlaybackSessionContext,
)
from sky_music.infrastructure.background import BackgroundCleanupError
from sky_music.ui.picker import (
    SongPickerResult,
)
from sky_music.ui.picker_helpers import save_theme
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
)
from sky_music.ui.textual_app.theme_css import (
    APP_CSS,
    TEXTUAL_THEME_TOKENS,
    TextualThemeTokens,
)
from sky_music.ui.textual_app.widgets import CustomFooter

# MetadataCoordinator is imported for test-monkeypatch compatibility: tests that
# need to stub the coordinator patch ``app_module.MetadataCoordinator`` (to avoid
# spawning real background threads during Textual pilot tests). The App itself no
# longer creates a coordinator — PickerScreen is the sole owner (Fix 4.2).
from sky_music.ui.textual_app.workers import MetadataCoordinator as MetadataCoordinator

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
        initial_hold_frames: float = 1.0,
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
        self.cfg = cfg or load_config()
        self.scan_code_mode = scan_code_mode

        self.theme_name: str
        self.active_theme: str
        self.background_mode: str
        self.hold_frames: float
        self.tempo_scale: float
        self.fps: int
        self.dry_run: bool
        self.verbose_hud: bool
        self.telemetry_enabled: bool

        self._init_params(
            theme_name=theme_name,
            background_mode=background_mode,
            initial_hold_frames=initial_hold_frames,
            initial_tempo=initial_tempo,
            initial_fps=initial_fps,
            initial_dry_run=initial_dry_run,
        )

        self.session = PlaybackSessionContext(
            hold_frames=self.hold_frames,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )

        # Playback state machine
        self.playback_mode = PlaybackMode.PICKER
        self.calibration_active = False
        self._risk_decisions: tuple[PendingRiskDecision, ...] = ()

        self._risk_index = 0
        self._risk_plan: PlaybackPlan | None = None
        self._risk_picker_result: SongPickerResult | None = None
        self._transitioning_to_playback = False
        self._active_playback_commands: PlaybackCommandBridge | None = None
        self._shutting_down_playback = False

        # Song choices cache — populated lazily by PickerScreen.on_mount()
        # so the App constructor does NOT scan the songs/ directory.
        self._choices: list[SongChoice] = []

        self._picker: PickerScreen | None = None

        self._update_available_version: str | None = None
        self._version_indicator_applied = False

    def _init_params(
        self,
        *,
        theme_name: str | None,
        background_mode: str | None,
        initial_hold_frames: float,
        initial_tempo: float,
        initial_fps: int | None,
        initial_dry_run: bool,
    ) -> None:
        self.hold_frames = float(initial_hold_frames)
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



    def get_default_screen(self) -> Screen[SongPickerResult | None]:
        self._picker = PickerScreen(
            name="picker",
            choices=None,
            theme_name=self.active_theme,
            background_mode=self.background_mode,
            hold_frames=self.hold_frames,
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
        self._set_version_indicator()
        self._report_last_update_result()
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

    def _update_header_tagline(self) -> None:
        """Sync the header tagline to reflect the current total song count."""
        total = len(self._choices)
        noun = "song" if total == 1 else "songs"
        tagline = f"precision music player  ♪ {total} {noun}"
        try:
            self.query_one("#appbar", GradientHeader).set_tagline(tagline)
        except Exception:
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[app] failed to set header tagline")

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

        def _on_clear_skip() -> None:
            persist_update_skip_version(self.cfg, "")
            self.notify("Skip-version cleared.", severity="information", timeout=4)

        def _on_settings_result(result: object) -> None:
            if result == "check_now":
                self.check_for_updates_worker(force=True)

        modal = UpdateSettingsModal(
            auto_check=self.cfg.update.auto_check,
            on_auto_check=_on_auto_check,
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

    def on_picker_hold_frames_changed(self, hold_frames: float) -> None:
        self.hold_frames = hold_frames
        self.session = PlaybackSessionContext(
            hold_frames=self.hold_frames,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )

    def on_picker_tempo_changed(self, tempo_scale: float) -> None:
        self.tempo_scale = tempo_scale
        self.session = PlaybackSessionContext(
            hold_frames=self.hold_frames,
            tempo_scale=self.tempo_scale,
            fps=self.fps,
            scan_code_mode=self.scan_code_mode,
        )

    def on_picker_fps_changed(self, fps: int) -> None:
        self.fps = resolve_game_fps(fps)
        self.session = PlaybackSessionContext(
            hold_frames=self.hold_frames,
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
            hold_frames=choice.hold_frames,
            tempo_scale=choice.tempo_scale,
            fps=choice.fps,
        )
        self.hold_frames = choice.hold_frames
        self.tempo_scale = choice.tempo_scale
        self.fps = resolve_game_fps(choice.fps)
        self.session = PlaybackSessionContext(
            hold_frames=self.hold_frames,
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

    def action_open_hold(self) -> None:
        picker = self._find_picker_screen()
        if picker is not None:
            picker.action_open_hold()
        else:
            from sky_music.ui.picker import HOLD_OPTIONS
            from sky_music.ui.textual_app.modals import OptionModal, PickerOption
            options = [PickerOption(value, f"{value:.2f} frames - {desc}") for value, desc in HOLD_OPTIONS]
            self.push_screen(OptionModal("Hold Duration", options, theme_name=self.active_theme), self._on_hold_selected)

    def _on_hold_selected(self, value: object | None) -> None:
        if value is not None:
            self.hold_frames = float(cast(float, value))
            self.session = PlaybackSessionContext(
                hold_frames=self.hold_frames,
                tempo_scale=self.tempo_scale,
                fps=self.fps,
                scan_code_mode=self.scan_code_mode,
            )
            picker = self._find_picker_screen()
            if picker is not None:
                picker.action_open_hold()

    def _find_picker_screen(self) -> PickerScreen | None:
        return self._picker

    # ── Event Handlers ──────────────────────────────────────────────

    def on_key(self, event: events.Key) -> None:
        if self.calibration_active:
            event.stop()
            event.prevent_default()
            return
        if self.handle_playback_card_key(event.key):
            event.stop()
            return




    def on_screen_resume(self, _event: events.ScreenResume) -> None:
        self.call_after_refresh(self._focus_table)

    # ── Playback Lifecycle ────────────────────────────────────────────

    def start_playback_workflow(self, picker_result: SongPickerResult) -> None:
        is_dry_run = picker_result.action == "dry_run"
        session = PlaybackSessionContext(
            hold_frames=picker_result.hold_frames,
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
                    "switch_hold",
                    f"Use recommended hold {res.risk_report.suggested_hold_frames:.2f} frames",
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
        close_result = None
        try:
            if picker is not None:
                close_result = picker.quiesce()
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
            snaps = getattr(close_result, "snapshots", [])
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

        from sky_music.orchestration.engine import PlaybackEngine

        is_dry_run = picker_result.action == "dry_run"

        renderer = SnapshotRenderer()

        main_mod = _get_main_module()
        telemetry_enabled = (
            bool(main_mod and main_mod.RUNTIME_STATE.telemetry_csv_enabled)
            or self.cfg.telemetry_enabled_by_default
            or is_dry_run
        )

        command_bridge = PlaybackCommandBridge(self.controls)
        self._active_playback_commands = command_bridge
        self._shutting_down_playback = False

        engine = PlaybackEngine(
            song=plan.song,
            actions=plan.actions,
            dry_run=is_dry_run,
            controls=command_bridge,
            renderer=renderer,
            telemetry_enabled=telemetry_enabled,
            require_focus=not is_dry_run,
            hold_label=plan.session.display_hold_label(),
            hold_frames=plan.session.hold_frames,
            game_fps=int(plan.active_policy.fps),
            tempo_scale=plan.session.tempo_scale,
            focus_restore_grace_us=int(plan.active_policy.focus_restore_grace_us),
            min_hold_us=int(plan.active_policy.min_hold_us),
            min_hold_margin_us=int(plan.active_policy.min_hold_margin_us),
            min_hold_margin_source=plan.active_policy.min_hold_margin_source,
        )
        engine.telemetry.record_schedule_metadata(plan.sched_meta)

        def handle_playback_result(result: Any) -> None:
            if result == "quit":
                self._active_playback_commands = None
                self._shutting_down_playback = False
                self.exit(None)
                return
            playback_error = None
            if isinstance(result, str) and result.startswith("error:"):
                playback_error = result.removeprefix("error:").strip()
            if picker is not None:
                picker.rearm()
            # Fix 4.2: App no longer re-creates a coordinator after playback.
            # picker.rearm() above already creates a fresh coordinator owned by
            # PickerScreen. An App-level coordinator here was the post-playback
            # dual-coordinator race (non-unified mode).
            self._focus_table()
            self._restore_picker_after_playback()
            if playback_error is not None:
                self._show_playback_error("Playback Error", playback_error)
            self.update_session_state_from_plan(
                plan,
                is_dry_run=picker_result.action == "dry_run",
            )

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
                hold_label=plan.session.display_hold_label(),
                tempo_scale=plan.session.tempo_scale,
                debug_mode=self.verbose_hud,
                result_callback=handle_playback_result,
                command_bridge=command_bridge,
                schedule_warnings=plan.sched_meta.warnings,
            )

        if not is_dry_run:
            # Discovery is read-only at startup. Do not steal foreground focus;
            # the explicit refocus hotkey is the only path that calls the
            # minimal Windows foreground API.
            from sky_music.platform.win32 import window_target

            if not window_target.is_sky_active():
                self.notify(
                    "Sky is not focused. Click the Sky window before playback; "
                    f"use {self.controls.refocus.display if self.controls is not None else 'your configured refocus key'} only if you explicitly request refocus.",
                    severity="warning",
                    timeout=8,
                )
            try:
                if self.controls is not None:
                    self.controls.start()
            except Exception as exc:
                if self.controls is not None:
                    with contextlib.suppress(Exception):
                        self.controls.close()
                self._active_playback_commands = None
                self._shutting_down_playback = False
                if picker is not None:
                    picker.rearm()
                self._restore_picker_after_playback()
                self._show_playback_error(
                    "Hotkey Registration Error",
                    f"Playback was not started because global hotkeys could not be registered: {exc}",
                )
                return
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
            main_mod.RUNTIME_STATE.session or PlaybackSessionContext.default(
                tempo_scale=main_mod.RUNTIME_STATE.tempo_scale,
                scan_code_mode=main_mod.RUNTIME_STATE.scan_code_mode,
            ),
            hold_frames=picker_result.hold_frames,
            tempo=picker_result.tempo_scale,
            fps=picker_result.fps,
        )
        main_mod.RUNTIME_STATE.apply_session(updated_session, user_cfg)
        main_mod.RUNTIME_STATE.dry_run = picker_result.action == "dry_run"

        persist_playback_defaults(
            user_cfg,
            hold_frames=picker_result.hold_frames,
            tempo_scale=picker_result.tempo_scale,
            fps=picker_result.fps,
        )

    def update_session_state_from_plan(
        self,
        plan: PlaybackPlan,
        *,
        is_dry_run: bool,
    ) -> None:
        """Persist the exact session that produced the effective playback plan."""
        main_mod = _get_main_module()
        if not main_mod:
            raise RuntimeError("Could not resolve main module to update runtime state.")

        from sky_music.config import persist_playback_defaults

        user_cfg = load_config()
        main_mod.RUNTIME_STATE.apply_session(plan.session, user_cfg)
        main_mod.RUNTIME_STATE.dry_run = is_dry_run
        persist_playback_defaults(
            user_cfg,
            hold_frames=plan.session.hold_frames,
            tempo_scale=plan.session.tempo_scale,
            fps=plan.session.fps,
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
        elif decision in {"switch_hold", "scale_tempo", "dry_run"}:
            rebuild_kwargs: dict[str, Any]
            if decision == "switch_hold":
                rebuild_kwargs = {"hold_frames": plan.risk_report.suggested_hold_frames}
            elif decision == "scale_tempo":
                rebuild_kwargs = {"tempo": plan.risk_report.suggested_tempo_scale}
            else:
                rebuild_kwargs = {"is_dry_run": True}

            result = rebuild_with(plan, cfg=self.cfg, **rebuild_kwargs)
            if isinstance(result, PlaybackError):
                self._show_playback_error("Rebuild Error", result.message)
                return

            # Apply new plan (hold/tempo/dry-run already baked in)
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
            from sky_music.config import persist_update_last_notified
            persist_update_last_notified(self.cfg, result.update.latest_version)
            self.call_from_thread(self._push_update_banner_modal, result.update)
        elif result.error is None and force:
            self.call_from_thread(
                self.notify,
                f"Sky Auto Player v{VERSION} is up to date.",
                severity="information",
                timeout=4,
            )

    def _push_update_banner_modal(self, update: Any) -> None:
        if update is None:
            return

        self._update_available_version = update.latest_version
        try:
            self.query_one("#appbar", GradientHeader).set_version(
                f"v{VERSION} \u2191", highlight=True, highlight_color=self._theme_tokens.accent
            )
        except Exception:
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[app] failed to set update version indicator")

        from sky_music.ui.textual_app.modals import UpdateBannerModal
        modal = UpdateBannerModal(
            latest_version=update.latest_version,
            current_version=VERSION,
            release_notes=getattr(update, "release_notes", "") or "",
            published_at=getattr(update, "published_at", "") or "",
            theme_name=self.active_theme,
        )
        self.push_screen(modal, lambda res: self._handle_update_response(res, update))

    def _restore_pending_update_indicator(self) -> None:
        notified = self.cfg.update.last_notified_version
        from sky_music.domain.update_checker import UpdateInfo, is_newer
        if notified and is_newer(notified, VERSION):
            mock_update = UpdateInfo(
                latest_version=notified,
                download_url="",
                release_notes="",
                html_url="",
                published_at=""
            )
            self._push_update_banner_modal(mock_update)

    def _clear_pending_update_indicator(self) -> None:
        self._update_available_version = None
        try:
            self.query_one("#appbar", GradientHeader).set_version(f"v{VERSION}")
        except Exception:
            from sky_music.platform.win32 import window_target
            window_target.debug_log("[app] failed to clear update indicator")

    def _handle_update_response(self, response: str | None, release: Any) -> None:
        from sky_music.orchestration.update_service import record_skip
        if response == "skip":
            record_skip(self.cfg, release.latest_version)
            self.notify(f"Skipped version {release.latest_version}", timeout=3)
        elif response == "update":
            self._launch_native_update(release)
        elif response == "github":
            self._open_manual_update_page()

    def _launch_native_update(self, release: Any) -> None:
        """Stage and spawn the native updater, then exit only after spawn."""
        import sys
        from pathlib import Path

        from sky_music.infrastructure.update_launcher import (
            UpdateLaunchError,
            UpdateLaunchRequest,
            launch_update,
        )

        if getattr(sys, "frozen", False):
            install_root = Path(sys.executable).resolve().parent
        else:
            install_root = Path(__file__).resolve().parents[4]
        try:
            launch_update(
                UpdateLaunchRequest(
                    install_root=install_root,
                    current_version=VERSION,
                    target_version=release.latest_version,
                    channel=self.cfg.update.channel,
                    restart=True,
                )
            )
        except UpdateLaunchError as exc:
            self.notify(
                f"Update could not start: {exc}. Choose Open GitHub Releases for manual update.",
                severity="error",
                timeout=10,
            )
            return
        self.notify("Update started. Sky Auto Player will restart when it completes.", timeout=6)
        self.exit()

    def _report_last_update_result(self) -> None:
        from sky_music.infrastructure.update_runtime import consume_last_result

        result = consume_last_result()
        if result is None:
            return
        if result.status == "success":
            self.notify(
                f"Updated successfully to v{result.target_version}.",
                severity="information",
                timeout=6,
            )
        elif result.status == "rolled_back":
            self.notify(
                f"Update to v{result.target_version} was rolled back: {result.message}",
                severity="error",
                timeout=10,
            )
        elif result.status == "failure":
            self.notify(
                f"Update to v{result.target_version} failed: {result.message}",
                severity="error",
                timeout=10,
            )

    def _open_manual_update_page(self) -> None:
        """Open only the project's fixed official Releases page."""
        from sky_music.orchestration.update_service import OFFICIAL_RELEASES_URL

        try:
            opened = webbrowser.open(OFFICIAL_RELEASES_URL, new=2)
        except OSError as exc:
            self.notify(f"Could not open GitHub Releases: {exc}", severity="error", timeout=8)
            return
        if not opened:
            self.notify(
                f"Open this URL manually: {OFFICIAL_RELEASES_URL}",
                severity="warning",
                timeout=8,
            )



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
    initial_hold_frames: float = 1.0,
    initial_tempo: float = 1.0,
    initial_fps: int | None = None,
    initial_dry_run: bool = False,
    scan_code_mode: str = "physical",
) -> SongPickerResult | None:
    from sky_music.orchestration.telemetry import TelemetryLogger

    app = SkyPickerApp(
        theme_name=theme_name,
        background_mode=background_mode,
        initial_hold_frames=initial_hold_frames,
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
    initial_hold_frames: float = 1.0,
    initial_tempo: float = 1.0,
    initial_fps: int | None = None,
    initial_dry_run: bool = False,
    scan_code_mode: str = "physical",
    controls: PlaybackControls | None = None,
    countdown_seconds: int = 3,
) -> int:
    from sky_music.orchestration.telemetry import TelemetryLogger

    app = SkyPickerApp(
        theme_name=theme_name,
        background_mode=background_mode,
        initial_hold_frames=initial_hold_frames,
        initial_tempo=initial_tempo,
        initial_fps=initial_fps,
        initial_dry_run=initial_dry_run,
        scan_code_mode=scan_code_mode,
        unified_mode=True,
        controls=controls,
        countdown_seconds=countdown_seconds,
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
