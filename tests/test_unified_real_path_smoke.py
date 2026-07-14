from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

from sky_music.config import AppConfig
from sky_music.infrastructure.background import WorkerSnapshot
from sky_music.orchestration.telemetry import TelemetryLogger
from sky_music.ui.picker import SongPickerResult
from sky_music.ui.textual_app import app as app_module
from sky_music.ui.textual_app.app import SkyPickerApp

SONGS = [
    Path("songs/Alpha.json"),
    Path("songs/Beta.json"),
]


class FakeMetadataCoordinator:
    instances: list[FakeMetadataCoordinator] = []

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
        return WorkerSnapshot(
            name=self.name,
            phase=self.phase,
            closed=self.closed,
            pending_count=0,
            running_count=0,
        )


def run_picker(coro: Any) -> Any:
    return asyncio.run(coro)


def test_unified_real_path_quiesces_picker_before_playback(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    engine_instantiated = False
    init_checked_cleanup = False

    class MockPlaybackEngine:
        def __init__(self, *args: Any, **kwargs: Any) -> None:
            nonlocal engine_instantiated, init_checked_cleanup
            engine_instantiated = True

            # Assert cleanup occurred and succeeded BEFORE engine creation
            assert TelemetryLogger.last_picker_cleanup is not None
            assert TelemetryLogger.last_picker_cleanup.get("ok") is True
            resources = TelemetryLogger.last_picker_cleanup.get("resources", [])
            assert len(resources) > 0
            for snap in resources:
                assert snap["closed"] is True
                assert snap["pending_count"] == 0
                assert snap["running_count"] == 0

            # Assert coordinator itself was closed
            assert len(FakeMetadataCoordinator.instances) > 0
            assert FakeMetadataCoordinator.instances[0].closed is True

            init_checked_cleanup = True

            class MockTelemetry:
                def record_schedule_metadata(self, meta: Any) -> None:
                    pass
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            return "done"

    monkeypatch.setattr("sky_music.orchestration.engine.PlaybackEngine", MockPlaybackEngine)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        from sky_music.domain import Millis, Note, NoteKey, Song
        from sky_music.domain.session_context import PlaybackSessionContext
        from sky_music.ui.textual_app.playback_controller import (
            PlaybackError,
            prepare_playback,
        )

        song = Song(
            name="Test Song",
            notes=(
                Note(time_ms=Millis(0), key=NoteKey("Key0")),
            ),
        )
        session = PlaybackSessionContext.balanced()
        plan = prepare_playback(song, session, app.cfg, is_dry_run=True)
        assert not isinstance(plan, PlaybackError)

        picker_result = SongPickerResult(
            song_path=SONGS[0],
            action="dry_run",
            profile_name="balanced",
            tempo_scale=1.0,
            fps=60,
        )

        # Triggers quiesce and execute_playback_plan
        app.execute_playback_plan(plan, picker_result)
        await pilot.pause()

        # Check engine was created and checked
        assert engine_instantiated is True
        assert init_checked_cleanup is True

        # Check metadata coordinator was rearmed after playback
        assert len(FakeMetadataCoordinator.instances) > 1
        # The new instance should be active and not closed
        assert FakeMetadataCoordinator.instances[-1].closed is False

        await pilot.press("escape")

    async def _run_app(actions_fn: Any) -> SkyPickerApp:
        app = SkyPickerApp(initial_dry_run=True, cfg=AppConfig())
        async with app.run_test() as pilot:
            await pilot.pause()
            await actions_fn(app, pilot)
        return app

    app = run_picker(_run_app(actions))
    assert app.return_value is None


def test_unified_cleanup_failure_blocks_playback_engine_creation(monkeypatch) -> None:
    FakeMetadataCoordinator.instances.clear()
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", FakeMetadataCoordinator)

    engine_instantiated = False

    class MockPlaybackEngine:
        def __init__(self, *args: Any, **kwargs: Any) -> None:
            nonlocal engine_instantiated
            engine_instantiated = True
            class MockTelemetry:
                def record_schedule_metadata(self, meta: Any) -> None:
                    pass
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            return "done"

    monkeypatch.setattr("sky_music.orchestration.engine.PlaybackEngine", MockPlaybackEngine)

    # Force quiesce to fail by patching picker_scope.close_all to raise an error
    original_init = SkyPickerApp.__init__
    def patched_init(self, *args: Any, **kwargs: Any) -> None:
        original_init(self, *args, **kwargs)
        # Monkeypatch close_all on the instantiated picker_scope
        def failing_close_all(*a: Any, **kw: Any) -> Any:
            from sky_music.infrastructure.background import (
                BackgroundCleanupError,
                ScopeCloseResult,
            )
            res = ScopeCloseResult(phase="picker", snapshots=(), errors=("mock error",))
            raise BackgroundCleanupError("Simulation of worker closing failure!", result=res)
        self.picker_scope.close_all = failing_close_all

    monkeypatch.setattr(SkyPickerApp, "__init__", patched_init)

    async def actions(app: SkyPickerApp, pilot: Any) -> None:
        from sky_music.domain import Millis, Note, NoteKey, Song
        from sky_music.domain.session_context import PlaybackSessionContext
        from sky_music.ui.textual_app.playback_controller import (
            PlaybackError,
            prepare_playback,
        )

        song = Song(
            name="Test Song",
            notes=(
                Note(time_ms=Millis(0), key=NoteKey("Key0")),
            ),
        )
        session = PlaybackSessionContext.balanced()
        plan = prepare_playback(song, session, app.cfg, is_dry_run=True)
        assert not isinstance(plan, PlaybackError)

        picker_result = SongPickerResult(
            song_path=SONGS[0],
            action="dry_run",
            profile_name="balanced",
            tempo_scale=1.0,
            fps=60,
        )

        import traceback
        print("EXECUTE CALLED!")
        traceback.print_stack()
        app.execute_playback_plan(plan, picker_result)
        await pilot.pause()

        print("IS INSTANTIATED:", engine_instantiated)
        # Engine must NOT be instantiated
        assert engine_instantiated is False

        # Cleanup failed state recorded
        assert TelemetryLogger.last_picker_cleanup is not None
        assert TelemetryLogger.last_picker_cleanup.get("ok") is False
        assert "Simulation of worker closing failure!" in TelemetryLogger.last_picker_cleanup.get("error", "")

        # Unpatch close_all so on_unmount doesn't fail
        if hasattr(app.picker_scope, "close_all"):
            del app.picker_scope.close_all

        await pilot.press("escape")

    async def _run_app(actions_fn: Any) -> SkyPickerApp:
        app = SkyPickerApp(initial_dry_run=True, cfg=AppConfig())
        async with app.run_test() as pilot:
            await pilot.pause()
            await actions_fn(app, pilot)
        return app

    app = run_picker(_run_app(actions))
    assert app.return_value is None
