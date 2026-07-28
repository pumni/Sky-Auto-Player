"""Phase H: Mid-song spin re-probe -- unit tests (H.4)."""
from __future__ import annotations

import pytest

from sky_music.orchestration.core.loop import (
    REPROBE_MIN_GAP_US,
    REPROBE_MIN_INTERVAL_US,
    DispatchLoop,
)
from sky_music.orchestration.core.ports import PlaybackCommand
from sky_music.orchestration.core.state import PlaybackState


class _FakeClock:
    def __init__(self, step_us: int = 1_000) -> None:
        self._us = 0
        self._step = step_us

    def now_us(self) -> int:
        v = self._us
        self._us += self._step
        return v


class _FakeSleeper:
    def __init__(self, clock: _FakeClock, wake_error_us: int = 500) -> None:
        self._clock = clock
        self._wake_error_us = wake_error_us
        self.sleep_calls: int = 0

    def sleep(self, seconds: float) -> None:
        self.sleep_calls += 1
        self._clock._us += int(seconds * 1_000_000) + self._wake_error_us


def _make_loop_for_reprobe(wake_error_us: int = 500):
    from sky_music.infrastructure.backend import DryRunBackend
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
    from sky_music.orchestration.core.coordinator import RuntimeDispatchCoordinator
    from sky_music.orchestration.core.loop import DispatchHealthMonitor
    from sky_music.orchestration.telemetry import TelemetryLogger

    class NoopFocusController:
        def is_active(self): return True
        def focus(self): return True

    clock = _FakeClock()
    sleeper = _FakeSleeper(clock, wake_error_us)
    backend = DryRunBackend()
    telemetry = TelemetryLogger(song_name="reprobe-test", enabled=False)

    from sky_music.orchestration.core.coordinator import RuntimeSchedule
    sched = RuntimeSchedule(batches=(), generation_count=0)

    coordinator = RuntimeDispatchCoordinator(sched, min_hold_us=0)
    health = DispatchHealthMonitor(
        backend=backend,
        clock=clock,
        focus_guard=NoopFocusController(),
        require_focus=False,
    )
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=sleeper,
        wait_strategy=HybridWaitStrategy(enable_event_wait=False),
        backend=backend,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(),
        health_monitor=health,
        min_hold_us=0,
        spin_threshold_us=700,
    )
    loop.enable_spin_reprobe = True
    loop._spin_floor_us = 700
    return loop, sleeper


def test_reprobe_runs_and_records_telemetry() -> None:
    """Large gap triggers reprobe, REPROBE_SAMPLES sleep calls are made."""
    loop, sleeper = _make_loop_for_reprobe(wake_error_us=800)
    elapsed_us = REPROBE_MIN_GAP_US + 1
    initial_calls = sleeper.sleep_calls
    loop._run_mid_song_reprobe(elapsed_us)
    assert sleeper.sleep_calls == initial_calls + 8
    assert loop._last_reprobe_elapsed_us == elapsed_us
    assert len(loop._reprobe_applied_thresholds) == 1


def test_reprobe_second_within_interval_guard_prevents() -> None:
    """Second reprobe within REPROBE_MIN_INTERVAL_US is blocked by the guard."""
    loop, sleeper = _make_loop_for_reprobe()
    first_elapsed = REPROBE_MIN_GAP_US + 1
    loop._run_mid_song_reprobe(first_elapsed)
    calls_after_first = sleeper.sleep_calls
    second_elapsed = first_elapsed + REPROBE_MIN_INTERVAL_US - 1
    interval_elapsed = second_elapsed - loop._last_reprobe_elapsed_us
    should_reprobe = interval_elapsed >= REPROBE_MIN_INTERVAL_US
    assert not should_reprobe
    assert sleeper.sleep_calls == calls_after_first


def test_reprobe_allowed_after_interval() -> None:
    """After REPROBE_MIN_INTERVAL_US has elapsed, a second reprobe is allowed."""
    loop, sleeper = _make_loop_for_reprobe()
    loop._run_mid_song_reprobe(REPROBE_MIN_GAP_US + 1)
    calls_after_first = sleeper.sleep_calls
    second_elapsed = loop._last_reprobe_elapsed_us + REPROBE_MIN_INTERVAL_US
    should_reprobe = second_elapsed - loop._last_reprobe_elapsed_us >= REPROBE_MIN_INTERVAL_US
    assert should_reprobe
    loop._run_mid_song_reprobe(second_elapsed)
    assert sleeper.sleep_calls == calls_after_first + 8
    assert len(loop._reprobe_applied_thresholds) == 2


def test_reprobe_kill_switch_false() -> None:
    """enable_spin_reprobe=False means the guard never fires."""
    loop, sleeper = _make_loop_for_reprobe()
    loop.enable_spin_reprobe = False
    remaining_us = REPROBE_MIN_GAP_US + 1
    elapsed_us = REPROBE_MIN_GAP_US + 1
    guard = (
        loop.enable_spin_reprobe
        and remaining_us >= REPROBE_MIN_GAP_US
        and elapsed_us - loop._last_reprobe_elapsed_us >= REPROBE_MIN_INTERVAL_US
    )
    assert not guard
    assert sleeper.sleep_calls == 0


def test_reprobe_cooperative_attempt_takes_one_sample_per_iteration() -> None:
    loop, sleeper = _make_loop_for_reprobe(wake_error_us=800)
    loop._begin_mid_song_reprobe(REPROBE_MIN_GAP_US + 1)

    for sample_index in range(7):
        assert loop._advance_mid_song_reprobe(
            REPROBE_MIN_GAP_US + 1,
            REPROBE_MIN_GAP_US + 1,
        ) is False
        assert sleeper.sleep_calls == sample_index + 1
        assert loop._reprobe_active is True

    assert loop._advance_mid_song_reprobe(
        REPROBE_MIN_GAP_US + 1,
        REPROBE_MIN_GAP_US + 1,
    ) is True
    assert sleeper.sleep_calls == 8
    assert loop._reprobe_active is False
    assert len(loop._reprobe_applied_thresholds) == 1


class _NoopProgressSink:
    def publish(self, **_kwargs) -> None:
        return None

    def finish(self, message: str) -> None:
        return None


class _QueuedCommandSource:
    def __init__(self, command: str | None, trigger_sample: int, *, follow_up: str | None = None) -> None:
        self.command = command
        self.trigger_sample = trigger_sample
        self.follow_up = follow_up
        self.poll_count = 0
        self.seen: list[str] = []

    def poll(self) -> str | None:
        poll_index = self.poll_count
        self.poll_count += 1
        if poll_index < self.trigger_sample:
            return None
        if poll_index == self.trigger_sample:
            if self.command is not None:
                self.seen.append(self.command)
            return self.command
        if self.follow_up is not None and poll_index == self.trigger_sample + 1:
            self.seen.append(self.follow_up)
            return self.follow_up
        return None


class _MutableFocus:
    def __init__(self) -> None:
        self.active = True

    def is_active(self) -> bool:
        return self.active


def _run_full_path_reprobe(
    *,
    trigger_sample: int,
    command: str | None,
    command_event: int | None,
    follow_up: str | None = None,
    focus_loss_sample: int | None = None,
) -> tuple[DispatchLoop, _QueuedCommandSource, _FakeSleeper, str | None]:
    loop, sleeper = _make_loop_for_reprobe(wake_error_us=0)
    # Make the reprobe eligibility guard true at elapsed=0 so this helper
    # reaches the production cooperative state machine rather than the normal
    # deadline wait path.
    loop._last_reprobe_elapsed_us = -REPROBE_MIN_INTERVAL_US
    abort_calls: list[str] = []
    if command == PlaybackCommand.PANIC:
        loop._abort_input_safe = lambda reason, **_kwargs: abort_calls.append(reason)  # type: ignore[method-assign]
        loop._test_abort_calls = abort_calls  # type: ignore[attr-defined]
    state = PlaybackState(start_perf=0)
    source = _QueuedCommandSource(command, trigger_sample, follow_up=follow_up)
    focus = _MutableFocus()
    if focus_loss_sample is not None:
        original_poll = source.poll

        def poll_with_focus_loss() -> str | None:
            result = original_poll()
            if source.poll_count == focus_loss_sample + 1:
                focus.active = False
            return result

        source.poll = poll_with_focus_loss  # type: ignore[method-assign]
        loop.health_monitor.require_focus = True

    # Leave a generous deterministic budget for the fake clock reads around
    # each sample; the command still has to be observed before sample N+1.
    target_elapsed_us = REPROBE_MIN_GAP_US + (trigger_sample + 1) * 10_000 + 1
    result = loop._wait_until_runtime_deadline(
        target_elapsed_us=target_elapsed_us,
        state=state,
        last_runtime_poll_us=-1_000,
        last_render_time_us=0,
        first_action_executed=False,
        total_time_us=target_elapsed_us,
        command_source=source,
        focus_signal=focus,
        progress_sink=_NoopProgressSink(),
        command_event=command_event,
    )
    return loop, source, sleeper, result[0]


@pytest.mark.parametrize("command_event", [None, 1234])
@pytest.mark.parametrize("trigger_sample", range(8))
@pytest.mark.parametrize("command", [PlaybackCommand.QUIT, PlaybackCommand.SKIP])
def test_full_path_reprobe_quit_and_skip_are_polled_between_samples(
    command_event: int | None,
    trigger_sample: int,
    command: str,
) -> None:
    loop, source, sleeper, result = _run_full_path_reprobe(
        trigger_sample=trigger_sample,
        command=command,
        command_event=command_event,
    )

    expected = "quit" if command == PlaybackCommand.QUIT else "skipped"
    assert result == expected
    assert source.seen == [command]
    assert sleeper.sleep_calls == trigger_sample + 1
    assert loop._reprobe_active is False


@pytest.mark.parametrize("command_event", [None, 1234])
@pytest.mark.parametrize("trigger_sample", range(8))
def test_full_path_reprobe_pause_is_serviced_between_samples(
    command_event: int | None,
    trigger_sample: int,
) -> None:
    loop, source, sleeper, result = _run_full_path_reprobe(
        trigger_sample=trigger_sample,
        command=PlaybackCommand.PAUSE,
        command_event=command_event,
        follow_up=PlaybackCommand.QUIT,
    )

    assert result == "quit"
    assert source.seen == [PlaybackCommand.PAUSE, PlaybackCommand.QUIT]
    assert sleeper.sleep_calls >= trigger_sample + 1
    assert loop._reprobe_active is False


@pytest.mark.parametrize("command_event", [None, 1234])
@pytest.mark.parametrize("trigger_sample", range(8))
def test_full_path_reprobe_panic_is_polled_between_samples(
    command_event: int | None,
    trigger_sample: int,
) -> None:
    loop, source, sleeper, _result = _run_full_path_reprobe(
        trigger_sample=trigger_sample,
        command=PlaybackCommand.PANIC,
        command_event=command_event,
    )
    assert source.seen == [PlaybackCommand.PANIC]
    assert sleeper.sleep_calls >= trigger_sample + 1
    assert loop._reprobe_active is False
    assert loop._test_abort_calls == ["panic"]  # type: ignore[attr-defined]


@pytest.mark.parametrize("command_event", [None, 1234])
@pytest.mark.parametrize("trigger_sample", range(8))
def test_full_path_reprobe_focus_loss_discards_partial_attempt(
    command_event: int | None,
    trigger_sample: int,
) -> None:
    loop, source, sleeper, result = _run_full_path_reprobe(
        trigger_sample=trigger_sample,
        command=None,
        command_event=command_event,
        follow_up=PlaybackCommand.QUIT,
        focus_loss_sample=trigger_sample,
    )

    assert result == "quit"
    assert source.seen == [PlaybackCommand.QUIT]
    assert sleeper.sleep_calls >= trigger_sample + 1
    assert loop._reprobe_active is False


def test_full_path_reprobe_discards_when_deadline_loses_required_gap() -> None:
    loop, sleeper = _make_loop_for_reprobe(wake_error_us=0)
    state = PlaybackState(start_perf=0)
    loop._begin_mid_song_reprobe(0)

    result = loop._wait_until_runtime_deadline(
        target_elapsed_us=REPROBE_MIN_GAP_US - 1,
        state=state,
        last_runtime_poll_us=-1_000,
        last_render_time_us=0,
        first_action_executed=False,
        total_time_us=REPROBE_MIN_GAP_US,
        command_source=_QueuedCommandSource(PlaybackCommand.QUIT, 99),
        focus_signal=_MutableFocus(),
        progress_sink=_NoopProgressSink(),
        command_event=None,
    )

    assert result[0] is None
    assert loop._reprobe_active is False
    assert loop._reprobe_sample_count == 0
    assert sleeper.sleep_calls > 0  # normal deadline wait still advances the fake clock
