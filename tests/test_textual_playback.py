from __future__ import annotations

import asyncio
import time

from textual.widgets import Static

from sky_music.ui.textual_app.playback_app import (
    PlaybackApp,
    PlaybackCard,
    PlaybackCommandBridge,
    SnapshotRenderer,
)


class FakeEngine:
    def __init__(
        self,
        renderer: SnapshotRenderer,
        total_us: int,
        result: str = "finished",
        finish_msg: str = "Finished playing Test Song",
    ) -> None:
        self.renderer = renderer
        self.total_us = total_us
        self.result = result
        self.finish_msg = finish_msg

    def play(self) -> str:
        self.renderer.render(
            current=2.0,
            total=self.total_us / 1_000_000,
            song_name="Test Song",
            status="playing",
        )
        time.sleep(0.3)
        self.renderer.finish(self.finish_msg)
        return self.result

def test_snapshot_renderer_unit() -> None:
    renderer = SnapshotRenderer()
    assert renderer.get_snapshot() is None
    assert not renderer.done
    
    renderer.render(
        current=1.5,
        total=10.0,
        song_name="My Song",
        status="paused",
        input_path_degraded=True,
    )
    snap = renderer.get_snapshot()
    assert snap is not None
    assert snap.current == 1.5
    assert snap.total == 10.0
    assert snap.song_name == "My Song"
    assert snap.status == "paused"
    assert snap.input_path_degraded is True
    
    renderer.update_counters(5000)
    assert renderer.max_lateness_us == 5000
    
    renderer.update_counters(2000)
    assert renderer.max_lateness_us == 5000
    
    renderer.finish("Stopped: My Song")
    assert renderer.done
    assert renderer.finish_message == "Stopped: My Song"

async def _run_app_test_renders() -> PlaybackApp:
    renderer = SnapshotRenderer()
    engine = FakeEngine(renderer, 5_000_000, "finished", "Finished playing Test Song")
    app = PlaybackApp(engine, renderer, "aurora", "Test Song", 5_000_000)  # type: ignore[arg-type]
    
    async with app.run_test() as pilot:
        await pilot.pause(0.2)
        # verify widgets
        time_widget = app.query_one("#time-info", Static)
        assert "0:02 / 0:05" in str(time_widget.render())
        
        status_widget = app.query_one("#status-info", Static)
        assert "Playing" in str(status_widget.render())
    return app

def test_playback_app_renders_snapshot() -> None:
    app = asyncio.run(_run_app_test_renders())
    assert app.return_value == "finished"

async def _run_app_test_skipped() -> PlaybackApp:
    renderer = SnapshotRenderer()
    engine = FakeEngine(renderer, 5_000_000, "skipped", "Skipped: Test Song")
    app = PlaybackApp(engine, renderer, "minimalist", "Test Song", 5_000_000)  # type: ignore[arg-type]
    
    async with app.run_test() as pilot:
        await pilot.pause(0.2)
    return app

def test_playback_app_runs_and_exits_skipped() -> None:
    app = asyncio.run(_run_app_test_skipped())
    assert app.return_value == "skipped"

async def _run_app_test_quit() -> PlaybackApp:
    renderer = SnapshotRenderer()
    engine = FakeEngine(renderer, 5_000_000, "quit", "Stopped: Test Song")
    app = PlaybackApp(engine, renderer, "slate", "Test Song", 5_000_000)  # type: ignore[arg-type]
    
    async with app.run_test() as pilot:
        await pilot.pause(0.2)
    return app

def test_playback_app_runs_and_exits_quit() -> None:
    app = asyncio.run(_run_app_test_quit())
    assert app.return_value == "quit"

def test_playback_app_handles_heavy_lateness_updates() -> None:
    renderer = SnapshotRenderer()
    for lateness in range(1, 1000):
        renderer.update_counters(lateness)
    assert renderer.max_lateness_us == 999


def test_playback_command_bridge_prioritizes_ui_commands_and_suppresses_duplicate() -> None:
    class BaseControls:
        def __init__(self) -> None:
            self._was_down: dict[str, bool] = {}
            self.poll_count = 0

        def poll(self) -> str | None:
            self.poll_count += 1
            return "pause"

    base = BaseControls()
    bridge = PlaybackCommandBridge(base)

    bridge.request("skip")

    assert bridge.poll() == "skip"
    assert "skip" not in base._was_down
    assert bridge.poll() == "pause"
    assert base.poll_count == 1


def test_playback_command_bridge_allows_repeated_ui_pause_commands() -> None:
    bridge = PlaybackCommandBridge(None)

    bridge.request("pause")
    bridge.request("pause")

    assert bridge.poll() == "pause"
    assert bridge.poll() == "pause"
    assert bridge.poll() is None


def test_playback_card_keys_enqueue_commands() -> None:
    bridge = PlaybackCommandBridge(None)
    card = PlaybackCard(theme_name="aurora")
    card._mode = "playing"
    card.command_bridge = bridge

    class Event:
        key = "f8"

        def __init__(self) -> None:
            self.stopped = False

        def stop(self) -> None:
            self.stopped = True

    event = Event()
    card.on_key(event)  # type: ignore[arg-type]

    assert event.stopped is True
    assert bridge.poll() == "pause"


def test_playback_card_unmount_requests_quit_when_playing() -> None:
    bridge = PlaybackCommandBridge(None)
    card = PlaybackCard(theme_name="aurora")
    card._mode = "playing"
    card.command_bridge = bridge

    card.on_unmount()

    assert bridge.poll() == "quit"


def test_unified_cancel_while_playing_requests_engine_quit() -> None:
    from sky_music.config import AppConfig
    from sky_music.ui.textual_app.app import SkyPickerApp

    bridge = PlaybackCommandBridge(None)
    app = SkyPickerApp(initial_dry_run=True, unified_mode=True, countdown_seconds=0, cfg=AppConfig())
    from sky_music.ui.textual_app.app_state import PlaybackMode
    app.playback_mode = PlaybackMode.PLAYING
    app._active_playback_commands = bridge

    app.action_cancel()

    assert bridge.poll() == "quit"


def test_unified_workflow_integration(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.orchestration.telemetry import TelemetryLogger
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    TEST_SONGS = [Path("songs/Alpha.json")]

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False
        @property
        def name(self) -> str:
            return "mock-metadata"
        @property
        def phase(self) -> str:
            return "picker"
        def refresh(self, paths) -> None:
            pass
        def cancel(self) -> None:
            pass
        def close(self, *, wait: bool = False) -> None:
            self.closed = True
        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,  # type: ignore[arg-type]
            playback_duration_us=5_000_000,  # type: ignore[arg-type]
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        active_sleep_policy = SleepPolicy(
            spin_threshold_us=1000,
            poll_s=0.025,
        )
        risk_report = ScheduleRiskReport(
            severity="low",
            impossible_repeats=0,
            impossible_same_key_repeats=0,
            compressed_holds=0,
            max_polyphony=1,
            min_any_note_gap_us=None,
            min_same_key_gap_us=None,
            dense_clusters=(),
            recommendations=(),
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=sched_meta,
            session=session,
            active_policy=active_policy,
            active_sleep_policy=active_sleep_policy,
            song=song,
            risk_report=risk_report,
            cfg=cfg,
        )

    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            return "finished"

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    TelemetryLogger.last_picker_cleanup = None

    async def run_integration_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.5)
            assert TelemetryLogger.last_picker_cleanup is not None
            assert TelemetryLogger.last_picker_cleanup.get("ok") is True
            await pilot.press("escape")

    asyncio.run(run_integration_test())


def test_unified_playback_quit_does_not_rearm_metadata(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    instances: list[Any] = []

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False
            instances.append(self)

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            active_sleep_policy=SleepPolicy(spin_threshold_us=1000, poll_s=0.025),
            song=song,
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            return "quit"

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_quit_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            assert len(instances) == 1
            await pilot.press("enter")
            await pilot.pause(0.3)

    asyncio.run(run_quit_test())
    assert len(instances) == 1


def test_unified_workflow_focuses_sky_before_non_dry_playback(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            active_sleep_policy=SleepPolicy(spin_threshold_us=1000, poll_s=0.025),
            song=song,
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    focus_calls: list[str] = []

    class MockFocusGuard:
        def focus(self) -> bool:
            focus_calls.append("focus")
            return True

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            return "finished"

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: False)
    monkeypatch.setattr(app_module, "Win32SkyFocusGuard", MockFocusGuard)

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_focus_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=False,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.3)

    asyncio.run(run_focus_test())
    assert focus_calls == ["focus"]


def test_in_place_playback_locks_picker_until_finish(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            active_sleep_policy=SleepPolicy(spin_threshold_us=1000, poll_s=0.025),
            song=song,
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            time.sleep(0.5)
            return "finished"

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: False)

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_lock_test(size: tuple[int, int]) -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test(size=size) as pilot:
            await pilot.pause()
            table = app.query_one("#songs")
            search = app.query_one("#search")
            card = app.query_one("#playback-card")
            songs_h_before = table.region.height
            await pilot.press("enter")
            await pilot.pause(0.2)
            assert app.playback_mode == "playing"
            assert search.disabled is True
            assert table.disabled is True
            assert card.region.bottom <= app.screen.region.bottom
            assert card.region.bottom == app.screen.region.bottom - 1
            assert table.region.bottom <= card.region.y
            assert table.region.height > 0
            assert card.styles.background == table.styles.background
            cursor_row = table.cursor_row  # type: ignore[attr-defined]
            await pilot.press("down")
            await pilot.pause(0.1)
            assert table.cursor_row == cursor_row  # type: ignore[attr-defined]
            await pilot.pause(0.6)
            assert app.playback_mode == "picker"
            assert search.disabled is False
            assert table.disabled is False

    asyncio.run(run_lock_test((100, 30)))
    asyncio.run(run_lock_test((60, 24)))


def test_card_anchored_after_countdown_grows(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_app import PlaybackCard
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            active_sleep_policy=SleepPolicy(spin_threshold_us=1000, poll_s=0.025),
            song=Song(name="Mock Song", notes=()),
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            time.sleep(0.5)
            return "finished"

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: False)

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_growth_test(size: tuple[int, int]) -> None:
        app = SkyPickerApp(
            initial_dry_run=False,
            unified_mode=True,
            countdown_seconds=3,
            cfg=AppConfig(),
        )
        async with app.run_test(size=size) as pilot:
            await pilot.pause()
            songs = app.query_one("#songs")
            songs_height_before = songs.region.height
            await pilot.press("enter")
            await pilot.pause(0.2)
            card = app.query_one("#playback-card", PlaybackCard)
            assert app.playback_mode == "countdown"
            card_height_countdown = card.region.height

            card._tick_countdown()
            card._tick_countdown()
            card._tick_countdown()
            await pilot.pause(0.2)

            assert app.playback_mode == "playing"
            assert card.region.bottom == app.screen.region.bottom - 1
            assert card.region.bottom <= app.screen.region.bottom
            assert card.region.height > card_height_countdown
            assert songs.region.height > 0
            assert songs.region.bottom <= card.region.y

    asyncio.run(run_growth_test((100, 30)))
    asyncio.run(run_growth_test((60, 24)))


def test_card_anchored_after_debug_toggle_grows(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_app import PlaybackCard
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            active_sleep_policy=SleepPolicy(spin_threshold_us=1000, poll_s=0.025),
            song=Song(name="Mock Song", notes=()),
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            time.sleep(0.5)
            return "finished"

    hotkey_down = {"value": False}
    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: hotkey_down["value"])

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_debug_growth_test(size: tuple[int, int]) -> None:
        hotkey_down["value"] = False
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test(size=size) as pilot:
            await pilot.pause()
            songs = app.query_one("#songs")
            songs_height_before = songs.region.height
            await pilot.press("enter")
            await pilot.pause(0.2)
            card = app.query_one("#playback-card", PlaybackCard)
            card_height_before = card.region.height

            hotkey_down["value"] = True
            card._poll()
            await pilot.pause(0.1)

            assert card.debug_mode is True
            assert card.region.bottom == app.screen.region.bottom - 1
            assert card.region.bottom <= app.screen.region.bottom
            assert card.region.height > card_height_before
            assert songs.region.height > 0
            assert songs.region.bottom <= card.region.y

    asyncio.run(run_debug_growth_test((100, 30)))
    asyncio.run(run_debug_growth_test((60, 24)))


def test_timing_line_no_bare_na() -> None:
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.ui.textual_app.playback_app import PlaybackCard

    framed = PlaybackCard(
        theme_name="aurora",
        song_name="Mock Song",
        active_policy=FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        ),
        debug_mode=True,
    )
    framed._mode = "playing"
    framed_render = str(framed.render())
    assert "Timing:" in framed_render
    assert "60fps" in framed_render
    assert "N/A" not in framed_render

    fallback = PlaybackCard(
        theme_name="aurora",
        song_name="Mock Song",
        active_policy=FrameTimingPolicy(
            fps=0,
            frame_us=0,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        ),
        debug_mode=True,
    )
    fallback._mode = "playing"
    fallback_render = str(fallback.render())
    assert "Timing:" in fallback_render
    assert "60fps" in fallback_render
    assert "unframed" not in fallback_render
    assert "N/A" not in fallback_render


def test_no_unframed_auto_or_na_in_default_header_and_timing(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_app import PlaybackCard
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        policy = FrameTimingPolicy(
            fps=session.fps,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            active_sleep_policy=SleepPolicy(spin_threshold_us=1000, poll_s=0.025),
            song=Song(name="Mock Song", notes=()),
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            time.sleep(0.2)
            return "finished"

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: False)

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_no_unframed_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(game_fps=0),
        )
        async with app.run_test(size=(100, 30)) as pilot:
            await pilot.pause()
            assert app.fps == 60
            app._render_status()
            status_text = str(app.query_one("#appbar").render())
            assert "60fps" in status_text
            await pilot.press("enter")
            await pilot.pause(0.1)
            card = app.query_one("#playback-card", PlaybackCard)
            rendered = str(card.render())
            for banned in ("unframed", "auto", "N/A"):
                assert banned not in status_text
                assert banned not in rendered
            assert "60fps" in rendered

    asyncio.run(run_no_unframed_test())


def test_config_path_is_cwd_independent(tmp_path, monkeypatch) -> None:
    import sky_music.config as config_module

    canonical_path = config_module.CONFIG_PATH
    assert canonical_path.is_absolute()
    monkeypatch.chdir(tmp_path)
    assert canonical_path == config_module.CONFIG_PATH


def test_header_fps_matches_policy_fps(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_app import PlaybackCard
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "mock-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, paths) -> None:
            pass

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> Any:
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        fps = session.fps or 0
        policy = FrameTimingPolicy(
            fps=fps,
            frame_us=16666 if fps else 0,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=ScheduleMetadata(actions=(), source_duration_us=5_000_000, playback_duration_us=5_000_000),  # type: ignore[arg-type]
            session=session,
            active_policy=policy,
            active_sleep_policy=SleepPolicy(spin_threshold_us=1000, poll_s=0.025),
            song=Song(name="Mock Song", notes=()),
            risk_report=ScheduleRiskReport(
                severity="low",
                impossible_repeats=0,
                impossible_same_key_repeats=0,
                compressed_holds=0,
                max_polyphony=1,
                min_any_note_gap_us=None,
                min_same_key_gap_us=None,
                dense_clusters=(),
                recommendations=(),
            ),
            cfg=cfg,
        )

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()

        def play(self) -> str:
            time.sleep(0.2)
            return "finished"

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: False)

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_match_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            initial_fps=60,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(game_fps=60),
        )
        async with app.run_test(size=(100, 30)) as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.1)
            card = app.query_one("#playback-card", PlaybackCard)
            assert card.active_policy is not None
            assert app.fps == card.active_policy.fps
            assert "60fps" in str(card.render())

    asyncio.run(run_match_test())


def test_unified_workflow_quiesce_failure(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.orchestration.telemetry import TelemetryLogger
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    TEST_SONGS = [Path("songs/Alpha.json")]

    class MockMetadataCoordinator:
        should_fail = True
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False
        @property
        def name(self) -> str:
            return "mock-metadata"
        @property
        def phase(self) -> str:
            return "picker"
        def refresh(self, paths) -> None:
            pass
        def cancel(self) -> None:
            pass
        def close(self, *, wait: bool = False) -> None:
            if MockMetadataCoordinator.should_fail:
                MockMetadataCoordinator.should_fail = False
                raise RuntimeError("Simulation of worker closing failure!")
            self.closed = True
        def snapshot(self) -> Any:
            from sky_music.infrastructure.background import WorkerSnapshot
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,  # type: ignore[arg-type]
            playback_duration_us=5_000_000,  # type: ignore[arg-type]
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        active_sleep_policy = SleepPolicy(
            spin_threshold_us=1000,
            poll_s=0.025,
        )
        risk_report = ScheduleRiskReport(
            severity="low",
            impossible_repeats=0,
            impossible_same_key_repeats=0,
            compressed_holds=0,
            max_polyphony=1,
            min_any_note_gap_us=None,
            min_same_key_gap_us=None,
            dense_clusters=(),
            recommendations=(),
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=sched_meta,
            session=session,
            active_policy=active_policy,
            active_sleep_policy=active_sleep_policy,
            song=song,
            risk_report=risk_report,
            cfg=cfg,
        )

    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()
        def play(self) -> str:
            return "finished"

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    class CleanupTracker:
        def __get__(self, instance, owner):
            return getattr(self, "_val", None)
        def __set__(self, instance, value):
            self._val = value
    
    monkeypatch.setattr(TelemetryLogger, "last_picker_cleanup", CleanupTracker())

    async def run_integration_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.5)
            assert TelemetryLogger.last_picker_cleanup is not None
            assert TelemetryLogger.last_picker_cleanup.get("ok") is False
            assert app.playback_mode == "error"
            card = app.query_one("#playback-card")
            assert "Failed to stop background workers" in str(card.render())
            assert card.has_focus
            await pilot.pause(0.2)
            await pilot.press("escape")
            await pilot.pause(0.2)
            assert app.playback_mode == "picker"

    asyncio.run(run_integration_test())



def test_unified_workflow_prepare_playback_error(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_controller import PlaybackError
    from sky_music.ui.textual_app.screens import picker as picker_module

    TEST_SONGS = [Path("songs/Alpha.json")]

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False
        @property
        def name(self) -> str:
            return "mock-metadata"
        @property
        def phase(self) -> str:
            return "picker"
        def refresh(self, paths) -> None:
            pass
        def cancel(self) -> None:
            pass
        def close(self, *, wait: bool = False) -> None:
            self.closed = True
        def snapshot(self) -> Any:
            from sky_music.infrastructure.background import WorkerSnapshot
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)

    def mock_prepare_playback_error(song_path, session, cfg, is_dry_run=False):
        return PlaybackError(code="test_error", message="Mocked playback error description")

    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback_error)

    async def run_integration_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.5)
            assert app.playback_mode == "error"
            card = app.query_one("#playback-card")
            assert "Mocked playback error description" in str(card.render())
            await pilot.pause()
            await pilot.press("escape")
            await pilot.pause(0.2)
            assert app.playback_mode == "picker"

    asyncio.run(run_integration_test())


def test_unified_workflow_risk_decisions(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.ui.textual_app.screens import picker as picker_module

    TEST_SONGS = [Path("songs/Alpha.json")]

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False
        @property
        def name(self) -> str:
            return "mock-metadata"
        @property
        def phase(self) -> str:
            return "picker"
        def refresh(self, paths) -> None:
            pass
        def cancel(self) -> None:
            pass
        def close(self, *, wait: bool = False) -> None:
            self.closed = True
        def snapshot(self) -> Any:
            from sky_music.infrastructure.background import WorkerSnapshot
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)

    def mock_prepare_playback_high_risk(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,  # type: ignore[arg-type]
            playback_duration_us=5_000_000,  # type: ignore[arg-type]
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        active_sleep_policy = SleepPolicy(
            spin_threshold_us=1000,
            poll_s=0.025,
        )
        risk_report = ScheduleRiskReport(
            severity="high",
            impossible_repeats=0,
            impossible_same_key_repeats=0,
            compressed_holds=0,
            max_polyphony=1,
            min_any_note_gap_us=None,
            min_same_key_gap_us=None,
            dense_clusters=(),
            recommendations=("Risk recommendation test",),
            suggested_profile="audience-safe",
            suggested_tempo_scale=0.92,
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=sched_meta,
            session=session,
            active_policy=active_policy,
            active_sleep_policy=active_sleep_policy,
            song=song,
            risk_report=risk_report,
            cfg=cfg,
        )

    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback_high_risk)

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()
        def play(self) -> str:
            return "finished"

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_risk_cancel_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.5)
            assert app.playback_mode == "risk"
            card = app.query_one("#playback-card")
            assert "Risk Level: HIGH" in str(card.render())
            await pilot.press("down")
            await pilot.press("down")
            await pilot.press("down")
            await pilot.press("down")
            await pilot.press("enter")
            await pilot.pause(0.5)
            assert app.playback_mode == "picker"
            await pilot.press("escape")

    async def run_risk_proceed_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.5)
            await pilot.press("enter")
            await pilot.pause(0.5)
            await pilot.press("escape")

    asyncio.run(run_risk_cancel_test())
    asyncio.run(run_risk_proceed_test())


def test_debug_stats_calculation() -> None:
    from sky_music.ui.textual_app.playback_app import PlaybackSnapshot, SnapshotRenderer

    renderer = SnapshotRenderer()
    
    # Test empty buffer handles gracefully
    stats = renderer.debug_stats()
    assert stats.max_lateness_us == 0
    assert stats.late_2ms == 0
    assert stats.late_5ms == 0
    assert stats.late_10ms == 0
    assert stats.p50_ms == 0.0
    assert stats.p95_ms == 0.0
    assert stats.sigma_onset_ms == 0.0
    assert stats.active_keys == 0
    assert stats.stuck_keys == 0
    assert stats.backend_status == "healthy"

    # Add latencies
    latencies = [1000, 2000, 3000, 4000, 5000, 15000]
    for lat in latencies:
        renderer.update_counters(lat)

    # Calculate expected stdev
    mean = sum(latencies) / len(latencies)
    variance = sum((x - mean) ** 2 for x in latencies) / len(latencies)
    expected_jitter = (variance ** 0.5) / 1000.0

    stats = renderer.debug_stats()
    assert stats.max_lateness_us == 15000
    assert stats.late_2ms == 4
    assert stats.late_5ms == 1
    assert stats.late_10ms == 1
    assert stats.p50_ms == 4.0
    assert stats.p95_ms == 15.0
    assert abs(stats.sigma_onset_ms - expected_jitter) < 1e-5

    # Mock backend health
    class MockBackendHealth:
        def __init__(self):
            self.active_count = 5
            self.failed_release_count = 2

    renderer.snapshot = PlaybackSnapshot(
        current=1.0,
        total=5.0,
        song_name="Test",
        backend_health=MockBackendHealth(),  # type: ignore[arg-type]
    )

    stats = renderer.debug_stats()
    assert stats.active_keys == 5
    assert stats.stuck_keys == 2
    assert stats.backend_status == "stuck:2"


def test_playback_screen_toggle_debug(monkeypatch) -> None:
    from pathlib import Path
    from typing import Any

    from sky_music.config import AppConfig
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app import playback_app as playback_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_app import PlaybackCard
    from sky_music.ui.textual_app.screens import picker as picker_module

    TEST_SONGS = [Path("songs/Alpha.json")]

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False
        @property
        def name(self) -> str:
            return "mock-metadata"
        @property
        def phase(self) -> str:
            return "picker"
        def refresh(self, paths) -> None:
            pass
        def cancel(self) -> None:
            pass
        def close(self, *, wait: bool = False) -> None:
            self.closed = True
        def snapshot(self) -> Any:
            from sky_music.infrastructure.background import WorkerSnapshot
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)

    # Mock prepare_playback to bypass risks and return low-risk PlaybackPlan
    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,  # type: ignore[arg-type]
            playback_duration_us=5_000_000,  # type: ignore[arg-type]
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        active_sleep_policy = SleepPolicy(
            spin_threshold_us=1000,
            poll_s=0.025,
        )
        risk_report = ScheduleRiskReport(
            severity="low",
            impossible_repeats=0,
            impossible_same_key_repeats=0,
            compressed_holds=0,
            max_polyphony=1,
            min_any_note_gap_us=None,
            min_same_key_gap_us=None,
            dense_clusters=(),
            recommendations=(),
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=sched_meta,
            session=session,
            active_policy=active_policy,
            active_sleep_policy=active_sleep_policy,
            song=song,
            risk_report=risk_report,
            cfg=cfg,
        )

    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()
        def play(self) -> str:
            import time
            time.sleep(2.0)
            return "finished"

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)
    hotkey_down = {"value": False}
    monkeypatch.setattr(playback_module, "is_hotkey_down", lambda hotkey: hotkey_down["value"])

    async def run_toggle_test() -> None:
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.2)
            
            assert app.playback_mode == "playing"
            screen = app.query_one("#playback-card", PlaybackCard)

            # Default debug mode should be False, and the debug stats line is absent
            assert screen.debug_mode is False
            assert "p95" not in str(screen.render())

            # Textual F2 no longer toggles; debug uses global hotkey polling.
            await pilot.press("f2")
            await pilot.pause(0.1)
            assert screen.debug_mode is False
            assert "p95" not in str(screen.render())

            hotkey_down["value"] = True
            screen._poll()
            assert screen.debug_mode is True
            assert "p95" in str(screen.render())

            screen._poll()
            assert screen.debug_mode is True

            hotkey_down["value"] = False
            screen._poll()
            assert screen.debug_mode is True

            hotkey_down["value"] = True
            screen._poll()
            assert screen.debug_mode is False
            assert "p95" not in str(screen.render())
            
            # Press F9 to exit screen cleanly
            await pilot.press("f9")
            await pilot.pause(0.2)
            await pilot.press("escape")

    asyncio.run(run_toggle_test())


def test_playback_screen_debug_mode_initial_state(monkeypatch) -> None:
    """PlaybackScreen(debug_mode=True/False) sets the attribute correctly;
    and when verbose_hud is True in SkyPickerApp, the panel is visible on mount."""
    from pathlib import Path
    from typing import Any

    # --- Part 1: pure attribute check (no Textual runtime needed) ---
    from unittest.mock import MagicMock

    from sky_music.config import AppConfig
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.ui.textual_app.playback_app import PlaybackCard, PlaybackScreen
    from sky_music.ui.textual_app.screens import picker as picker_module
    mock_engine = MagicMock()
    mock_renderer = MagicMock()

    screen_true = PlaybackScreen(
        engine=mock_engine,
        renderer=mock_renderer,
        theme_name="aurora",
        song_name="Test Song",
        total_us=5_000_000,
        debug_mode=True,
    )
    assert screen_true.debug_mode is True

    screen_false = PlaybackScreen(
        engine=mock_engine,
        renderer=mock_renderer,
        theme_name="aurora",
        song_name="Test Song",
        total_us=5_000_000,
        debug_mode=False,
    )
    assert screen_false.debug_mode is False

    # --- Part 2: integration — verbose_hud=True → panel block on mount ---
    TEST_SONGS = [Path("songs/Alpha.json")]

    class MockMetadataCoordinator:
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False
        @property
        def name(self) -> str:
            return "mock-metadata"
        @property
        def phase(self) -> str:
            return "picker"
        def refresh(self, paths) -> None:
            pass
        def cancel(self) -> None:
            pass
        def close(self, *, wait: bool = False) -> None:
            self.closed = True
        def snapshot(self) -> Any:
            from sky_music.infrastructure.background import WorkerSnapshot
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                pending_count=0,
                running_count=0,
            )

    monkeypatch.setattr(app_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(picker_module, "get_song_choices", lambda force_refresh=False: TEST_SONGS)
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)
    monkeypatch.setattr(picker_module, "MetadataCoordinator", MockMetadataCoordinator)

    from sky_music.domain import Song
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,  # type: ignore[arg-type]
            playback_duration_us=5_000_000,  # type: ignore[arg-type]
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,  # type: ignore[arg-type]
            hold_us=100000,  # type: ignore[arg-type]
            min_hold_us=50000,  # type: ignore[arg-type]
            focus_restore_grace_us=2000000,  # type: ignore[arg-type]
        )
        active_sleep_policy = SleepPolicy(
            spin_threshold_us=1000,
            poll_s=0.025,
        )
        risk_report = ScheduleRiskReport(
            severity="low",
            impossible_repeats=0,
            impossible_same_key_repeats=0,
            compressed_holds=0,
            max_polyphony=1,
            min_any_note_gap_us=None,
            min_same_key_gap_us=None,
            dense_clusters=(),
            recommendations=(),
        )
        return PlaybackPlan(
            actions=(),
            sched_meta=sched_meta,
            session=session,
            active_policy=active_policy,
            active_sleep_policy=active_sleep_policy,
            song=song,
            risk_report=risk_report,
            cfg=cfg,
        )

    monkeypatch.setattr(app_module, "prepare_playback", mock_prepare_playback)

    class MockTelemetry:
        def record_schedule_metadata(self, sched_meta) -> None:
            pass

    class MockPlaybackEngine:
        def __init__(self, *args, **kwargs) -> None:
            self.telemetry = MockTelemetry()
        def play(self) -> str:
            import time
            time.sleep(2.0)
            return "finished"

    import sky_music.orchestration.engine as engine_module
    monkeypatch.setattr(engine_module, "PlaybackEngine", MockPlaybackEngine)

    async def run_debug_on_test() -> None:
        # verbose_hud=True → debug_mode=True on PlaybackScreen
        cfg = AppConfig()
        cfg.verbose_hud = True
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=cfg,
        )
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.press("enter")
            await pilot.pause(0.2)

            assert app.playback_mode == "playing"
            screen = app.query_one("#playback-card", PlaybackCard)
            assert screen.debug_mode is True
            assert "p95" in str(screen.render())

            # Press F9 to exit cleanly
            await pilot.press("f9")
            await pilot.pause(0.2)
            await pilot.press("escape")

    asyncio.run(run_debug_on_test())
