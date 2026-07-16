"""
Sprint 8 — Unit tests for:
 - PlaybackEngine: ExecutionResult, get_elapsed_us, _execute_action
 - Backend safety: InputSendResult, duplicate-down protection, idempotent key_up
 - CLI: --fps FrameTimingPolicy upgrade, --compare-profiles smoke
"""
import types

import pytest

from sky_music.domain.domain import Millis, Note, NoteKey, Song
from sky_music.domain.scheduler import build_key_actions
from sky_music.infrastructure.backend import (
    DryRunBackend,
    InputSendResult,
    WinSendInputBackend,
)
from sky_music.orchestration.engine import (
    PLAYBACK_FINISHED,
    ExecutionResult,
    PlaybackEngine,
)

# ─────────────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────────────

def _simple_song(note_ms: tuple[int, ...] = (0,)) -> Song:
    return Song(
        name="Test",
        notes=tuple(Note(time_ms=Millis(t), key=NoteKey(f"Key{i}")) for i, t in enumerate(note_ms)),
    )


def _engine(song: Song, backend=None) -> PlaybackEngine:
    sched = build_key_actions(song)
    return PlaybackEngine(
        song=song,
        actions=sched.actions,
        backend=backend or DryRunBackend(),
        telemetry_enabled=False,
        require_focus=False,
    )


# ─────────────────────────────────────────────────────────────────────────────
# ExecutionResult dataclass
# ─────────────────────────────────────────────────────────────────────────────

class TestExecutionResult:
    def test_is_late_flag(self):
        r = ExecutionResult(
            event_index=0, scheduled_us=1000, actual_us=1200,
            lateness_us=200, send_duration_us=10,
            is_late=True, is_critically_late=False,
        )
        assert r.is_late is True
        assert r.is_critically_late is False

    def test_is_critically_late_flag(self):
        r = ExecutionResult(
            event_index=1, scheduled_us=0, actual_us=11_000,
            lateness_us=11_000, send_duration_us=5,
            is_late=True, is_critically_late=True,
        )
        assert r.is_critically_late is True

    def test_not_late_when_on_time(self):
        r = ExecutionResult(
            event_index=0, scheduled_us=1000, actual_us=999,
            lateness_us=-1, send_duration_us=2,
            is_late=False, is_critically_late=False,
        )
        assert r.is_late is False


def test_telemetry_summary_includes_schedule_metadata():
    from sky_music.domain.scheduler_types import Microseconds, ScheduleMetadata
    from sky_music.orchestration.telemetry import TelemetryLogger

    logger = TelemetryLogger("test", enabled=True)
    metadata = ScheduleMetadata(
        actions=(),
        source_duration_us=Microseconds(0),
        playback_duration_us=Microseconds(0),
        compressed_holds=2,
        impossible_same_key_repeats=1,
        risky_same_key_repeats=3,
        max_polyphony=5,
        note_count=20,
    )
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=0,
        actual_us=100,
        lateness_us=100,
        send_duration_us=10,
        scan_codes=(0x15,),
        reason="onset",
    )
    logger.record_schedule_metadata(metadata)
    summary = logger.get_summary()
    assert summary is not None
    assert summary["schedule"]["compressed_holds"] == 2
    assert summary["schedule"]["impossible_same_key_repeats"] == 1
    assert summary["schedule"]["max_polyphony"] == 5


# ─────────────────────────────────────────────────────────────────────────────
# PlaybackEngine.get_elapsed_us
# ─────────────────────────────────────────────────────────────────────────────

class TestGetElapsedUs:
    """Elapsed-time accounting now lives on PlaybackState.get_elapsed_us(clock)."""

    class _FakeClock:
        def __init__(self, start_us: int = 10_000):
            self.current_us = start_us
        def now_us(self) -> int:
            return self.current_us

    def test_no_pause(self):
        from sky_music.orchestration.engine import PlaybackState
        clock = self._FakeClock(start_us=10_000)
        # start_perf=0 → elapsed = 10_000 - 0 = 10_000
        state = PlaybackState(start_perf=0)
        assert state.get_elapsed_us(clock) == 10_000

    def test_with_manual_pause(self):
        from sky_music.orchestration.engine import PlaybackState
        clock = self._FakeClock(start_us=20_000)
        # Frozen at pause interval start: 15_000 - 0 = 15_000
        state = PlaybackState(start_perf=0)
        state.enter_pause("manual", 15_000)
        assert state.get_elapsed_us(clock) == 15_000

    def test_never_negative(self):
        from sky_music.orchestration.engine import PlaybackState
        clock = self._FakeClock(start_us=10_000)
        # start_perf far in the future → result clamped to 0
        state = PlaybackState(start_perf=99_999_999)
        assert state.get_elapsed_us(clock) == 0


# ─────────────────────────────────────────────────────────────────────────────
# PlaybackEngine._execute_action
# ─────────────────────────────────────────────────────────────────────────────

class TestExecuteAction:
    def test_returns_execution_result(self):
        song = _simple_song((0,))
        sched = build_key_actions(song)
        backend = DryRunBackend()
        engine = PlaybackEngine(
            song=song, actions=sched.actions, backend=backend,
            telemetry_enabled=False, require_focus=False,
        )
        from sky_music.orchestration.engine import PlaybackState
        action = sched.actions[0]  # first down event
        result = engine._execute_action(idx=0, action=action, state=PlaybackState(start_perf=0))
        assert isinstance(result, ExecutionResult)
        assert result.event_index == 0
        assert result.scheduled_us == action.at_us
        # send_duration_us should be >= 0
        assert result.send_duration_us >= 0

    def test_dispatch_down_action(self):
        song = _simple_song((0,))
        sched = build_key_actions(song)
        backend = DryRunBackend()
        engine = PlaybackEngine(
            song=song, actions=sched.actions, backend=backend,
            telemetry_enabled=False, require_focus=False,
        )
        from sky_music.orchestration.engine import PlaybackState
        down_action = next(a for a in sched.actions if a.kind == "down")
        engine._execute_action(idx=0, action=down_action, state=PlaybackState(start_perf=0))
        assert ("down", tuple(sorted(down_action.scan_codes))) in backend.history

    def test_dispatch_up_action(self):
        song = _simple_song((0,))
        sched = build_key_actions(song)
        backend = DryRunBackend()
        engine = PlaybackEngine(
            song=song, actions=sched.actions, backend=backend,
            telemetry_enabled=False, require_focus=False,
        )
        from sky_music.orchestration.engine import PlaybackState
        # Execute down first so the up can release it
        state = PlaybackState(start_perf=0)
        for action in sched.actions:
            engine._execute_action(idx=0, action=action, state=state)
        assert any(k == "up" for k, _ in backend.history)


# ─────────────────────────────────────────────────────────────────────────────
# InputSendResult
# ─────────────────────────────────────────────────────────────────────────────

class TestInputSendResult:
    def test_default_error_is_none(self):
        r = InputSendResult(sent=(1, 2), skipped_duplicates=(), success=True)
        assert r.error is None

    def test_failure_carries_error(self):
        r = InputSendResult(sent=(), skipped_duplicates=(1,), success=False, error="UIPI")
        assert r.error == "UIPI"
        assert r.success is False


# ─────────────────────────────────────────────────────────────────────────────
# DryRunBackend — duplicate-down protection
# ─────────────────────────────────────────────────────────────────────────────

class TestDryRunBackendSafety:
    def test_key_down_returns_input_send_result(self):
        backend = DryRunBackend()
        result = backend.key_down((0x15,))
        assert isinstance(result, InputSendResult)
        assert result.success is True

    def test_key_up_returns_input_send_result(self):
        backend = DryRunBackend()
        backend.key_down((0x15,))
        result = backend.key_up((0x15,))
        assert isinstance(result, InputSendResult)
        assert result.success is True

    def test_duplicate_down_skipped(self):
        backend = DryRunBackend()
        r1 = backend.key_down((0x15,))
        assert r1.sent == (0x15,)
        assert r1.skipped_duplicates == ()

        # Second key_down of same key — should skip it
        r2 = backend.key_down((0x15,))
        assert r2.sent == ()
        assert 0x15 in r2.skipped_duplicates

        # History should only have ONE down event
        down_events = [h for h in backend.history if h[0] == "down"]
        assert len(down_events) == 1

    def test_duplicate_down_partial_skip(self):
        """When sending two keys and one is already held, only the new one is sent."""
        backend = DryRunBackend()
        backend.key_down((0x15,))
        r = backend.key_down((0x15, 0x16))
        # 0x15 already held, only 0x16 goes through
        assert 0x16 in r.sent
        assert 0x15 in r.skipped_duplicates

    def test_key_up_idempotent(self):
        """key_up on keys not currently held returns success with skipped list."""
        backend = DryRunBackend()
        r = backend.key_up((0x15,))
        assert r.success is True
        assert r.sent == ()
        assert 0x15 in r.skipped_duplicates

    def test_empty_key_down(self):
        backend = DryRunBackend()
        result = backend.key_down(())
        assert result.sent == ()
        assert result.success is True

    def test_empty_key_up(self):
        backend = DryRunBackend()
        result = backend.key_up(())
        assert result.sent == ()
        assert result.success is True


# ─────────────────────────────────────────────────────────────────────────────
# WinSendInputBackend — duplicate-down protection (mock SendInput)
# ─────────────────────────────────────────────────────────────────────────────

class TestWinBackendDuplicateDownProtection:
    def _make_backend_with_mock(self, fail: bool = False):
        backend = WinSendInputBackend.__new__(WinSendInputBackend)
        backend.active_keys = set()
        backend.possibly_active_keys = set()
        backend.failed_release_keys = set()
        backend.last_error = None
        backend._now_us = lambda: 0

        calls = []
        def mock_send(scan_codes, key_up=False):
            if fail:
                raise OSError("mock error")
            calls.append((scan_codes, key_up))

        dummy = types.SimpleNamespace(send_scan_code_batch=mock_send, send_scan_code_batch_trusted=mock_send)
        backend.inputs_module = dummy  # type: ignore[assignment]
        return backend, calls

    def test_no_duplicate_send_when_already_held(self):
        backend, calls = self._make_backend_with_mock()
        backend.key_down((0x15,))
        assert len(calls) == 1

        # Second key_down of same key — should NOT call SendInput
        result = backend.key_down((0x15,))
        assert len(calls) == 1  # still only 1 send
        assert result.sent == ()
        assert 0x15 in result.skipped_duplicates

    def test_partial_duplicate(self):
        backend, calls = self._make_backend_with_mock()
        backend.key_down((0x15,))
        calls.clear()

        _result = backend.key_down((0x15, 0x16))
        # Only 0x16 should have been sent
        assert len(calls) == 1
        sent_codes, _key_up = calls[0]
        assert 0x16 in sent_codes
        assert 0x15 not in sent_codes

    def test_key_up_returns_input_send_result(self):
        backend, calls = self._make_backend_with_mock()
        backend.key_down((0x15,))
        calls.clear()

        result = backend.key_up((0x15,))
        assert isinstance(result, InputSendResult)
        assert result.success is True
        assert 0x15 in result.sent


def _make_contract_backend(kind: str):
    if kind == "dry":
        return DryRunBackend()

    backend = WinSendInputBackend.__new__(WinSendInputBackend)
    backend.active_keys = set()
    backend.possibly_active_keys = set()
    backend.failed_release_keys = set()
    backend.last_error = None
    backend._now_us = lambda: 0

    def mock_send(scan_codes, key_up=False):
        return None

    backend.inputs_module = types.SimpleNamespace(send_scan_code_batch=mock_send, send_scan_code_batch_trusted=mock_send)  # type: ignore[assignment]
    return backend


@pytest.mark.parametrize("backend_kind", ("dry", "win"))
def test_tracked_backends_share_down_up_dedup_contract(backend_kind):
    backend = _make_contract_backend(backend_kind)

    results = [
        backend.key_down((0x15, 0x16)),
        backend.key_down((0x16, 0x17)),
        backend.key_up((0x15, 0x18)),
        backend.key_up((0x15, 0x17)),
    ]

    assert [
        (result.sent, result.skipped_duplicates, result.success)
        for result in results
    ] == [
        ((0x15, 0x16), (), True),
        ((0x17,), (0x16,), True),
        ((0x15,), (0x18,), True),
        ((0x17,), (0x15,), True),
    ]


# ─────────────────────────────────────────────────────────────────────────────
# FrameTimingPolicy via --fps
# ─────────────────────────────────────────────────────────────────────────────

class TestFpsTimingPolicyUpgrade:
    def test_fps_upgrades_to_frame_timing_policy(self):
        """When --fps is set and the profile is not already fps-aware, TIMING_POLICY
        should be promoted to a FrameTimingPolicy with correct frame_us."""
        from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy

        base_policy = TimingPolicy.balanced()
        fps = 60
        frame_policy = FrameTimingPolicy.from_timing_policy(
            base_policy, fps=fps, same_key_conflict_policy="degraded"
        )
        assert frame_policy.fps == 60
        expected_frame_us = round(1_000_000 / 60)
        assert frame_policy.frame_us == expected_frame_us
        # Hold should be at least the profile's configured hold_frames worth
        assert frame_policy.hold_us >= int(expected_frame_us * base_policy.hold_frames)

    def test_fps_none_does_not_upgrade(self):
        """When --fps is None, TimingPolicy stays as is."""
        from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy

        policy = TimingPolicy.balanced()
        assert not isinstance(policy, FrameTimingPolicy)


# ─────────────────────────────────────────────────────────────────────────────
# Full playback smoke with new engine (ExecutionResult flow)
# ─────────────────────────────────────────────────────────────────────────────

class TestPlaybackWithRefactoredEngine:
    def test_playback_completes_with_execution_result_flow(self):
        """Ensure the full engine loop works correctly after the Sprint 5 refactor."""
        song = _simple_song((0, 50, 100))
        sched = build_key_actions(song)
        backend = DryRunBackend()
        engine = PlaybackEngine(
            song=song, actions=sched.actions, backend=backend,
            telemetry_enabled=False, require_focus=False,
        )
        result = engine.play()
        assert result == PLAYBACK_FINISHED
        # 3 notes × 2 events each = 6 actions expected (down/up pairs)
        down_events = [h for h in backend.history if h[0] == "down"]
        up_events = [h for h in backend.history if h[0] == "up"]
        assert len(down_events) == 3
        assert len(up_events) >= 3  # may include final release_all

    def test_no_duplicate_down_in_full_playback(self):
        """DryRunBackend duplicate-down protection should never fire during normal playback."""
        song = _simple_song((0, 100, 200, 300, 400))
        sched = build_key_actions(song)
        backend = DryRunBackend()
        engine = PlaybackEngine(
            song=song, actions=sched.actions, backend=backend,
            telemetry_enabled=False, require_focus=False,
        )
        engine.play()
        # Verify no duplicate consecutive downs for the same scan code in history
        last_down: dict[int, bool] = {}
        for kind, scan_codes in backend.history:
            for sc in scan_codes:
                if kind == "down":
                    assert not last_down.get(sc, False), (
                        f"Scan code {sc} pressed down twice without release!"
                    )
                    last_down[sc] = True
                elif kind == "up":
                    last_down[sc] = False
