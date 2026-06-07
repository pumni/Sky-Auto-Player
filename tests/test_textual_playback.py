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
