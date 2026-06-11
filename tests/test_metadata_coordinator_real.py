from __future__ import annotations

from pathlib import Path
from typing import Any
from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.ui.textual_app.workers import MetadataCoordinator


class FakeApp:
    def __init__(self) -> None:
        self.refreshed = False
        self.calls = []

    def call_from_thread(self, callback: Any, *args: Any, **kwargs: Any) -> Any:
        self.calls.append((callback, args, kwargs))
        callback(*args, **kwargs)

    def refresh_metadata_rows(self) -> None:
        self.refreshed = True


def test_metadata_coordinator_state_transitions() -> None:
    app = FakeApp()
    session = PlaybackSessionContext.balanced()
    cfg = AppConfig()
    
    coord = MetadataCoordinator(app, session, cfg)
    assert coord.snapshot().state == "open"
    assert coord.snapshot().closed is False
    
    coord.close(wait=False)
    assert coord.snapshot().state == "closing"
    assert coord.snapshot().closed is False
    
    coord.close(wait=True)
    assert coord.snapshot().state == "closed"
    assert coord.snapshot().closed is True


def test_metadata_coordinator_cancel_stages(monkeypatch) -> None:
    app = FakeApp()
    session = PlaybackSessionContext.balanced()
    cfg = AppConfig()
    
    coord = MetadataCoordinator(app, session, cfg)
    
    stages_called = []
    
    def mock_warm(song_paths, sess, c):
        stages_called.append("warm")
        # cancel right inside the first stage
        coord.cancel()
        return 1
        
    def mock_hydrate(paths, sess, c):
        stages_called.append("hydrate")
        
    import sky_music.ui.textual_app.workers as workers_module
    monkeypatch.setattr(workers_module, "hydrate_persistent_metadata_for_paths", mock_warm)
    monkeypatch.setattr(workers_module, "hydrate_and_fill_raw_metadata", mock_hydrate)
    
    coord.refresh([Path("some/song.json")])
    
    # Wait for the coordinator to finish
    coord.close(wait=True)
    
    assert "warm" in stages_called
    assert "hydrate" not in stages_called
    assert coord.snapshot().state == "closed"


def test_metadata_coordinator_debounce_request_id(monkeypatch) -> None:
    import time
    app = FakeApp()
    session = PlaybackSessionContext.balanced()
    cfg = AppConfig()
    
    coord = MetadataCoordinator(app, session, cfg)
    
    stages_called = []
    
    def mock_warm(song_paths, sess, c):
        stages_called.append("warm")
        # Trigger another refresh to increment request_id ONLY for the first request
        if len(stages_called) == 1:
            coord.refresh([Path("new/path.json")])
        return 1
        
    def mock_hydrate(paths, sess, c):
        stages_called.append("hydrate")
        
    import sky_music.ui.textual_app.workers as workers_module
    monkeypatch.setattr(workers_module, "hydrate_persistent_metadata_for_paths", mock_warm)
    monkeypatch.setattr(workers_module, "hydrate_and_fill_raw_metadata", mock_hydrate)
    
    coord.refresh([Path("first/path.json")])
    
    # We need to wait a bit to let the second request actually start before we close,
    # or just accept that the second one might be cancelled by close().
    # The key thing to verify is that the first one aborted.
    
    # Let's use a flag to wait for the second warm
    for _ in range(50):
        if stages_called.count("warm") >= 2:
            break
        time.sleep(0.01)

    coord.close(wait=True)
    
    assert stages_called[0] == "warm"
    # The first hydrate should have been skipped.
    # The second hydrate should have happened (or at least the second warm should have happened)
    
    warm_count = stages_called.count("warm")
    hydrate_count = stages_called.count("hydrate")
    
    assert warm_count == 2
    # Since we waited for the second warm, and then closed with wait=True, 
    # the second hydrate should also finish.
    assert hydrate_count == 1


def test_telemetry_summary_contains_picker_cleanup() -> None:
    from sky_music.orchestration.telemetry import TelemetryLogger
    
    TelemetryLogger.last_picker_cleanup = {
        "ok": True,
        "resources": [
            {
                "name": "test-picker",
                "phase": "picker",
                "state": "closed",
                "closed": True,
                "pending_count": 0,
                "running_count": 0
            }
        ]
    }
    
    logger = TelemetryLogger(song_name="Test Song", enabled=True)
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=1000,
        actual_us=1010,
        lateness_us=10,
        send_duration_us=5,
        scan_codes=(1, 2),
        reason="press"
    )
    summary = logger.get_summary()
    
    assert "background" in summary
    assert summary["background"]["picker_cleanup"]["ok"] is True
    assert summary["background"]["picker_cleanup"]["resources"][0]["name"] == "test-picker"

