from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any

import pytest

from sky_music.config import AppConfig
from sky_music.ui.picker import SongPickerResult
from sky_music.ui.picker_helpers import get_song_choices
from sky_music.ui.picker_metadata import SongUiMetadata
from sky_music.ui.textual_app import app as app_module
from sky_music.ui.textual_app.app import (
    TEXTUAL_THEME_TOKENS,
    SkyPickerApp,
    SongChoice,
    _metadata_cells,
    _picker_cleanup_failed,
    choose_song_interactively_textual,
    rank_song_choices,
)
from sky_music.ui.picker_theme import THEME_PRESETS, remove_accents


SONGS = [
    Path("songs/Alpha.json"),
    Path("songs/Beta.json"),
    Path("songs/Gamma.json"),
]


class FakeMetadataCoordinator:
    instances: list["FakeMetadataCoordinator"] = []

    def __init__(self, *_args: Any, **_kwargs: Any) -> None:
        self.refreshed: list[list[Path]] = []
        self.close_waits: list[bool] = []
        self.shutdown_started = False
        self.closed = False
        self.instances.append(self)

    @property
    def name(self) -> str:
        return "textual-picker-metadata"

    @property
    def phase(self) -> str:
        return "picker"

    def refresh(self, paths: list[Path]) -> None:
        self.refreshed.append(paths)

    def cancel(self) -> None:
        self.shutdown_started = True

    def close(self, *, wait: bool = False) -> None:
        self.close_waits.append(wait)
        self.shutdown_started = True
        if wait:
            self.closed = True

    def snapshot(self) -> WorkerSnapshot:
        from sky_music.infrastructure.background import WorkerSnapshot
        return WorkerSnapshot(
            name=self.name,
            phase=self.phase,
            closed=self.closed,
            pending_count=0,
            running_count=0,
        )


def run_picker(coro: Any) -> Any:
    return asyncio.run(coro)


async def _run_app(actions: Any) -> SkyPickerApp:
    app = SkyPickerApp(initial_dry_run=True, cfg=AppConfig())
    async with app.run_test() as pilot:
        await pilot.pause()
        await actions(app, pilot)
    return app


def test_textual_picker_opens_with_all_songs(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        table = app.query_one("#songs")
        assert table.row_count == len(SONGS)
        await pilot.pause()
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None
    assert FakeMetadataCoordinator.instances[0].refreshed == [SONGS]
    assert FakeMetadataCoordinator.instances[0].close_waits == [True]


def test_textual_picker_filters_and_selects_current_row(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        await pilot.click("#search")
        await pilot.press("a")
        table = app.query_one("#songs")
        assert table.row_count == 3
        app.set_focus(table)
        await pilot.press("down")
        await pilot.press("enter")

    app = run_picker(_run_app(actions))
    assert app.return_value is not None
    assert app.return_value.song_path == SONGS[1]
    assert app.return_value.action == "dry_run"


def test_search_typing_shortcut_letter_does_not_open_modal(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        await pilot.click("#search")
        await pilot.press("p")
        await pilot.pause()
        search = app.query_one("#search")
        assert getattr(search, "value") == "p"
        assert type(app.screen).__name__ == "Screen"
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None


def test_rank_song_choices_empty_query_preserves_order() -> None:
    choices = [
        SongChoice(path=path, search_key=remove_accents(path.stem).casefold())
        for path in SONGS
    ]

    assert rank_song_choices(choices, "") == choices


def test_rank_song_choices_handles_typo_with_fuzzy_score() -> None:
    choices = [
        SongChoice(Path("songs/Diamonds.json"), "diamonds"),
        SongChoice(Path("songs/Dandelions.json"), "dandelions"),
        SongChoice(Path("songs/Despacito.json"), "despacito"),
    ]

    ranked = rank_song_choices(choices, "dimonds")
    assert ranked
    assert ranked[0].path.stem == "Diamonds"


def test_rank_song_choices_benchmark_under_frame_budget() -> None:
    paths = get_song_choices(force_refresh=True)
    choices = [
        SongChoice(path=path, search_key=remove_accents(path.stem).casefold())
        for path in paths
    ]
    assert len(choices) >= 100

    queries = ["dimonds", "lovly", "take me", "yuem", "interstelar", "summr"]
    for query in queries:
        rank_song_choices(choices, query)

    elapsed: list[float] = []
    for query in queries:
        started = time.perf_counter()
        rank_song_choices(choices, query)
        elapsed.append(time.perf_counter() - started)

    assert max(elapsed) < 0.016


def test_textual_theme_tokens_cover_all_picker_presets() -> None:
    assert set(TEXTUAL_THEME_TOKENS) == set(THEME_PRESETS)


def test_table_arrow_moves_one_row_from_initial_focus(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        table = app.query_one("#songs")
        assert app.focused is table
        assert table.cursor_row == 0
        await pilot.press("down")
        assert table.cursor_row == 1
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None


def test_shortcuts_and_arrow_survive_modal_close(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    saves: list[tuple[bool, bool]] = []
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)
    monkeypatch.setattr(
        app_module,
        "save_config",
        lambda cfg: saves.append((cfg.verbose_hud, cfg.telemetry_enabled_by_default)),
    )

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        table = app.query_one("#songs")
        app.action_open_profile()
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
        assert app.focused is table

        await pilot.press("p")
        await pilot.pause()
        assert type(app.screen).__name__ == "OptionModal"
        await pilot.press("escape")
        await pilot.pause()
        await pilot.press("down")
        assert table.cursor_row == 1
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None
    assert saves == []


def test_textual_picker_escape_returns_none(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(_app: SkyPickerApp, pilot: Any) -> None:
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None


def test_textual_metadata_cells_gate_risk_until_analyzed() -> None:
    raw = SongUiMetadata(
        path=SONGS[0],
        name="Alpha",
        duration_seconds=62.0,
        note_count=12,
        max_polyphony=1,
        min_note_gap_ms=100.0,
        min_same_key_gap_ms=200.0,
        risk="low",
        recommended_profile="balanced",
        recommended_tempo_scale=1.0,
        warnings=(),
        analyzed=False,
    )

    assert _metadata_cells(raw) == ("1:02", "12", "...", "...")


def test_profile_modal_persists_and_invalidates_metadata(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    persisted: list[str] = []
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)
    monkeypatch.setattr(app_module, "persist_default_profile", lambda _cfg, profile: persisted.append(profile))

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        app.action_open_profile()
        await pilot.pause()
        await pilot.press("enter")
        assert app.profile_name == "local-precise"
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None
    assert persisted == ["local-precise"]
    assert FakeMetadataCoordinator.instances[0].shutdown_started is True
    assert FakeMetadataCoordinator.instances[0].closed is True
    assert FakeMetadataCoordinator.instances[0].close_waits == [True]
    assert len(FakeMetadataCoordinator.instances) >= 2


def test_tempo_fps_and_theme_modals_persist(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    persisted_tempo: list[float] = []
    persisted_fps: list[int | None] = []
    persisted_theme: list[str] = []
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)
    monkeypatch.setattr(app_module, "persist_default_tempo", lambda _cfg, tempo: persisted_tempo.append(tempo))
    monkeypatch.setattr(app_module, "persist_default_fps", lambda _cfg, fps: persisted_fps.append(fps))
    monkeypatch.setattr(app_module, "save_theme", lambda theme: persisted_theme.append(theme))

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        app.action_open_tempo()
        await pilot.pause()
        await pilot.press("enter")
        assert app.tempo_scale == 0.90
        app.action_open_fps()
        await pilot.pause()
        await pilot.press("down")
        await pilot.press("enter")
        assert app.fps == 30
        app.action_open_theme()
        await pilot.pause()
        await pilot.press("down")
        await pilot.press("enter")
        assert app.active_theme == "minimalist"
        assert app.screen.has_class("theme-minimalist")
        assert not app.screen.has_class("theme-aurora")
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None
    assert persisted_tempo == [0.90]
    assert persisted_fps == [30]
    assert persisted_theme == ["minimalist"]


def test_command_palette_toggles_dry_run(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        assert app.dry_run is True
        app.action_open_commands()
        await pilot.pause()
        for _ in range(5):
            await pilot.press("down")
        await pilot.press("enter")
        assert app.dry_run is False
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None


def test_preview_detail_toggle(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        detail = app.query_one("#detail")
        assert "Alpha" in str(detail.render())
        app.action_toggle_preview()
        assert str(detail.render()) == "Details hidden"
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None


def test_hud_and_telemetry_toggles_save_config(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    saves: list[tuple[bool, bool]] = []
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)
    monkeypatch.setattr(
        app_module,
        "save_config",
        lambda cfg: saves.append((cfg.verbose_hud, cfg.telemetry_enabled_by_default)),
    )

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        app.action_toggle_hud()
        app.action_toggle_telemetry()
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None
    assert saves == [(True, False), (True, True)]


def test_help_and_calibration_modals_open(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)
    monkeypatch.setattr(
        "sky_music.orchestration.calibration.load_latest_telemetry_summary",
        lambda: None,
    )

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        app.action_open_help()
        await pilot.pause()
        assert type(app.screen).__name__ == "InfoModal"
        await pilot.press("escape")
        app.action_open_calibration()
        await pilot.pause()
        assert type(app.screen).__name__ == "InfoModal"
        await pilot.press("escape")
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None


def test_reload_clears_metadata_and_refreshes_song_list(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    clear_calls: list[bool] = []
    lists = [
        SONGS,
        [Path("songs/Delta.json")],
    ]
    calls = 0

    def fake_get_song_choices(force_refresh: bool = False) -> list[Path]:
        nonlocal calls
        calls += 1
        return lists[1] if calls > 1 else lists[0]

    monkeypatch.setattr(app_module, "get_song_choices", fake_get_song_choices)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)
    monkeypatch.setattr(app_module, "clear_metadata_cache", lambda: clear_calls.append(True))

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        assert len(app.choices) == 3
        app.action_reload_songs()
        assert [choice.path for choice in app.choices] == [Path("songs/Delta.json")]
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None
    assert clear_calls == [True]


def test_calibration_apply_persists_and_updates_session(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    persisted: list[tuple[str, float, int]] = []
    summary = {
        "song": "Alpha",
        "profile": "balanced",
        "tempo_scale": 1.0,
        "fps": 30,
        "lateness_us": {"p95_us": 12000, "p99_us": 20000, "over_10ms": 6},
        "send_duration_us": {"p95_us": 1000},
        "backend": {"panic_release_failures": 0},
        "schedule": {"impossible_same_key_repeats": 1, "risky_same_key_repeats": 6, "note_count": 100},
    }
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)
    monkeypatch.setattr(
        "sky_music.orchestration.calibration.load_latest_telemetry_summary",
        lambda: summary,
    )
    monkeypatch.setattr(
        app_module,
        "persist_calibration_defaults",
        lambda _cfg, *, profile_name, tempo_scale, fps: persisted.append((profile_name, tempo_scale, fps)),
    )

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        app.action_open_calibration()
        await pilot.pause()
        await pilot.press("enter")
        assert app.profile_name == "local-precise"
        assert app.fps == 30
        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None
    assert persisted == [("local-precise", 0.88, 30)]
    assert FakeMetadataCoordinator.instances[0].closed is True


def test_search_debouncing_behavior(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        import sys
        from unittest.mock import patch
        assert app._search_timer is None

        with patch.dict(sys.modules):
            sys.modules.pop("pytest", None)
            sys.modules.pop("unittest", None)
            
            await pilot.click("#search")
            await pilot.press("a")
            assert app._search_timer is not None
            
            app.action_confirm()
            assert app._search_timer is None

    app = run_picker(_run_app(actions))
    assert app.return_value is not None
    assert app.return_value.song_path == SONGS[0]


def test_search_interaction_navigation_and_escape(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        table = app.query_one("#songs")
        search = app.query_one("#search")
        
        assert table.has_focus
        assert not search.has_focus

        await pilot.click("#search")
        assert search.has_focus
        assert not table.has_focus

        assert table.cursor_row == 0
        await pilot.press("down")
        assert table.cursor_row == 1

        await pilot.press("up")
        assert table.cursor_row == 0

        await pilot.press("escape")
        assert table.has_focus
        assert not search.has_focus

        await pilot.press("escape")

    app = run_picker(_run_app(actions))
    assert app.return_value is None


def test_double_click_row_selects_song(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        table = app.query_one("#songs")
        from textual.widgets import DataTable
        
        row_keys = list(table._data.keys())
        event = DataTable.RowSelected(table, cursor_row=1, row_key=row_keys[1])
        app.post_message(event)
        await pilot.pause()

    app = run_picker(_run_app(actions))
    assert app.return_value is not None
    assert app.return_value.song_path == SONGS[1]


class FailingCloseCoordinator(FakeMetadataCoordinator):
    """Coordinator whose final close(wait=True) cannot stop its worker."""

    def close(self, *, wait: bool = False) -> None:
        self.close_waits.append(wait)
        self.shutdown_started = True
        raise RuntimeError("boom: executor did not stop")


def test_picker_cleanup_failed_predicate() -> None:
    # Missing record => clean (no picker ran); explicit ok=False or absent ok => failed.
    assert _picker_cleanup_failed(None) is False
    assert _picker_cleanup_failed({"ok": True}) is False
    assert _picker_cleanup_failed({"ok": False}) is True
    assert _picker_cleanup_failed({}) is True


def test_textual_cleanup_failure_is_recorded(monkeypatch) -> None:
    """on_unmount must record ok=False (with the error) when a worker cannot be stopped."""
    from sky_music.orchestration.telemetry import TelemetryLogger

    TelemetryLogger.last_picker_cleanup = None
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FailingCloseCoordinator)

    async def scenario() -> None:
        app = SkyPickerApp(initial_dry_run=True, cfg=AppConfig())
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("escape")

    # on_unmount re-raises after recording; tolerate whichever way Textual routes it.
    try:
        run_picker(scenario())
    except Exception:
        pass

    cleanup = TelemetryLogger.last_picker_cleanup
    assert cleanup is not None
    assert cleanup["ok"] is False
    assert "boom" in (cleanup.get("error") or "")


def test_choose_textual_aborts_on_failed_cleanup(monkeypatch) -> None:
    """A failed cleanup must abort before playback, independent of Textual exception routing."""
    from sky_music.orchestration.telemetry import TelemetryLogger

    def fake_run(self: SkyPickerApp) -> None:
        TelemetryLogger.last_picker_cleanup = {"ok": False, "error": "boom", "resources": []}
        return None

    monkeypatch.setattr(SkyPickerApp, "run", fake_run)
    with pytest.raises(RuntimeError, match="cleanup failed before playback"):
        choose_song_interactively_textual()


def test_choose_textual_returns_result_on_clean_cleanup(monkeypatch) -> None:
    from sky_music.orchestration.telemetry import TelemetryLogger

    sentinel = SongPickerResult(
        song_path=Path("songs/Alpha.json"),
        action="play",
        profile_name="balanced",
        tempo_scale=1.0,
    )

    def fake_run(self: SkyPickerApp) -> SongPickerResult:
        TelemetryLogger.last_picker_cleanup = {"ok": True, "resources": []}
        return sentinel

    monkeypatch.setattr(SkyPickerApp, "run", fake_run)
    assert choose_song_interactively_textual() is sentinel
