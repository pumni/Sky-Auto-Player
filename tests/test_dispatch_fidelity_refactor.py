from __future__ import annotations

from sky_music.domain import Millis, Note, NoteKey, Song
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import (
    FrameTimingPolicy,
    Microseconds,
)
from sky_music.infrastructure.backend import DryRunBackend, InputSendResult
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.core.coordinator import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)
from sky_music.orchestration.core.loop import (
    DispatchHealthMonitor,
    DispatchLoop,
    PlaybackState,
)
from sky_music.orchestration.telemetry import TelemetryLogger


class FakeClock:
    def __init__(self, start_us=0):
        self.time_us = start_us
    def now_us(self):
        return self.time_us

class FakeSleeper:
    def __init__(self, clock):
        self.clock = clock
    def sleep(self, seconds: float):
        self.clock.time_us += max(1, int(seconds * 1_000_000))

class MockWaitStrategy:
    def __init__(self, clock):
        self.clock = clock
        self.waits = []
        self.sleeps = []
        self.last_spin_threshold_us = 0
    def spin_until_us(self, target_system_us, clock):
        self.clock.time_us = target_system_us
    def wait_until_us(self, target_system_us, clock, sleeper, spin_threshold_us, policy, command_event=None):
        self.last_spin_threshold_us = spin_threshold_us
        now = self.clock.now_us()
        if target_system_us - now > spin_threshold_us:
            # We would sleep
            self.sleeps.append((now, target_system_us))
            # Advance clock by 100ms chunks to simulate wait, but never beyond target - spin_threshold
            max_sleep = target_system_us - now - spin_threshold_us
            step = min(100_000, max_sleep)
            clock.time_us += step
            return True
        return False

class MockCommandSource:
    def poll(self): return None

class MockFocusSignal:
    def is_active(self) -> bool:
        return True
        
    @property
    def has_focus(self) -> bool:
        return True
        
    def focus(self) -> bool:
        return True
        
    def check_sync(self) -> bool:
        return True

class MockProgressSink:
    def publish(
        self, *, elapsed_us: int, total_us: int, status: str, lateness_us: int | None = None, health=None, input_path_degraded: bool = False, force: bool = False, counters=None,
    ) -> None:
        pass
        
    def finish(self, message: str) -> None:
        pass

def create_song(notes):
    return Song(name="test", notes=tuple(notes))

def setup_loop(clock, notes, core_warmup_budget_us=500, hold_us=100_000):
    song = create_song(notes)
    timing_policy = FrameTimingPolicy(
        fps=60,
        frame_us=Microseconds(16667),
        hold_us=Microseconds(hold_us),
        min_hold_us=Microseconds(hold_us),
        focus_restore_grace_us=Microseconds(100_000)
    )
    metadata = build_key_actions(song, policy=timing_policy)
    intents = compile_runtime_intents(metadata.actions)
    coordinator = RuntimeDispatchCoordinator(intents, min_hold_us=hold_us)
    backend = DryRunBackend()
    sleeper = FakeSleeper(clock)
    wait_strategy = MockWaitStrategy(clock)
    telemetry = TelemetryLogger("test")
    health = DispatchHealthMonitor(backend, clock, MockFocusSignal(), require_focus=False)
    
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=sleeper,
        wait_strategy=wait_strategy,
        backend=backend,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(poll_s=0.002),
        health_monitor=health,
        min_hold_us=hold_us,
        spin_threshold_us=1_000,
        core_warmup_budget_us=core_warmup_budget_us,
    )
    return loop, wait_strategy

def test_warmup_cold_detection_uses_elapsed_clock_domain_after_epoch_rebase():
    # Set the raw clock to a large value (10_000_000_000 us = 10k seconds)
    clock = FakeClock(10_000_000_000)
    
    # Schedule two notes separated by 5ms
    # 5ms is > spin_threshold (1ms) so it enters the wait loop, 
    # but < SEND_COLD_THRESHOLD_US (20ms) so it legitimately shouldn't trigger warmup
    notes = [
        Note(time_ms=Millis(0), key=NoteKey("Key1")),
        Note(time_ms=Millis(5), key=NoteKey("Key2"))
    ]
    loop, wait_strategy = setup_loop(clock, notes, hold_us=10_000)
    
    # Track spin thresholds
    spins = []
    
    # We patch WaitStrategy instead of the loop hook
    original_wait = wait_strategy.wait_until_us
    def mock_wait(*args, target_system_us=0, spin_threshold_us=0, **kwargs) -> bool:
        spins.append(spin_threshold_us)
        return original_wait(target_system_us=target_system_us, spin_threshold_us=spin_threshold_us, **kwargs)
        
    wait_strategy.wait_until_us = mock_wait
    
    state = PlaybackState(start_perf=clock.now_us())
    
    loop.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(), total_time_us=2000)
    
    # Note 1 completes at roughly elapsed=0
    # Note 2 is evaluated at elapsed=1000
    # The gap in elapsed time is ~1000 us, which is < SEND_COLD_THRESHOLD_US (20_000)
    # The correct behavior is that it should NOT have the 500us budget applied for Note 2.
    assert len(spins) >= 2 and all(s == 1000 for s in spins)
    assert spins[1] == 1000, f"Cold guard budget should not be applied, expected 1000, got: {spins[1]}"

def test_cold_guard_is_adjacent_to_final_spin_after_long_gap():
    clock = FakeClock(0)
    # Long gap: 0ms then 1000ms
    notes = [
        Note(time_ms=Millis(0), key=NoteKey("Key1")),
        Note(time_ms=Millis(1000), key=NoteKey("Key2"))
    ]
    loop, wait_strategy = setup_loop(clock, notes)
    
    state = PlaybackState(start_perf=clock.now_us())
    loop.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(), total_time_us=2000000)
    
    # Wait until us is called with the increased threshold (1000 + 500 = 1500)
    assert wait_strategy.last_spin_threshold_us == 1500
    
    # The actual sleep/spin tracking is delegated to wait_strategy.
    # The sleep time should leave exactly the effective spin threshold (1500) remaining.
    assert len(wait_strategy.sleeps) > 0
    last_sleep = wait_strategy.sleeps[-1]
    
    # Sleep target is the second note's scheduled time
    target_us = 1_000_000
    # Wait strategy stops simulating sleeps when (target - now) <= spin_threshold
    assert (target_us - last_sleep[1]) <= 1500

def test_default_cold_guard_budget_is_200_us():
    import inspect

    from sky_music.orchestration.core.loop import DispatchLoop
    
    # The loop should use exactly 200us extra for the cold guard by default
    sig = inspect.signature(DispatchLoop.__init__)
    assert sig.parameters['core_warmup_budget_us'].default == 200, "Default cold guard budget should be 200us"

def test_first_mid_song_reprobe_is_not_due_before_interval():
    clock = FakeClock()
    notes = [
        Note(time_ms=Millis(0), key=NoteKey("Key1")),
        Note(time_ms=Millis(10_000), key=NoteKey("Key1")) # 10s gap, > REPROBE_MIN_GAP_US (0.5s)
    ]
    loop, _ = setup_loop(clock, notes)
    # Mock mid song reprobe to track if it happened
    reprobes = []
    def mock_reprobe(elapsed_us):
        reprobes.append(elapsed_us)
        loop._last_reprobe_elapsed_us = elapsed_us
    loop._run_mid_song_reprobe = mock_reprobe
    state = PlaybackState(start_perf=clock.now_us())
    
    # The first reprobe should not happen until 30s interval
    loop.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(), total_time_us=10_000_000)
    
    # Check telemetry for reprobes
    assert len(reprobes) == 0, f"Reprobe occurred before the 30s interval at {reprobes}"

def test_send_completion_is_sampled_at_platform_call_return_boundary():
    # F5: `send_completed_us` is sampled by the backend after the platform wrapper has already returned

    from sky_music.infrastructure.backend import WinSendInputBackend
    
    clock = FakeClock()
    
    class FakeInputsModule:
        def send_scan_code_batch_trusted(self, scan_codes, key_up, **kwargs):
            # Advance clock DURING the call to simulate kernel time
            clock.time_us += 100_000
            # Sample time right after the simulated kernel call, before any post-processing
            sampled_time = clock.now_us()
            # Simulate some wrapper overhead after sampling
            clock.time_us += 5_000 
            return len(scan_codes), sampled_time

    backend = WinSendInputBackend()
    backend.inputs_module = FakeInputsModule()  # type: ignore
    backend._now_us = clock.now_us
    _start_time = clock.now_us()

    _sent, completed_us = backend._emit((0x10,), key_up=False)

    # We expect `completed_us` to be sampled INSIDE send_scan_code_batch_trusted (at 100_000),
    # NOT after the method returns (which would be 105_000).
    assert completed_us == 100_000, f"Expected 100_000, got: {completed_us}"

def test_exact_timestamp_chord_uses_one_sendinput_call():
    # This just ensures we call backend._emit exactly once for a chord
    clock = FakeClock()
    notes = [
        Note(time_ms=Millis(0), key=NoteKey("Key1")),
        Note(time_ms=Millis(0), key=NoteKey("Key2")),
        Note(time_ms=Millis(0), key=NoteKey("Key3")),
    ]
    loop, _ = setup_loop(clock, notes)
    
    from unittest.mock import MagicMock, patch

    from sky_music.infrastructure.backend import InputSendResult
    from sky_music.platform.win32 import inputs
    loop.backend = MagicMock()
    loop.backend.key_down.return_value = InputSendResult(sent=tuple(1 for _ in notes), skipped_duplicates=(), success=True, send_completed_us=1000)
    
    state = PlaybackState(start_perf=clock.now_us())
    loop.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(), total_time_us=1000)
    
    # Verify key_down was called EXACTLY ONCE with all 3 scan codes
    assert loop.backend.key_down.call_count == 1
    args, _kwargs = loop.backend.key_down.call_args
    assert len(args[0]) == 3, "Chord was split into multiple key_down calls"


    with patch.object(inputs.user32, "SendInput", return_value=3) as native_send:
        native_result = inputs.send_scan_code_batch_trusted(
            (0x1E, 0x1F, 0x20),
            clock_now=lambda: 123,
        )

    native_send.assert_called_once()
    assert native_result.inserted == 3

def test_single_key_trusted_batch_avoids_duplicate_set_allocation():
    """F6 regression + Phase 3 §10.1: the trusted single-key path must take
    the explicit one-key fast path, NOT allocate a ``set(scan_codes)`` and
    NOT trigger any partial-send bookkeeping for a clean send.

    Behavioral test: drive ``send_scan_code_batch_trusted((0x1E,), ...)`` with
    a mocked ``user32.SendInput`` that returns the requested count (full
    success).  We then assert:
      - exactly one ``SendInput`` call was made (no retry),
      - the returned ``PlatformSendResult`` reports the single key as inserted,
      - the partial-send diagnostic counters (``_DIAG.keys_retried``,
        ``_DIAG.keys_dropped``, ``_DIAG.partial_send_events``) did not move.
    A future refactor that reintroduces ``set(scan_codes)`` allocation on the
    hot path would, in real code, need an extra pre-validation pass that
    would splatter these counters or split the SendInput — both observable.
    """
    from unittest.mock import patch

    from sky_music.platform.win32 import inputs

    inputs.reset_send_diagnostics()
    send_patch = patch.object(inputs.user32, "SendInput", return_value=1)
    send_mock = send_patch.start()
    try:
        with patch("builtins.set", side_effect=AssertionError("set allocation regressed")):
            result = inputs.send_scan_code_batch_trusted((0x1E,), key_up=False)
    finally:
        send_patch.stop()

    # One-key contract: exactly one SendInput call, no retry, no set alloc.
    assert send_mock.call_count == 1, (
        f"single-key trusted path made {send_mock.call_count} SendInput calls; "
        f"expected exactly 1 (one-key fast path, no partial retry)."
    )
    assert result.inserted == 1, (
        f"inserted={result.inserted}; expected 1 for the single scan code."
    )
    assert inputs._DIAG.keys_retried == 0
    assert inputs._DIAG.keys_dropped == 0
    assert inputs._DIAG.partial_send_events == 0
    # Belt-and-braces: requested count matches the input scan-code tuple length,
    # which means the trusted path did not split the input or trim it.
    assert result.requested == 1


def test_progress_publication_does_not_lock_per_dispatch():
    """F7 regression: dispatch hot path must not acquire the cross-thread
    progress lock per send.

    Behavioral test: run a dense synthetic block of N chord dispatches in one
    ``loop.run`` call; count ``ProgressSink.publish`` invocations. The lock is
    acquired in ``SnapshotProgressSink.publish`` — its count must be far lower
    than the dispatch count (Phase 3 §10.2 cadence: ~30 ms snapshots).
    """
    import threading
    from unittest.mock import MagicMock, patch

    from sky_music.orchestration import playback_supervisor
    from sky_music.orchestration.playback_supervisor import SnapshotProgressSink

    # 20 notes at 1 ms apart — 20 dispatches in 20 ms (~1000 dispatches/s).
    # Cycling through Key0..Key14 (the Sky 15-key layout) every 15 notes means
    # each chord is single-key with a long sleep between same-key repeats, so
    # same-key conflict / dedup does not muddy the publish count.
    keys = [NoteKey(f"Key{i % 15}") for i in range(20)]
    notes = [
        Note(time_ms=Millis(i * 1), key=keys[i]) for i in range(20)
    ]
    clock = FakeClock(0)
    loop, _ = setup_loop(clock, notes, hold_us=10_000)

    backend = MagicMock()
    backend.key_down.return_value = InputSendResult(
        sent=(0x1E,), skipped_duplicates=(), success=True, send_completed_us=0
    )
    loop.backend = backend

    real_lock = threading.Lock
    lock_acquisitions = 0

    class _CountingLock:
        def __init__(self) -> None:
            self._lock = real_lock()

        def __enter__(self) -> _CountingLock:
            nonlocal lock_acquisitions
            lock_acquisitions += 1
            self._lock.acquire()
            return self

        def __exit__(self, *_args: object) -> None:
            self._lock.release()

    with patch.object(playback_supervisor.threading, "Lock", _CountingLock):
        progress_sink = SnapshotProgressSink()
        state = PlaybackState(start_perf=clock.now_us())
        loop.run(
            state,
            MockCommandSource(),
            MockFocusSignal(),
            progress_sink,
            total_time_us=20_000,
        )

    # Dispatch count == backend.key_down call count (one dispatch per note).
    dispatch_count = backend.key_down.call_count
    publish_count = lock_acquisitions
    assert dispatch_count == 20, (
        f"Expected 20 dispatches for 20 dense notes, got {dispatch_count}"
    )
    # publish() may fire for pause/focus transitions and the final "finished"
    # — but it MUST NOT fire once per dispatched note (that is the F7 defect).
    # The cadence contract (plan §10.2.1) is "30–50 ms snapshot cadence"; over
    # a 20 ms dense burst we expect publish_count to be <= a small fixed number
    # of lifecycle publishes (refocus finish / final status), well below 20.
    assert publish_count < dispatch_count, (
        f"publish() was called {publish_count} times for {dispatch_count} "
        f"dispatches — per-dispatch lock acquisition regressed (F7)."
    )


def test_telemetry_cap_never_flushes_on_dispatch_thread():
    """F8 regression: ``TelemetryLogger.record()`` must not trigger disk I/O
    on the dispatch thread when the hard cap is reached.

    Behavioral test: fill the in-memory buffer past ``_TELEMETRY_MAX_BUFFER``
    on a TelemetryLogger whose log_filepath points at a temp dir, then assert
    no CSV file was created / opened / written by ``record()``.
    """
    import tempfile
    from pathlib import Path

    from sky_music.orchestration.telemetry import _TELEMETRY_MAX_BUFFER, TelemetryLogger

    with tempfile.TemporaryDirectory() as tmp:
        logger = TelemetryLogger(
            song_name="cap_test", enabled=True,
        )
        logger.log_filepath = Path(tmp) / "telemetry.csv"

        # record() at least 2x the hard cap → cap is hit on the very first call
        # and again before this loop ends. We submit >cap records so the
        # drop-half path runs more than once.
        above_cap = _TELEMETRY_MAX_BUFFER + 5
        for i in range(above_cap):
            logger.record(
                event_index=i, kind="down",
                scheduled_us=i * 1000, actual_us=i * 1000 + 1,
                lateness_us=1, send_duration_us=1,
                scan_codes=(0x1E,), reason="note",
                sent_scan_codes=(0x1E,),
            )

        # record() must have stayed in-memory: no CSV file exists yet.
        assert not logger.log_filepath.exists(), (
            f"{logger.log_filepath} was created by record() — disk I/O on the "
            f"dispatch thread regressed (F8)."
        )
        # The cap policy must have dropped records (drop-half) and bumped the
        # honest dropped counter.
        assert logger._dropped_count > 0, (
            "Hard cap reached but _dropped_count stayed 0 — telemetry honesty "
            "regression."
        )
        # And the in-memory list is bounded by the cap (never grows past it
        # by more than one append pre-drop).
        assert len(logger.records) <= _TELEMETRY_MAX_BUFFER, (
            f"records list grew to {len(logger.records)} > cap "
            f"{_TELEMETRY_MAX_BUFFER} — unbounded memory regression."
        )


def test_waitable_timer_recomputes_remaining_immediately_before_arm():
    """F9 regression: relative waitable-timer duration must be recomputed from
    a clock sample taken immediately before arming, not from a sample taken
    before setup.

    Behavioral test: use a fake clock that advances between the initial
    ``now_us`` and the moment ``set_waitable_timer_relative_us`` is invoked.
    Assert the value handed to ``SetWaitableTimer`` reflects the *post-setup*
    clock sample, i.e. the recomputed remaining interval.
    """
    from sky_music.infrastructure.timing import Clock, RealSleeper, SleepPolicy
    from sky_music.infrastructure.wait_strategy import HybridWaitStrategy

    # Fake clock with manual advance-on-now semantics.
    class AdvanceClock(Clock):
        def __init__(self, start_us: int, advance_after_first_read_per_call: int):
            self._t = start_us
            self._drift = advance_after_first_read_per_call
            self._reads_in_call = 0

        def now_us(self) -> int:
            v = self._t
            # Each successive read within a single wait_until_us invocation
            # returns a larger value to simulate clock drift during setup.
            self._t += self._drift
            return v

        def reset(self) -> None:
            self._reads_in_call = 0

    # We need the strategy to take the "high-resolution waitable timer" path,
    # so provide a sleeper that advertises is_high_resolution and exposes a
    # handle for set_waitable_timer_relative_us.
    class FakeHighResSleeper(RealSleeper):
        is_high_resolution = True
        handle = 9999

    # Monkeypatch the inputs module so we never touch real Win32.
    armed_durations: list[int] = []
    wait_results: list[int] = []

    from sky_music.platform.win32 import inputs as inputs_mod


    def fake_set_timer(handle, delay_us):
        armed_durations.append(delay_us)
        return True

    def fake_wait(handles, timeout_ms):
        wait_results.append(timeout_ms)
        return inputs_mod.WAIT_OBJECT_0  # timer-signaled path

    clock = AdvanceClock(start_us=0, advance_after_first_read_per_call=5000)
    # First read returns 0, second read returns 5000 (post-setup drift).
    # If the strategy computes remaining from the FIRST read, the value
    # would be remaining = target - 0 - guard = target - guard.
    # If recomputed from the SECOND read, it would be target - 5000 - guard.
    target_us = 100_000
    guard_us = 1000
    expected_if_stale = target_us - guard_us  # 99_000
    expected_if_recomputed = target_us - 5000 - guard_us  # 94_000

    sleeper = FakeHighResSleeper()
    policy = SleepPolicy(poll_s=0.002)
    strategy = HybridWaitStrategy(enable_event_wait=True)

    import unittest.mock as _mock

    with _mock.patch.object(
        inputs_mod, "set_waitable_timer_relative_us", side_effect=fake_set_timer
    ), _mock.patch.object(
        inputs_mod, "wait_for_multiple_objects", side_effect=fake_wait
    ), _mock.patch.object(
        inputs_mod, "WAIT_OBJECT_0", inputs_mod.WAIT_OBJECT_0
    ):
        strategy.wait_until_us(
            target_system_us=target_us,
            clock=clock,
            sleeper=sleeper,
            spin_threshold_us=guard_us,
            policy=policy,
            command_event=12345,
        )

    assert armed_durations, (
        "wait_until_us did not invoke set_waitable_timer_relative_us — the "
        "high-resolution timer path was not exercised."
    )
    actual = armed_durations[0]
    assert actual == expected_if_recomputed, (
        f"Armed with stale remaining {actual} µs; expected recomputed "
        f"{expected_if_recomputed} µs (with first-read stale value being "
        f"{expected_if_stale} µs). F9 regression."
    )


def test_auto_priority_never_selects_time_critical_fallback():
    """F10 regression: under ``mode='auto'`` the priority scope must never
    report ``thread:time_critical`` as the acquired tier, even if MMCSS and
    HIGHEST both fail. ``TIME_CRITICAL`` is only permissible as an explicit
    user-selected expert mode.

    Behavioral test: drive ``DispatchThreadPriorityScope(mode='auto')`` with
    all rungs stubbed to fail, then assert the outcome tier is ``'off'`` —
    not ``'thread:time_critical'`` and not any string containing
    ``'time_critical'``.
    """
    import unittest.mock as _mock

    from sky_music.infrastructure.rt_priority import DispatchThreadPriorityScope

    fake_handle = 4242
    with _mock.patch(
        "sky_music.platform.win32.inputs.av_set_mm_thread_characteristics",
        return_value=None,
    ), _mock.patch(
        "sky_music.platform.win32.inputs.get_current_thread", return_value=fake_handle
    ), _mock.patch(
        "sky_music.platform.win32.inputs.get_thread_priority", return_value=0
    ), _mock.patch(
        "sky_music.platform.win32.inputs.set_thread_priority", return_value=False
    ), _mock.patch(
        "sky_music.infrastructure.rt_priority.inputs.disable_thread_power_throttling",
        return_value=False,
    ):
        scope = DispatchThreadPriorityScope(mode="auto")
        scope.__enter__()
        try:
            outcome = scope.outcome
        finally:
            scope.__exit__(None, None, None)

    assert outcome is not None, "Priority scope did not produce an outcome"
    assert "time_critical" not in str(outcome.acquired).lower(), (
        f"auto mode resolved to {outcome.acquired!r} — auto fallback to "
        f"TIME_CRITICAL must be impossible (F10)."
    )
    assert outcome.acquired == "off", (
        f"auto mode with all rungs failing should degrade to 'off', got "
        f"{outcome.acquired!r}"
    )


def test_partial_send_remains_bounded_and_safe():
    """Plan §16.2 / Phase 2 acceptance: a partial-send on a chord must stay
    bounded and safe — exactly one immediate same-frame retry on the musical
    note-on path, no late sleep-retry, dropped keys honestly tallied.

    Behavioral test: stub ``user32.SendInput`` to return 1 for a 4-key
    request on the first call (a partial down-send) and then 1 again on the
    immediate retry (still partial).  Per the musical no-retry policy
    (plan §2 invariants, plan §2 B5/G5), we expect:
      - exactly **two** ``SendInput`` invocations (first + same-frame retry),
      - **no** ``_retry_wait_seconds`` sleep invoked (this is the late-retry
        forbidden path),
      - ``_DIAG.partial_send_events == 1`` (one chord split),
      - ``_DIAG.keys_dropped == 2`` (2 keys still unsent after the retry),
      - the trusted result reports ``inserted == 2`` (1 + 1) for ``requested == 4``.
    """
    from unittest.mock import patch

    from sky_music.platform.win32 import inputs

    inputs.reset_send_diagnostics()

    with patch.object(
        inputs.user32, "SendInput",
        side_effect=[1, 1],  # first call lands 1/4, retry lands 1/3
    ) as send_mock, patch.object(
        inputs, "_retry_wait_seconds",
    ) as retry_wait_mock:
        result = inputs.send_scan_code_batch_trusted(
            (0x1E, 0x1F, 0x20, 0x21), key_up=False,
        )

    # Exactly two SendInput calls — no late-retry sleep, no third attempt.
    assert send_mock.call_count == 2, (
        f"partial down-send made {send_mock.call_count} SendInput calls; "
        f"expected exactly 2 (first + same-frame retry)."
    )
    # The musical no-retry policy forbids the 2 ms sleep retry path.
    retry_wait_mock.assert_not_called()

    # Honest diagnostic counters must be incremented exactly.
    assert inputs._DIAG.partial_send_events == 1, (
        f"partial_send_events={inputs._DIAG.partial_send_events} (expected 1)."
    )
    # First sent 1, retry sent 1 → 2 keys sent total, 2 keys still unsent.
    assert result.inserted == 2, (
        f"inserted={result.inserted}; expected sent(1) + retry(1) == 2."
    )
    assert result.requested == 4
    # The remainder (2 keys) is honestly dropped — no stuck-key tracking,
    # no late retry.  The coordinator promotes these to DROPPED_BACKEND.
    assert inputs._DIAG.keys_dropped == 2, (
        f"keys_dropped={inputs._DIAG.keys_dropped} (expected 2)."
    )
    assert inputs._DIAG.keys_retried == 1, (
        f"keys_retried={inputs._DIAG.keys_retried} (expected 1 — the 1 key "
        f"recovered by the same-frame retry)."
    )


def test_partial_send_release_completes_remainder():
    """Plan §16.2: note-off (``key_up=True``) MUST complete the remainder on
    partial send, since a stuck key is unacceptable for the safety path.

    Behavioral test: ``user32.SendInput`` returns 1 for 4 requested on the
    first call (partial).  Because ``key_up=True`` is the safety path,
    ``_send_scan_code_batch_impl`` is invoked with ``complete_remainder=True``
    and must fall through to ``send_input_batch`` for the remaining 3 keys.
    We stub the eventual remaining batches to succeed (3/3) and expect:
      - first + remainder-completion calls all succeed (no keys dropped),
      - the trusted result reports ``inserted == 4`` (all keys released).
    """
    from unittest.mock import patch

    from sky_music.platform.win32 import inputs

    inputs.reset_send_diagnostics()

    # First call returns 1/4. The remainder path then calls SendInput again
    # with the 3-key leftover; we return 3 there (full success of the
    # remainder).  No more calls allowed.
    with patch.object(
        inputs.user32, "SendInput",
        side_effect=[1, 3],
    ) as send_mock, patch.object(
        inputs, "_retry_wait_seconds",
    ) as retry_wait_mock:
        result = inputs.send_scan_code_batch_trusted(
            (0x1E, 0x1F, 0x20, 0x21), key_up=True,
        )

    assert send_mock.call_count == 2, (
        f"partial up-send made {send_mock.call_count} SendInput calls; "
        f"expected exactly 2 (first + remainder-completion)."
    )
    # The release safety path is allowed (but not required) to use the
    # bounded remainder batch path; late sleep-retry is still forbidden here
    # because the remainder fits in one SendInput call.
    retry_wait_mock.assert_not_called()

    # All keys released — none dropped on the safety path.
    assert result.inserted == 4, (
        f"inserted={result.inserted}; expected all 4 keys released "
        f"(first 1 + remainder 3)."
    )
    assert inputs._DIAG.keys_dropped == 0, (
        f"keys_dropped={inputs._DIAG.keys_dropped} — a partial note-off must "
        f"never drop keys (stuck-key safety)."
    )


def test_one_key_and_chord_telemetry_timestamp_relationships():
    """Plan §16.5 mục 6 + §3 measurement-model terminology — verify the
    per-event telemetry fields describe the documented timestamp chain.

    Behavioral test:
      - For a single-key down dispatch, ``send_completed_us`` (sampled by the
        platform seam) and ``dispatch_completed_us`` (elapsed playback time
        at completion) both fall on the SendInput-return boundary; ``actual_us``
        is the call-entry timeline instant; ``send_duration_pure_us`` is the
        native-call cost; ``bookkeeping_us`` is the Python tail after the
        native return; ``visible_lateness_us == dispatch_completed_us -
        scheduled_us``.

      - For a 3-key chord (chord_stagger_us == 0 fidelity mode) the same
        chain holds AND ``backend.key_down`` is invoked exactly once with the
        3-tuple, so all three members share one ``dispatch_completed_us``.

    Invariants live in the TelemetryRecord structure rather than the CSV
    layer so the test is deterministic under both retain-records and the
    deferred-export path.
    """
    from unittest.mock import MagicMock

    from sky_music.domain.domain import Millis, Note, NoteKey, Song
    from sky_music.domain.scheduler import build_key_actions
    from sky_music.domain.scheduler_types import (
        FrameTimingPolicy,
        Microseconds,
    )
    from sky_music.infrastructure.backend import InputSendResult
    from sky_music.orchestration.core.coordinator import (
        RuntimeDispatchCoordinator,
        compile_runtime_intents,
    )
    from sky_music.orchestration.telemetry import TelemetryLogger

    policy = FrameTimingPolicy(
        fps=120, frame_us=Microseconds(8333),
        hold_us=Microseconds(50_000), min_hold_us=Microseconds(50_000),
        focus_restore_grace_us=Microseconds(100_000),
    )

    # Wrapper that advances the raw clock during the backend call so the
    # platform seam's ``send_completed_us`` (raw perf_counter µs) sits
    # BEFORE ``send_end_raw`` (orchestration read taken after backend returns).
    # In real Win32 the native return is the moment we sample, then
    # bookkeeping runs in Python — so send_end_raw >= send_completed_us.
    class _DelayedKeyBackend(DryRunBackend):
        def __init__(self, real, clock, completion_us: int, bookkeping_us: int = 20):
            super().__init__()
            self._real = real
            self._clock = clock
            self._completion_us = completion_us
            self._bookkeping_us = bookkeping_us

        def key_down(self, scan_codes):
            self._clock.time_us = self._completion_us
            r = self._real.key_down(scan_codes)
            self._clock.time_us = self._completion_us + self._bookkeping_us
            return r

        def key_up(self, scan_codes):
            self._clock.time_us = self._completion_us
            r = self._real.key_up(scan_codes)
            self._clock.time_us = self._completion_us + self._bookkeping_us
            return r

        def __getattr__(self, item):
            return getattr(self._real, item)

    # --- Single-key case --------------------------------------------------
    clock_single = FakeClock(0)
    song = Song(name="one_key", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    metadata = build_key_actions(song, policy=policy)
    intents = compile_runtime_intents(metadata.actions)
    coord = RuntimeDispatchCoordinator(intents, min_hold_us=50_000)
    single_backend = MagicMock()
    single_backend.key_down.return_value = InputSendResult(
        sent=(0x1E,), skipped_duplicates=(), success=True,
        send_completed_us=100,
    )
    single_backend.key_up.return_value = InputSendResult(
        sent=(0x1E,), skipped_duplicates=(), success=True,
        send_completed_us=200,
    )
    delayed_single = _DelayedKeyBackend(single_backend, clock_single, 100, 20)
    telemetry = TelemetryLogger(
        "one_key", enabled=True, retain_records_after_save=True,
    )
    health = DispatchHealthMonitor(
        delayed_single, clock_single, MockFocusSignal(), require_focus=False,
    )
    loop_single = DispatchLoop(
        coordinator=coord, clock=clock_single,
        sleeper=FakeSleeper(clock_single),
        wait_strategy=MockWaitStrategy(clock_single),
        backend=delayed_single, telemetry=telemetry,
        sleep_policy=SleepPolicy(poll_s=0.002),
        health_monitor=health,
        min_hold_us=50_000, spin_threshold_us=1_000,
    )
    state = PlaybackState(start_perf=clock_single.now_us())
    loop_single.run(state, MockCommandSource(), MockFocusSignal(), MockProgressSink(),
                   total_time_us=60_000)

    # Find the down record for the single key.
    down_records = [r for r in telemetry.records if r.kind == "down"]
    assert down_records, "no down telemetry recorded for the single-key send"
    down = down_records[0]
    # The completion timestamp propagated by the platform seam is visible at
    # the orchestration layer — the contract that plan §3 / Phase 2 mandates.
    assert down.dispatch_completed_us == state.get_elapsed_us(
        clock_single, 100
    ), (
        f"single-key dispatch_completed_us={down.dispatch_completed_us}; "
        f"expected elapsed-of(send_completed_us=100)="
        f"{state.get_elapsed_us(clock_single, 100)} (Phase 2 contract)."
    )
    # Pure send duration is measured from call-entry raw clock to native
    # return; bookkeeping is whatever Python tail runs after that point.
    assert down.send_duration_pure_us == 100 - down.actual_us, (
        f"send_duration_pure_us={down.send_duration_pure_us}; expected "
        f"send_completed_us(100) - actual_us({down.actual_us})."
    )
    assert down.bookkeeping_us >= 0
    # Visible lateness = completion elapsed - authored time, per plan §3.
    assert down.dispatch_completed_us is not None
    assert down.visible_lateness_us == down.dispatch_completed_us - down.scheduled_us, (
        f"visible_lateness_us={down.visible_lateness_us} != "
        f"dispatch_completed_us({down.dispatch_completed_us}) - "
        f"scheduled_us({down.scheduled_us}) — plan §3 model regression."
    )

    # --- Chord case (chord_stagger_us == 0 fidelity mode) -----------------
    clock_chord = FakeClock(0)
    chord_song = Song(
        name="chord",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(0), key=NoteKey("Key1")),
            Note(time_ms=Millis(0), key=NoteKey("Key2")),
        ),
    )
    chord_metadata = build_key_actions(chord_song, policy=policy)
    chord_intents = compile_runtime_intents(chord_metadata.actions)
    chord_coord = RuntimeDispatchCoordinator(chord_intents, min_hold_us=50_000)
    chord_backend = MagicMock()
    # Platform seam reports all 3 keys inserted in one SendInput.
    chord_backend.key_down.return_value = InputSendResult(
        sent=(0x1E, 0x1F, 0x20), skipped_duplicates=(), success=True,
        send_completed_us=500,
    )
    chord_backend.key_up.return_value = InputSendResult(
        sent=(0x1E, 0x1F, 0x20), skipped_duplicates=(), success=True,
        send_completed_us=600,
    )
    delayed_chord = _DelayedKeyBackend(chord_backend, clock_chord, 500, 20)
    chord_telemetry = TelemetryLogger(
        "chord", enabled=True, retain_records_after_save=True,
    )
    chord_health = DispatchHealthMonitor(
        delayed_chord, clock_chord, MockFocusSignal(), require_focus=False,
    )
    loop_chord = DispatchLoop(
        coordinator=chord_coord, clock=clock_chord,
        sleeper=FakeSleeper(clock_chord),
        wait_strategy=MockWaitStrategy(clock_chord),
        backend=delayed_chord, telemetry=chord_telemetry,
        sleep_policy=SleepPolicy(poll_s=0.002),
        health_monitor=chord_health,
        min_hold_us=50_000, spin_threshold_us=1_000,
    )
    chord_state = PlaybackState(start_perf=clock_chord.now_us())
    loop_chord.run(
        chord_state, MockCommandSource(), MockFocusSignal(),
        MockProgressSink(), total_time_us=60_000,
    )

    # Chord contract: ONE key_down call to the backend, with the full
    # 3-tuple scan-code payload.
    assert chord_backend.key_down.call_count == 1, (
        f"chord (fidelity mode) split into "
        f"{chord_backend.key_down.call_count} key_down calls; expected 1 "
        f"(plan §16.2: one KeyAction, one native SendInput call)."
    )
    sent_payload = chord_backend.key_down.call_args[0][0]
    assert len(sent_payload) == 3, (
        f"chord key_down received {len(sent_payload)} scan codes; expected 3."
    )

    # All 3 chord members share one completion timestamp (1 native call).
    chord_downs = [r for r in chord_telemetry.records if r.kind == "down"]
    assert len(chord_downs) == 1, (
        f"chord produced {len(chord_downs)} down telemetry records; expected "
        "1 (one contiguous batch, no per-member scheduling)."
    )
    chord_down = chord_downs[0]
    # Same timestamp-relationship invariants as the single-key case.
    assert chord_down.dispatch_completed_us == chord_state.get_elapsed_us(
        clock_chord, 500
    ), (
        f"chord dispatch_completed_us={chord_down.dispatch_completed_us}; "
        f"expected elapsed-of(send_completed_us=500)="
        f"{chord_state.get_elapsed_us(clock_chord, 500)} (Phase 2 contract)."
    )
    assert chord_down.dispatch_completed_us is not None
    assert chord_down.visible_lateness_us == (
        chord_down.dispatch_completed_us - chord_down.scheduled_us
    )
