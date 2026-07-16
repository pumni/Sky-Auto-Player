import sys

import pytest

from sky_music.domain import Millis, Note, NoteKey, Song
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy, KeyAction, TimingPolicy
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import (
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SKIPPED,
    PlaybackEngine,
    PlaybackState,
)


def _frame_policy(d: dict | None = None) -> FrameTimingPolicy:
    return FrameTimingPolicy.from_timing_policy(TimingPolicy.from_dict(d or {}))

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


class AdvancingReadClock(FakeClock):
    """Simulation clock that advances during busy-wait clock reads."""
    def __init__(self, start_us=0, read_step_us=10):
        super().__init__(start_us)
        self.read_step_us = read_step_us

    def now_us(self):
        current_us = self.time_us
        self.time_us += self.read_step_us
        return current_us


class CountingControls:
    def __init__(self):
        self.poll_calls = 0

    def poll(self):
        self.poll_calls += 1
        return


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

    with open(engine.telemetry.log_filepath, newline="") as f:
        reader = csv.DictReader(f)
        rows = list(reader)
        assert len(rows) == 2
        assert "send_duration_us" in rows[0]
        assert "visible_lateness_us" in rows[0]

def test_dispatch_lead_time_triggering_earlier():
    """Verify that dispatch_lead_us triggers authored dispatches earlier by the lead time."""
    song = Song(
        name="LeadTimeSong",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(100), key=NoteKey("Key1")),
        )
    )
    policy = _frame_policy({"hold_us": 20000, "min_hold_us": 20000})
    sched_meta = build_key_actions(song, policy=policy)

    clock = FakeClock()
    sleeper = FakeSleeper(clock)
    backend = DryRunBackend()

    engine = PlaybackEngine(
        song=song, actions=sched_meta.actions, backend=backend,
        telemetry_enabled=True, require_focus=False,
        clock=clock, sleeper=sleeper,
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        dispatch_lead_us=10000,  # 10ms lead time
        # Asserts on engine.telemetry.records after play(); production hygiene clears
        # records inside save(), so opt in to retention for the assertion window only.
        retain_telemetry_records_after_save=True,
    )

    res = engine.play()
    assert res == PLAYBACK_FINISHED

    records = engine.telemetry.records
    key1_downs = [r for r in records if r["kind"] == "down" and r["event_index"] == 2]  # event_index 2 is Note 2 down (event 0: down 0, event 1: up 0, event 2: down 1)
    assert key1_downs
    assert key1_downs[0]["actual_us"] == 90000

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


def test_runtime_polling_is_throttled_during_approach_phase():
    song = Song(name="Polling Cadence", notes=())
    action = KeyAction(kind="down", scan_codes=(0x15,), at_us=10_000, reason="onset")  # type: ignore[arg-type]
    clock = AdvancingReadClock()
    controls = CountingControls()
    engine = PlaybackEngine(
        song=song,
        actions=(action,),
        backend=DryRunBackend(),
        controls=controls,
        telemetry_enabled=False,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=500),
    )

    assert engine.play() == PLAYBACK_FINISHED
    assert 2 <= controls.poll_calls <= 12


def test_final_spin_does_not_poll_controls_or_focus():
    song = Song(name="Pure Final Spin", notes=())
    action = KeyAction(kind="down", scan_codes=(0x15,), at_us=500, reason="onset")  # type: ignore[arg-type]
    clock = AdvancingReadClock()
    controls = CountingControls()
    guard = _CountingFocusGuard(active=True)
    engine = PlaybackEngine(
        song=song,
        actions=(action,),
        backend=DryRunBackend(),
        controls=controls,
        telemetry_enabled=False,
        require_focus=True,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=800),
        focus_guard=guard,
    )

    assert engine.play() == PLAYBACK_FINISHED
    assert controls.poll_calls == 0
    # 1 call from engine.play() startup, 1 call from health_monitor during _execute_action
    assert guard.is_active_calls == 2

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
    state = PlaybackState(start_perf=0)
    state.enter_pause("focus", 0)

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
    state = PlaybackState(start_perf=0)
    state.enter_pause("focus", 0)

    waiting, command = engine._process_wait_states(state, True, 1.0)

    assert waiting is True
    assert command is None
    # Focus reason cleared on regain; manual pause taken during grace keeps the
    # single contiguous interval open (no double-count accumulate until fully unpaused).
    assert not state.has_pause_reason("focus")
    assert state.has_pause_reason("manual")
    assert state.pause_interval_started_us is not None


class _CountingFocusGuard:
    """FocusGuard that records how many times is_active() actually runs."""
    def __init__(self, active: bool = True):
        self.active = active
        self.is_active_calls = 0
    def is_active(self) -> bool:
        self.is_active_calls += 1
        return self.active
    def focus(self) -> bool:
        return True


def _focus_cache_engine(guard, clock):
    song = Song(name="FocusCache", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    sched = build_key_actions(song)
    return PlaybackEngine(
        song=song, actions=sched.actions, backend=DryRunBackend(),
        telemetry_enabled=False, require_focus=True,
        clock=clock, sleeper=FakeSleeper(clock), focus_guard=guard,
    )


def test_focus_check_is_memoised_within_ttl():
    """Repeated focus checks inside the TTL window hit the heavy is_active() only once."""
    clock = FakeClock(start_us=1_000_000)
    guard = _CountingFocusGuard(active=True)
    engine = _focus_cache_engine(guard, clock)

    # Many checks at the same instant -> exactly one real is_active() call.
    for _ in range(50):
        assert engine._focus_is_active() is True
    assert guard.is_active_calls == 1


def test_focus_check_refreshes_after_ttl():
    """Once the TTL elapses, the next check re-queries the guard (so alt-tab is detected)."""
    clock = FakeClock(start_us=1_000_000)
    guard = _CountingFocusGuard(active=True)
    engine = _focus_cache_engine(guard, clock)

    assert engine._focus_is_active() is True
    assert guard.is_active_calls == 1

    # Within TTL: still cached.
    clock.sleep_us(engine._focus_cache_ttl_us - 1)
    assert engine._focus_is_active() is True
    assert guard.is_active_calls == 1

    # Past TTL: focus has been lost in the meantime -> re-queried and observed.
    clock.sleep_us(2)
    guard.active = False
    assert engine._focus_is_active() is False
    assert guard.is_active_calls == 2


@pytest.mark.skipif(sys.platform != "win32", reason="win32 SendInput backend only")
def test_send_scan_code_batch_builds_correct_cached_inputs(monkeypatch):
    """The cached-INPUT fast path must emit the same down/up scan-code events as before.

    Since the Phase-1.3 batch-array cache, send_scan_code_batch calls user32.SendInput directly
    with a cached ctypes array instead of routing through send_input_batch, so the capture seam
    is the SendInput call itself.
    """
    from sky_music.platform.win32 import inputs

    captured = []

    def fake_send_input(count, input_array, struct_size):
        captured.append([input_array[i] for i in range(count)])
        return count

    monkeypatch.setattr(inputs.user32, "SendInput", fake_send_input)

    inputs.send_scan_code_batch((30, 31), key_up=False)
    down_batch = captured[-1]
    assert [ki.ki.wScan for ki in down_batch] == [30, 31]
    assert all(ki.type == inputs.INPUT_KEYBOARD for ki in down_batch)
    assert all(ki.ki.dwFlags == inputs.KEYEVENTF_SCANCODE for ki in down_batch)

    inputs.send_scan_code_batch((30,), key_up=True)
    up_batch = captured[-1]
    assert up_batch[0].ki.dwFlags == (inputs.KEYEVENTF_SCANCODE | inputs.KEYEVENTF_KEYUP)

    # Same (scan_code, flags) reuses the cached object; different flags do not collide.
    assert inputs._cached_key_input(30, inputs.KEYEVENTF_SCANCODE) is inputs._cached_key_input(30, inputs.KEYEVENTF_SCANCODE)
    assert inputs._cached_key_input(30, inputs.KEYEVENTF_SCANCODE) is not inputs._cached_key_input(
        30, inputs.KEYEVENTF_SCANCODE | inputs.KEYEVENTF_KEYUP
    )
