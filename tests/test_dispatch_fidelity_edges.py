from __future__ import annotations

import threading
from collections.abc import Callable
from contextvars import ContextVar
from typing import cast
from unittest.mock import MagicMock, patch

import pytest

from sky_music.domain import ScanCode
from sky_music.domain.scheduler_types import ActionKind, KeyAction, Microseconds
from sky_music.infrastructure.backend import DryRunBackend, InputSendResult
from sky_music.infrastructure.timing import Clock, SleepPolicy
from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
from sky_music.orchestration.core.coordinator import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)
from sky_music.orchestration.core.loop import (
    DispatchHealthMonitor,
    DispatchLoop,
    PlaybackState,
)
from sky_music.orchestration.core.ports import PLAYBACK_QUIT
from sky_music.orchestration.playback_supervisor import (
    PlaybackSupervisor,
    SharedFocusSignal,
)
from sky_music.orchestration.telemetry import TelemetryLogger
from sky_music.platform.win32 import inputs


def _sequenced_clock(values: list[int]) -> tuple[Callable[[], int], list[int]]:
    samples: list[int] = []
    remaining = iter(values)

    def now_us() -> int:
        value = next(remaining)
        samples.append(value)
        return value

    return now_us, samples


def test_full_send_samples_once_immediately_after_native_return() -> None:
    operations: list[str] = []

    def send_input(*_args: object) -> int:
        operations.append("native")
        return 2

    def now_us() -> int:
        operations.append("clock")
        return 101

    with patch.object(inputs.user32, "SendInput", side_effect=send_input):
        result = inputs.send_scan_code_batch_trusted(
            (0x1E, 0x1F),
            clock_now=now_us,
        )

    assert operations == ["native", "clock"]
    assert result.requested == 2
    assert result.inserted == 2
    assert result.completed_us == 101
    assert result.win32_error == 0


@pytest.mark.parametrize(
    ("key_up", "native_results", "expected_inserted"),
    [
        (False, [0, 2], 2),
        (False, [1, 1], 2),
        (True, [1, 2], 3),
    ],
)
def test_partial_send_reports_final_native_attempt_timestamp(
    key_up: bool,
    native_results: list[int],
    expected_inserted: int,
) -> None:
    now_us, samples = _sequenced_clock([100, 200])

    with patch.object(
        inputs.user32,
        "SendInput",
        side_effect=native_results,
    ) as send_input:
        result = inputs.send_scan_code_batch_trusted(
            (0x1E, 0x1F, 0x20),
            key_up=key_up,
            clock_now=now_us,
        )

    assert send_input.call_count == 2
    assert samples == [100, 200]
    assert result.inserted == expected_inserted
    assert result.completed_us == 200


def test_final_failed_attempt_surfaces_win32_error_after_timestamp() -> None:
    operations: list[str] = []
    timestamps = iter((100, 200))

    def send_input(*_args: object) -> int:
        operations.append("native")
        return 0

    def now_us() -> int:
        operations.append("clock")
        return next(timestamps)

    def get_last_error() -> int:
        operations.append("error")
        return 1234

    with (
        patch.object(inputs.user32, "SendInput", side_effect=send_input),
        patch.object(inputs.ctypes, "get_last_error", side_effect=get_last_error),
    ):
        result = inputs.send_scan_code_batch_trusted(
            (0x1E,),
            clock_now=now_us,
        )

    assert operations == [
        "native",
        "clock",
        "error",
        "native",
        "clock",
        "error",
    ]
    assert result.inserted == 0
    assert result.completed_us == 200
    assert result.win32_error == 1234


class _ManualClock:
    def __init__(self, values: list[int]) -> None:
        self._values = iter(values)
        self.current_us = values[0]

    def now_us(self) -> int:
        self.current_us = next(self._values, self.current_us)
        return self.current_us


class _HighResolutionSleeper:
    is_high_resolution = True
    handle = 901

    def __init__(self, clock: _ManualClock) -> None:
        self.clock = clock
        self.sleeps: list[float] = []

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.clock.current_us += int(seconds * 1_000_000)


class _AdvancingWaitStrategy(HybridWaitStrategy):
    def spin_until_us(self, target_system_us: int, clock: Clock) -> None:
        cast(_ManualClock, clock).current_us = target_system_us


def test_simultaneous_command_and_timer_prioritizes_command_handle() -> None:
    clock = _ManualClock([0, 0])
    sleeper = _HighResolutionSleeper(clock)
    strategy = _AdvancingWaitStrategy(enable_event_wait=True)
    waited_handles: list[tuple[int, ...]] = []

    def wait_for_multiple(handles: tuple[int, ...], _timeout_ms: int) -> int:
        waited_handles.append(handles)
        # Windows returns the lowest-index signalled handle when both are ready.
        return inputs.WAIT_OBJECT_0

    with (
        patch.object(inputs, "set_waitable_timer_relative_us", return_value=True),
        patch.object(inputs, "wait_for_multiple_objects", side_effect=wait_for_multiple),
    ):
        interrupted = strategy.wait_until_us(
            target_system_us=100_000,
            clock=clock,
            sleeper=sleeper,
            spin_threshold_us=1_000,
            policy=SleepPolicy(),
            command_event=902,
        )

    assert waited_handles == [(902, 901)]
    assert interrupted is True
    assert clock.current_us < 100_000


def test_timer_only_result_uses_second_handle_and_reaches_deadline() -> None:
    clock = _ManualClock([0, 0])
    sleeper = _HighResolutionSleeper(clock)
    strategy = _AdvancingWaitStrategy(enable_event_wait=True)

    with (
        patch.object(inputs, "set_waitable_timer_relative_us", return_value=True),
        patch.object(
            inputs,
            "wait_for_multiple_objects",
            return_value=inputs.WAIT_OBJECT_0 + 1,
        ),
    ):
        interrupted = strategy.wait_until_us(
            target_system_us=100_000,
            clock=clock,
            sleeper=sleeper,
            spin_threshold_us=1_000,
            policy=SleepPolicy(),
            command_event=902,
        )

    assert interrupted is False
    assert clock.current_us == 100_000


def test_waitable_timer_recomputes_delay_adjacent_to_arm() -> None:
    clock = _ManualClock([0, 5_000])
    sleeper = _HighResolutionSleeper(clock)
    strategy = _AdvancingWaitStrategy(enable_event_wait=True)
    armed_delays: list[int] = []

    with (
        patch.object(
            inputs,
            "set_waitable_timer_relative_us",
            side_effect=lambda _handle, delay: armed_delays.append(delay) or True,
        ),
        patch.object(
            inputs,
            "wait_for_multiple_objects",
            return_value=inputs.WAIT_OBJECT_0 + 1,
        ),
    ):
        strategy.wait_until_us(
            target_system_us=100_000,
            clock=clock,
            sleeper=sleeper,
            spin_threshold_us=1_000,
            policy=SleepPolicy(),
            command_event=902,
        )

    assert armed_delays == [94_000]


def test_wait_failure_uses_bounded_sleep_instead_of_long_gap_spin() -> None:
    clock = _ManualClock([0, 0])
    sleeper = _HighResolutionSleeper(clock)
    strategy = _AdvancingWaitStrategy(enable_event_wait=True)

    with (
        patch.object(inputs, "set_waitable_timer_relative_us", return_value=True),
        patch.object(inputs, "wait_for_multiple_objects", return_value=None),
    ):
        interrupted = strategy.wait_until_us(
            target_system_us=5_000_000,
            clock=clock,
            sleeper=sleeper,
            spin_threshold_us=1_000,
            policy=SleepPolicy(),
            command_event=902,
        )

    assert interrupted is False
    assert sleeper.sleeps == [0.002]
    assert clock.current_us == 2_000


def test_short_remaining_interval_bypasses_timer_arm() -> None:
    clock = _ManualClock([99_500])
    sleeper = _HighResolutionSleeper(clock)
    strategy = _AdvancingWaitStrategy(enable_event_wait=True)

    with patch.object(inputs, "set_waitable_timer_relative_us") as arm_timer:
        interrupted = strategy.wait_until_us(
            target_system_us=100_000,
            clock=clock,
            sleeper=sleeper,
            spin_threshold_us=1_000,
            policy=SleepPolicy(),
            command_event=902,
        )

    assert interrupted is False
    arm_timer.assert_not_called()
    assert clock.current_us == 100_000


class _AlwaysFocused:
    def is_active(self) -> bool:
        return True

    def focus(self) -> bool:
        return True


class _QuitCommandSource:
    def poll(self) -> str:
        return "quit"


class _NullProgressSink:
    def publish(self, **_kwargs: object) -> None:
        return None

    def finish(self, message: str) -> None:
        return None


class _RecordingProgressSink(_NullProgressSink):
    def __init__(self) -> None:
        self.snapshots: list[dict[str, object]] = []

    def publish(self, **kwargs: object) -> None:
        self.snapshots.append(kwargs)

class _RecordingBackend(DryRunBackend):
    def __init__(self) -> None:
        super().__init__()
        self.down_calls = 0

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        self.down_calls += 1
        return super().key_down(scan_codes)


def test_simultaneous_ready_command_is_polled_before_note_on() -> None:
    clock = _ManualClock([0])
    sleeper = _HighResolutionSleeper(clock)
    wait_strategy = _AdvancingWaitStrategy(enable_event_wait=True)
    actions = (
        KeyAction(ActionKind.DOWN, (ScanCode(0x1E),), Microseconds(100_000)),
        KeyAction(ActionKind.UP, (ScanCode(0x1E),), Microseconds(110_000)),
    )
    coordinator = RuntimeDispatchCoordinator(
        compile_runtime_intents(actions),
        min_hold_us=10_000,
    )
    backend = _RecordingBackend()
    focus_signal = _AlwaysFocused()
    progress_sink = _RecordingProgressSink()
    health = DispatchHealthMonitor(
        backend,
        clock,
        focus_signal,
        require_focus=False,
    )
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=sleeper,
        wait_strategy=wait_strategy,
        backend=backend,
        telemetry=TelemetryLogger("simultaneous-command"),
        sleep_policy=SleepPolicy(),
        health_monitor=health,
        min_hold_us=10_000,
        spin_threshold_us=1_000,
    )

    with (
        patch.object(inputs, "set_waitable_timer_relative_us", return_value=True),
        patch.object(
            inputs,
            "wait_for_multiple_objects",
            return_value=inputs.WAIT_OBJECT_0,
        ),
    ):
        result = loop.run(
            state=PlaybackState(start_perf=clock.now_us()),
            command_source=_QuitCommandSource(),
            focus_signal=focus_signal,
            progress_sink=progress_sink,
            total_time_us=110_000,
            command_event=902,
        )

    assert result == PLAYBACK_QUIT
    assert backend.down_calls == 0
    assert progress_sink.snapshots[-1]["status"] == "stopped"
    assert progress_sink.snapshots[-1]["force"] is True
    assert progress_sink.snapshots[-1]["counters"] is not None


def test_terminal_progress_is_published_on_dispatch_error() -> None:
    clock = _ManualClock([0])
    sleeper = _HighResolutionSleeper(clock)
    actions = (
        KeyAction(ActionKind.DOWN, (ScanCode(0x1E),), Microseconds(100_000)),
        KeyAction(ActionKind.UP, (ScanCode(0x1E),), Microseconds(110_000)),
    )
    coordinator = RuntimeDispatchCoordinator(
        compile_runtime_intents(actions),
        min_hold_us=10_000,
    )
    backend = _RecordingBackend()
    focus_signal = _AlwaysFocused()
    health = DispatchHealthMonitor(
        backend,
        clock,
        focus_signal,
        require_focus=False,
    )
    loop = DispatchLoop(
        coordinator=coordinator,
        clock=clock,
        sleeper=sleeper,
        wait_strategy=_AdvancingWaitStrategy(enable_event_wait=False),
        backend=backend,
        telemetry=TelemetryLogger("terminal-error"),
        sleep_policy=SleepPolicy(),
        health_monitor=health,
        min_hold_us=10_000,
        spin_threshold_us=1_000,
    )
    progress_sink = _RecordingProgressSink()

    with patch.object(
        loop,
        "_wait_until_runtime_deadline",
        side_effect=RuntimeError("dispatch failure"),
    ), pytest.raises(RuntimeError, match="dispatch failure"):
        loop.run(
            state=PlaybackState(start_perf=clock.now_us()),
            command_source=_QuitCommandSource(),
            focus_signal=focus_signal,
            progress_sink=progress_sink,
            total_time_us=110_000,
        )

    assert progress_sink.snapshots[-1]["status"] == "error"
    assert progress_sink.snapshots[-1]["force"] is True
    assert progress_sink.snapshots[-1]["counters"] is not None


def test_shared_focus_signal_uses_event_and_survives_concurrent_reads() -> None:
    signal = SharedFocusSignal(False)
    assert isinstance(signal._event, threading.Event)
    start = threading.Barrier(3)
    observed: list[bool] = []

    def writer() -> None:
        start.wait()
        for index in range(10_000):
            signal.set_active(index % 2 == 0)
        signal.set_active(True)

    def reader() -> None:
        start.wait()
        observed.extend(signal.is_active() for _ in range(10_000))

    writer_thread = threading.Thread(target=writer, context=None)
    reader_thread = threading.Thread(target=reader, context=None)
    writer_thread.start()
    reader_thread.start()
    start.wait()
    writer_thread.join()
    reader_thread.join()

    assert observed
    assert all(type(value) is bool for value in observed)
    assert signal.is_active() is True


def test_dispatch_thread_starts_with_empty_context() -> None:
    inherited = ContextVar[str]("dispatch-context-test")
    token = inherited.set("caller-value")
    seen: list[str | None] = []
    clock = _ManualClock([0])
    sleeper = _HighResolutionSleeper(clock)
    telemetry = MagicMock()
    telemetry.runtime_options = {}
    dispatch_loop_mock = MagicMock()
    dispatch_loop = cast(DispatchLoop, dispatch_loop_mock)
    dispatch_loop.sleeper = sleeper
    dispatch_loop_mock.run.side_effect = (
        lambda **_kwargs: seen.append(inherited.get(None)) or PLAYBACK_QUIT
    )
    supervisor = PlaybackSupervisor(
        controls=None,
        focus_guard=MagicMock(),
        require_focus=False,
        renderer=None,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(),
        clock=clock,
        sleeper=sleeper,
        song_name="empty-context",
        rt_priority_mode="off",
        enable_timer_guard=False,
        enable_event_wait=False,
        enable_epoch_rebase=False,
    )

    class _PriorityOff:
        outcome = None

        def __init__(self, _mode: str) -> None:
            return None

        def __enter__(self) -> _PriorityOff:
            return self

        def __exit__(self, *_args: object) -> None:
            return None

    try:
        with patch(
            "sky_music.infrastructure.rt_priority.DispatchThreadPriorityScope",
            _PriorityOff,
        ):
            result = supervisor.run(
                dispatch_loop=dispatch_loop,
                coordinator=MagicMock(),
                state=PlaybackState(start_perf=clock.now_us()),
                total_time_us=0,
                use_dispatch_thread=True,
            )
    finally:
        inherited.reset(token)

    assert result == PLAYBACK_QUIT
    assert seen == [None]
