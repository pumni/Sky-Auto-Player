import sys
from pathlib import Path
import pytest
import time
from typing import Tuple, Optional

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.domain import Song, Note, NoteKey, Millis
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import TimingPolicy, KeyAction, FrameTimingPolicy

def _frame_policy(d: dict | None = None) -> FrameTimingPolicy:
    return FrameTimingPolicy.from_timing_policy(TimingPolicy.from_dict(d or {}))
from sky_music.infrastructure.backend import WinSendInputBackend, DryRunBackend
from sky_music.orchestration.engine import (
    PlaybackEngine,
    PlaybackState,
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SKIPPED,
)
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.infrastructure.hotkeys import HotkeyBinding, PlaybackControls

class FakeClock:
    def __init__(self, start_us=0):
        self.time_us = start_us
    def now_us(self):
        return self.time_us
    def sleep_us(self, duration_us):
        self.time_us += duration_us

class FakeSleeper:
    def __init__(self, clock):
        self.clock = clock
    def sleep(self, seconds: float):
        advance = max(1, int(seconds * 1_000_000))
        self.clock.sleep_us(advance)
    def sleep_us(self, duration_us):
        self.clock.sleep_us(max(1, duration_us))

def test_dry_run_playback_execution():
    """Verify PlaybackEngine interacts correctly with the InputBackend and dispatches correct batches."""
    song = Song(
        name="Mock Playback Song",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(50), key=NoteKey("Key1")),
        )
    )

    policy = _frame_policy()
    sched_meta = build_key_actions(song, policy=policy)
    actions = sched_meta.actions

    backend = DryRunBackend()
    engine = PlaybackEngine(
        song=song, actions=actions, backend=backend,
        telemetry_enabled=False, require_focus=False
    )

    res = engine.play()
    assert res == PLAYBACK_FINISHED
    assert len(backend.history) == 4
    assert backend.history[0][0] == "down"
    assert backend.history[2][0] == "down"

def test_dry_run_playback_without_focus():
    """Verify that dry-run playback executes successfully without requiring active Sky window focus."""
    song = Song(name="Focusless", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    policy = _frame_policy()
    sched_meta = build_key_actions(song, policy=policy)

    backend = DryRunBackend()
    engine = PlaybackEngine(
        song=song, actions=sched_meta.actions, backend=backend,
        telemetry_enabled=False, require_focus=False
    )

    res = engine.play()
    assert res == PLAYBACK_FINISHED

def test_telemetry_includes_send_duration_us(tmp_path):
    """Verify that high-precision telemetry logger records and saves the send_duration_us metric."""
    import csv
    song = Song(name="Telemetry", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    policy = _frame_policy()
    sched_meta = build_key_actions(song, policy=policy)

    backend = DryRunBackend()
    engine = PlaybackEngine(
        song=song, actions=sched_meta.actions, backend=backend,
        telemetry_enabled=True, require_focus=False
    )
    engine.telemetry.log_filepath = tmp_path / "test_telemetry.csv"

    res = engine.play()
    assert res == PLAYBACK_FINISHED

    with open(engine.telemetry.log_filepath, "r", newline="") as f:
        reader = csv.DictReader(f)
        rows = list(reader)
        assert len(rows) == 2
        assert "send_duration_us" in rows[0]

def test_deterministic_playback_with_fake_time():
    """Verify that using FakeClock and FakeSleeper runs a long playback instantly with microsecond precision."""
    song = Song(
        name="Long Song",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(5000), key=NoteKey("Key1")),
        )
    )
    policy = _frame_policy({"hold_us": 20000})
    sched_meta = build_key_actions(song, policy=policy)

    clock = FakeClock()
    sleeper = FakeSleeper(clock)
    backend = DryRunBackend()

    engine = PlaybackEngine(
        song=song, actions=sched_meta.actions, backend=backend,
        telemetry_enabled=False, require_focus=False,
        clock=clock, sleeper=sleeper,
        sleep_policy=SleepPolicy(spin_threshold_us=-1)
    )

    res = engine.play()
    assert res == PLAYBACK_FINISHED
    assert clock.now_us() >= 5_020_000

class MockControls:
    def __init__(self, command=None):
        self.command = command
        self.enabled = True
    def poll(self):
        return self.command


class SequenceControls:
    def __init__(self, commands):
        self.commands = list(commands)
        self.enabled = True

    def poll(self):
        if not self.commands:
            return None
        return self.commands.pop(0)

def test_playback_quit_command():
    song = Song(name="QuitTest", notes=(Note(time_ms=Millis(1000), key=NoteKey("Key0")),))
    sched = build_key_actions(song)
    backend = DryRunBackend()
    controls = MockControls(command="quit")
    
    engine = PlaybackEngine(
        song=song, actions=sched.actions, backend=backend,
        telemetry_enabled=False, require_focus=False,
        controls=controls
    )
    
    res = engine.play()
    assert res == PLAYBACK_QUIT
    assert len(backend.history) == 0 # Quit before first note

def test_playback_skip_command():
    song = Song(name="SkipTest", notes=(Note(time_ms=Millis(1000), key=NoteKey("Key0")),))
    sched = build_key_actions(song)
    backend = DryRunBackend()
    controls = MockControls(command="skip")
    
    engine = PlaybackEngine(
        song=song, actions=sched.actions, backend=backend,
        telemetry_enabled=False, require_focus=False,
        controls=controls
    )
    
    res = engine.play()
    assert res == PLAYBACK_SKIPPED


def test_focus_restore_grace_handles_skip_command():
    song = Song(name="GraceSkip", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    sched = build_key_actions(song)
    clock = FakeClock()
    sleeper = FakeSleeper(clock)
    backend = DryRunBackend()
    controls = SequenceControls(["skip"])
    engine = PlaybackEngine(
        song=song,
        actions=sched.actions,
        backend=backend,
        controls=controls,
        telemetry_enabled=False,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        focus_restore_grace_us=50_000,
    )
    state = PlaybackState(start_perf=0, focus_pause_started_us=0)

    waiting, command = engine._process_wait_states(state, True, 1.0)

    assert waiting is True
    assert command == PLAYBACK_SKIPPED


def test_focus_restore_grace_handles_pause_command():
    song = Song(name="GracePause", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    sched = build_key_actions(song)
    clock = FakeClock()
    sleeper = FakeSleeper(clock)
    backend = DryRunBackend()
    controls = SequenceControls(["pause"])
    engine = PlaybackEngine(
        song=song,
        actions=sched.actions,
        backend=backend,
        controls=controls,
        telemetry_enabled=False,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        focus_restore_grace_us=50_000,
    )
    state = PlaybackState(start_perf=0, focus_pause_started_us=0)

    waiting, command = engine._process_wait_states(state, True, 1.0)

    assert waiting is True
    assert command is None
    assert state.focus_pause_started_us is None
    assert state.manual_pause_started_us is not None
    assert state.pause_time_us > 0
