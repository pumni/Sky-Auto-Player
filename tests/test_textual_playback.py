from __future__ import annotations

import asyncio
from textual.widgets import Static

import time

from sky_music.ui.textual_app.playback_app import PlaybackApp, SnapshotRenderer

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
    app = PlaybackApp(engine, renderer, "aurora", "Test Song", 5_000_000)
    
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
    app = PlaybackApp(engine, renderer, "minimalist", "Test Song", 5_000_000)
    
    async with app.run_test() as pilot:
        await pilot.pause(0.2)
    return app

def test_playback_app_runs_and_exits_skipped() -> None:
    app = asyncio.run(_run_app_test_skipped())
    assert app.return_value == "skipped"

async def _run_app_test_quit() -> PlaybackApp:
    renderer = SnapshotRenderer()
    engine = FakeEngine(renderer, 5_000_000, "quit", "Stopped: Test Song")
    app = PlaybackApp(engine, renderer, "slate", "Test Song", 5_000_000)
    
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


def test_unified_workflow_integration(monkeypatch) -> None:
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.domain import Song
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.orchestration.telemetry import TelemetryLogger
    from sky_music.infrastructure.background import WorkerSnapshot
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.config import AppConfig
    from pathlib import Path
    from typing import Any

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
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,
            playback_duration_us=5_000_000,
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,
            hold_us=100000,
            min_hold_us=50000,
            focus_restore_grace_us=2000000,
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


def test_unified_workflow_quiesce_failure(monkeypatch) -> None:
    from sky_music.ui.textual_app import app as app_module
    from sky_music.orchestration.telemetry import TelemetryLogger
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.config import AppConfig
    from pathlib import Path
    from typing import Any
    from sky_music.domain import Song
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan

    TEST_SONGS = [Path("songs/Alpha.json")]

    class MockMetadataCoordinator:
        should_fail = True
        def __init__(self, *args, **kwargs) -> None:
            self.closed = False
            print("MOCK INIT! should_fail =", MockMetadataCoordinator.should_fail, "ID:", id(self))
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
            print("MOCK CLOSE CALLED! should_fail =", MockMetadataCoordinator.should_fail, "ID:", id(self))
            if MockMetadataCoordinator.should_fail:
                MockMetadataCoordinator.should_fail = False
                print("MOCK CLOSE RAISING RUNTIMEERROR!")
                raise RuntimeError("Simulation of worker closing failure!")
            self.closed = True
            print("MOCK CLOSE SUCCESSFUL!")
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
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        print("MOCK PREPARE_PLAYBACK CALLED!")
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,
            playback_duration_us=5_000_000,
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,
            hold_us=100000,
            min_hold_us=50000,
            focus_restore_grace_us=2000000,
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

    import traceback
    class CleanupTracker:
        def __get__(self, instance, owner):
            return getattr(self, "_val", None)
        def __set__(self, instance, value):
            self._val = value
            print("SETTING last_picker_cleanup to:", value)
            traceback.print_stack()
    
    monkeypatch.setattr(TelemetryLogger, "last_picker_cleanup", CleanupTracker())

    async def run_integration_test() -> None:
        print("APP_MODULE:", app_module)
        print("METADATA_COORDINATOR IN APP_MODULE:", getattr(app_module, "MetadataCoordinator", None))
        print("MOCK_METADATA_COORDINATOR:", MockMetadataCoordinator)
        app = SkyPickerApp(
            initial_dry_run=True,
            unified_mode=True,
            countdown_seconds=0,
            cfg=AppConfig(),
        )
        print("APP UNIFIED MODE:", app.unified_mode)
        async with app.run_test() as pilot:
            await pilot.pause()
            print("BEFORE ENTER - last_picker_cleanup:", TelemetryLogger.last_picker_cleanup)
            await pilot.press("enter")
            await pilot.pause(0.5)
            print("AFTER ENTER - last_picker_cleanup:", TelemetryLogger.last_picker_cleanup)
            assert TelemetryLogger.last_picker_cleanup is not None
            assert TelemetryLogger.last_picker_cleanup.get("ok") is False
            from sky_music.ui.textual_app.modals import InfoModal
            assert isinstance(app.screen, InfoModal)
            await pilot.press("enter")
            await pilot.pause(0.2)
            await pilot.press("escape")

    asyncio.run(run_integration_test())



def test_unified_workflow_prepare_playback_error(monkeypatch) -> None:
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.config import AppConfig
    from sky_music.ui.textual_app.playback_controller import PlaybackError
    from pathlib import Path
    from typing import Any

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
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)

    def mock_prepare_playback_error(song_path, session, cfg, is_dry_run=False):
        print("MOCK_PREPARE_PLAYBACK_ERROR CALLED!")
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
            print("CHOICES:", app.choices)
            print("FILTERED:", app.filtered)
            print("BEFORE ENTER - SCREEN:", app.screen, "STACK:", app.screen_stack)
            await pilot.press("enter")
            await pilot.pause(0.5)
            print("AFTER ENTER - SCREEN:", app.screen, "STACK:", app.screen_stack)
            from sky_music.ui.textual_app.modals import InfoModal
            assert isinstance(app.screen, InfoModal)
            await pilot.press("enter")
            await pilot.pause()
            await pilot.press("escape")

    asyncio.run(run_integration_test())


def test_unified_workflow_risk_decisions(monkeypatch) -> None:
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan
    from sky_music.domain import Song
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.config import AppConfig
    from pathlib import Path
    from typing import Any

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
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)

    def mock_prepare_playback_high_risk(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,
            playback_duration_us=5_000_000,
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,
            hold_us=100000,
            min_hold_us=50000,
            focus_restore_grace_us=2000000,
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
            from sky_music.ui.textual_app.modals import OptionModal
            assert isinstance(app.screen, OptionModal)
            await pilot.press("down")
            await pilot.press("down")
            await pilot.press("down")
            await pilot.press("down")
            await pilot.press("enter")
            await pilot.pause(0.5)
            assert not isinstance(app.screen, OptionModal)
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
    from sky_music.ui.textual_app.playback_app import SnapshotRenderer, PlaybackSnapshot

    renderer = SnapshotRenderer()
    
    # Test empty buffer handles gracefully
    stats = renderer.debug_stats()
    assert stats.max_lateness_us == 0
    assert stats.late_2ms == 0
    assert stats.late_5ms == 0
    assert stats.late_10ms == 0
    assert stats.p50_ms == 0.0
    assert stats.p95_ms == 0.0
    assert stats.jitter_ms == 0.0
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
    assert abs(stats.jitter_ms - expected_jitter) < 1e-5

    # Mock backend health
    class MockBackendHealth:
        def __init__(self):
            self.active_count = 5
            self.failed_release_count = 2

    renderer.snapshot = PlaybackSnapshot(
        current=1.0,
        total=5.0,
        song_name="Test",
        backend_health=MockBackendHealth(),
    )

    stats = renderer.debug_stats()
    assert stats.active_keys == 5
    assert stats.stuck_keys == 2
    assert stats.backend_status == "stuck:2"


def test_playback_screen_toggle_debug(monkeypatch) -> None:
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.config import AppConfig
    from sky_music.ui.textual_app.playback_app import PlaybackScreen
    from pathlib import Path
    from typing import Any

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
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)

    # Mock prepare_playback to bypass risks and return low-risk PlaybackPlan
    from sky_music.domain import Song
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,
            playback_duration_us=5_000_000,
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,
            hold_us=100000,
            min_hold_us=50000,
            focus_restore_grace_us=2000000,
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
            
            # Now we should be on PlaybackScreen
            assert isinstance(app.screen, PlaybackScreen)
            screen: PlaybackScreen = app.screen
            
            # Default debug mode should be False, and panel is hidden
            assert screen.debug_mode is False
            panel = screen.query_one("#debug-panel")
            assert panel.styles.display == "none"
            
            # Press F2 to toggle debug mode
            await pilot.press("f2")
            await pilot.pause(0.1)
            assert screen.debug_mode is True
            assert panel.styles.display == "block"
            
            # Press F2 again to toggle back
            await pilot.press("f2")
            await pilot.pause(0.1)
            assert screen.debug_mode is False
            assert panel.styles.display == "none"
            
            # Press F9 to exit screen cleanly
            await pilot.press("f9")
            await pilot.pause(0.2)
            await pilot.press("escape")

    asyncio.run(run_toggle_test())


def test_playback_screen_debug_mode_initial_state(monkeypatch) -> None:
    """PlaybackScreen(debug_mode=True/False) sets the attribute correctly;
    and when verbose_hud is True in SkyPickerApp, the panel is visible on mount."""
    from sky_music.ui.textual_app import app as app_module
    from sky_music.ui.textual_app.app import SkyPickerApp
    from sky_music.config import AppConfig
    from sky_music.ui.textual_app.playback_app import PlaybackScreen
    from pathlib import Path
    from typing import Any

    # --- Part 1: pure attribute check (no Textual runtime needed) ---
    from unittest.mock import MagicMock
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
    monkeypatch.setattr(app_module, "MetadataCoordinator", MockMetadataCoordinator)

    from sky_music.domain import Song
    from sky_music.domain.scheduler import ScheduleMetadata
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.domain.analyzer import ScheduleRiskReport
    from sky_music.ui.textual_app.playback_controller import PlaybackPlan

    def mock_prepare_playback(song_path, session, cfg, is_dry_run=False):
        song = Song(name="Mock Song", notes=())
        sched_meta = ScheduleMetadata(
            actions=(),
            source_duration_us=5_000_000,
            playback_duration_us=5_000_000,
        )
        active_policy = FrameTimingPolicy(
            fps=60,
            frame_us=16666,
            hold_us=100000,
            min_hold_us=50000,
            focus_restore_grace_us=2000000,
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

            assert isinstance(app.screen, PlaybackScreen)
            screen: PlaybackScreen = app.screen
            assert screen.debug_mode is True
            panel = screen.query_one("#debug-panel")
            assert panel.styles.display == "block"

            # Press F9 to exit cleanly
            await pilot.press("f9")
            await pilot.pause(0.2)
            await pilot.press("escape")

    asyncio.run(run_debug_on_test())




