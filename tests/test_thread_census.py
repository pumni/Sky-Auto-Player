"""Runtime backstop: no picker-phase worker thread may be alive at playback start."""

from __future__ import annotations

import sys
import threading
from pathlib import Path

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.domain.domain import Song, Note, NoteKey, Millis
from sky_music.domain.scheduler import build_key_actions
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.telemetry import TelemetryLogger


def _engine() -> PlaybackEngine:
    song = Song(name="T", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    sched = build_key_actions(song)
    return PlaybackEngine(
        song=song,
        actions=sched.actions,
        backend=DryRunBackend(),
        telemetry_enabled=False,
        require_focus=False,
    )


def test_thread_census_clean_when_no_picker_workers() -> None:
    TelemetryLogger.last_thread_census = None
    leaked = _engine()._record_thread_census()
    assert leaked is None
    assert TelemetryLogger.last_thread_census == {"clean": True, "leaked_threads": []}


def test_thread_census_detects_leaked_picker_worker() -> None:
    TelemetryLogger.last_thread_census = None
    release = threading.Event()
    worker = threading.Thread(
        target=release.wait, name="sky-picker-meta-leak", daemon=True
    )
    worker.start()
    try:
        leaked = _engine()._record_thread_census()
        assert leaked is not None
        assert "sky-picker-meta-leak" in leaked
        census = TelemetryLogger.last_thread_census
        assert census["clean"] is False
        assert "sky-picker-meta-leak" in census["leaked_threads"]
    finally:
        release.set()
        worker.join(timeout=2.0)
