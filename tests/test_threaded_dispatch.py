from __future__ import annotations

import itertools
import threading
import time
from dataclasses import dataclass

from sky_music.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import (
    BackendHealth,
    InputSendResult,
    ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.dispatch_loop import PlaybackState
from sky_music.orchestration.engine import PLAYBACK_QUIT, PlaybackEngine
from sky_music.orchestration.playback_supervisor import PlaybackSupervisor
from sky_music.orchestration.telemetry import TelemetryLogger
from sky_music.platform.win32 import inputs
from sky_music.ui.hud import ProgressRenderer


@dataclass(frozen=True, slots=True)
class BackendCall:
    kind: str
    scan_codes: tuple[int, ...]
    thread_id: int
    perf_ns: int


class ThreadRecordingBackend:
    def __init__(self) -> None:
        self.active: set[int] = set()
        self.calls: list[BackendCall] = []
        self._lock = threading.Lock()

    def _record(self, kind: str, scan_codes: tuple[int, ...]) -> None:
        with self._lock:
            self.calls.append(
                BackendCall(
                    kind=kind,
                    scan_codes=scan_codes,
                    thread_id=threading.get_ident(),
                    perf_ns=time.perf_counter_ns(),
                )
            )

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(scan_code for scan_code in scan_codes if scan_code not in self.active)
        skipped = tuple(scan_code for scan_code in scan_codes if scan_code in self.active)
        if sent:
            self.active.update(sent)
            self._record("down", sent)
        return InputSendResult(sent=sent, skipped_duplicates=skipped, success=True)

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(scan_code for scan_code in scan_codes if scan_code in self.active)
        skipped = tuple(scan_code for scan_code in scan_codes if scan_code not in self.active)
        if sent:
            self.active.difference_update(sent)
            self._record("up", sent)
        return InputSendResult(sent=sent, skipped_duplicates=skipped, success=True)

    def release_all(self) -> ReleaseAllOutcome:
        attempted = tuple(sorted(self.active))
        self.active.clear()
        self._record("release_all", attempted)
        return ReleaseAllOutcome(
            attempted=attempted,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )

    def release_all_full_instrument(self) -> ReleaseAllOutcome:
        return self.release_all()

    def set_clock(self, clock: object) -> None:
        return None

    def get_health(self) -> BackendHealth:
        self._record("get_health", ())
        return BackendHealth(
            active_count=len(self.active),
            possibly_active_count=0,
            failed_release_count=0,
            last_error=None,
        )

    def get_send_diagnostics(self) -> dict[str, int]:
        return {}


class BlockingRenderer:
    def __init__(self, block_s: float) -> None:
        self.block_s = block_s
        self.events: list[tuple[str, str]] = []

    def render(
        self,
        elapsed: float,
        total: float,
        song_name: str,
        *,
        status: str = "playing",
        force: bool = False,
        input_path_degraded: bool = False,
        backend_health: BackendHealth | None = None,
    ) -> None:
        self.events.append(("render", status))
        time.sleep(self.block_s)

    def finish(self, message: str) -> None:
        self.events.append(("finish", message))

    def update_counters(self, lateness_us: int, **kwargs: object) -> None:
        self.events.append(("counter", str(lateness_us)))


class CpuBoundRenderer:
    def __init__(self, block_s: float) -> None:
        self.block_s = block_s
        self.events: list[tuple[str, str]] = []

    def render(
        self,
        elapsed: float,
        total: float,
        song_name: str,
        *,
        status: str = "playing",
        force: bool = False,
        input_path_degraded: bool = False,
        backend_health: BackendHealth | None = None,
    ) -> None:
        self.events.append(("render", status))
        deadline_s = time.perf_counter() + self.block_s
        while time.perf_counter() < deadline_s:
            pass

    def finish(self, message: str) -> None:
        self.events.append(("finish", message))

    def update_counters(self, lateness_us: int, **kwargs: object) -> None:
        self.events.append(("counter", str(lateness_us)))


class BlockingFocusGuard:
    def __init__(self, block_s: float) -> None:
        self.block_s = block_s
        self.focus_calls = 0

    def is_active(self) -> bool:
        time.sleep(self.block_s)
        return True

    def focus(self) -> bool:
        self.focus_calls += 1
        return True


class CpuBoundFocusGuard:
    def __init__(self, block_s: float) -> None:
        self.block_s = block_s

    def is_active(self) -> bool:
        deadline_s = time.perf_counter() + self.block_s
        while time.perf_counter() < deadline_s:
            pass
        return True

    def focus(self) -> bool:
        return True


class TimedControls:
    def __init__(self, commands: tuple[tuple[float, str], ...]) -> None:
        self.commands = list(commands)
        self.started_s: float | None = None
        self.enabled = True

    def poll(self) -> str | None:
        if self.started_s is None:
            self.started_s = time.perf_counter()
        if not self.commands:
            return None
        at_s, command = self.commands[0]
        if time.perf_counter() - self.started_s < at_s:
            return None
        self.commands.pop(0)
        return command


class TimedFocusGuard:
    def __init__(self, inactive_from_s: float, inactive_to_s: float) -> None:
        self.inactive_from_s = inactive_from_s
        self.inactive_to_s = inactive_to_s
        self.started_s: float | None = None

    def is_active(self) -> bool:
        if self.started_s is None:
            self.started_s = time.perf_counter()
        elapsed_s = time.perf_counter() - self.started_s
        return not (self.inactive_from_s <= elapsed_s < self.inactive_to_s)

    def focus(self) -> bool:
        return True


class StepClock:
    def __init__(self, values: tuple[int, ...]) -> None:
        self.values = list(values)
        self.last_value = values[-1]

    def now_us(self) -> int:
        if self.values:
            self.last_value = self.values.pop(0)
        return self.last_value


class NoopSleeper:
    def sleep(self, seconds: float) -> None:
        return


class StubDispatchLoop:
    def __init__(self) -> None:
        self.sleeper = NoopSleeper()
        self.health_monitor = None
        self.start_perf_at_run: int | None = None

    def run(
        self,
        *,
        state: PlaybackState,
        command_source: object,
        focus_signal: object,
        progress_sink: object,
        total_time_us: int,
        command_event: int | None,
    ) -> str:
        self.start_perf_at_run = state.start_perf
        return "finished"


class CpuBoundControls:
    def __init__(self, block_s: float) -> None:
        self.block_s = block_s
        self.enabled = True

    def poll(self) -> str | None:
        deadline_s = time.perf_counter() + self.block_s
        while time.perf_counter() < deadline_s:
            pass
        return None


def action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(scan_code) for scan_code in scan_codes),
        at_us=Microseconds(at_us),
        reason="threaded-test",
    )


def assert_down_intervals_near(
    backend: ThreadRecordingBackend,
    *,
    expected_count: int,
    interval_us: int,
    tolerance_us: int,
) -> None:
    down_calls = [call for call in backend.calls if call.kind == "down"]
    assert len(down_calls) == expected_count
    intervals_us = [
        (right.perf_ns - left.perf_ns) / 1_000
        for left, right in itertools.pairwise(down_calls)
    ]
    assert max(abs(actual_us - interval_us) for actual_us in intervals_us) < tolerance_us


def test_playback_state_rebase_epoch_preserves_pause_offset() -> None:
    state = PlaybackState(start_perf=1_000, pause_time_us=250)

    delta_us = state.rebase_epoch(2_500)

    assert delta_us == 1_500
    assert state.start_perf == 2_500
    assert state.epoch_us == 2_750


def test_threaded_epoch_rebase_defaults_off_in_engine_runtime_options() -> None:
    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-rebase-default-off", notes=()),
        actions=(action(0, "down", 21),),
        backend=backend,
        require_focus=False,
        telemetry_enabled=True,
        use_dispatch_thread=True,
    )

    engine.play()

    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_options"]["epoch_rebase"] is False
    assert "epoch_rebase_us" not in summary["runtime_options"]


def test_threaded_epoch_rebase_records_measured_delta_when_enabled() -> None:
    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-rebase-enabled", notes=()),
        actions=(action(0, "down", 21),),
        backend=backend,
        require_focus=False,
        telemetry_enabled=True,
        use_dispatch_thread=True,
        enable_epoch_rebase=True,
    )

    engine.play()

    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_options"]["epoch_rebase"] is True
    assert isinstance(summary["runtime_options"]["epoch_rebase_us"], int)
    assert summary["runtime_options"]["epoch_rebase_us"] >= 0


def test_threaded_supervisor_rebases_epoch_as_last_pre_run_step() -> None:
    telemetry = TelemetryLogger("threaded-rebase", enabled=True)
    clock = StepClock((2_500,))
    supervisor = PlaybackSupervisor(
        controls=None,
        focus_guard=BlockingFocusGuard(block_s=0.0),
        require_focus=False,
        renderer=None,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(),
        clock=clock,
        sleeper=NoopSleeper(),
        song_name="threaded-rebase",
        rt_priority_mode="off",
        enable_timer_guard=False,
        enable_event_wait=False,
        enable_epoch_rebase=True,
    )
    dispatch_loop = StubDispatchLoop()
    state = PlaybackState(start_perf=1_000)

    result = supervisor.run(
        dispatch_loop=dispatch_loop,  # type: ignore[arg-type]
        coordinator=None,  # type: ignore[arg-type]
        state=state,
        total_time_us=0,
        use_dispatch_thread=True,
    )

    assert result == "finished"
    assert dispatch_loop.start_perf_at_run == 2_500
    assert telemetry.runtime_options["epoch_rebase_us"] == 1_500


def test_threaded_dispatch_isolates_onsets_from_slow_ui_and_focus(monkeypatch) -> None:
    # Sky is foreground (focus_guard is active); model the platform foreground state the
    # threaded pre-down gate now consults via ``is_foreground_cached_hwnd`` (§2.1). Without
    # this the mock DLL leaves the ``sky`` HWND global None → the probe blocks every down.
    monkeypatch.setattr(inputs, "is_foreground_cached_hwnd", lambda: True)
    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-ui-isolation", notes=()),
        actions=(
            action(0, "down", 21),
            action(20_000, "down", 22),
            action(40_000, "down", 23),
        ),
        backend=backend,
        renderer=BlockingRenderer(block_s=0.05),
        focus_guard=BlockingFocusGuard(block_s=0.02),
        require_focus=True,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
    )

    engine.play()

    assert_down_intervals_near(
        backend,
        expected_count=3,
        interval_us=20_000,
        tolerance_us=8_000,
    )


def test_threaded_dispatch_keeps_all_backend_calls_on_dispatch_thread() -> None:
    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-owner", notes=()),
        actions=(action(0, "down", 21), action(100_000, "up", 21)),
        backend=backend,
        controls=TimedControls(((0.01, "pause"), (0.02, "quit"))),
        require_focus=False,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
    )

    assert engine.play() == PLAYBACK_QUIT

    assert backend.calls
    assert {call.thread_id for call in backend.calls} == {backend.calls[0].thread_id}
    assert any(call.kind == "release_all" for call in backend.calls)


def test_threaded_dispatch_refocus_publishes_and_wakes_control_path() -> None:
    backend = ThreadRecordingBackend()
    renderer = BlockingRenderer(block_s=0.0)
    focus_guard = BlockingFocusGuard(block_s=0.0)
    engine = PlaybackEngine(
        song=Song(name="threaded-refocus", notes=()),
        actions=(action(0, "down", 21), action(200_000, "up", 21)),
        backend=backend,
        controls=TimedControls(((0.01, "refocus"), (0.03, "quit"))),
        renderer=renderer,
        focus_guard=focus_guard,
        require_focus=True,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
        enable_event_wait=True,
    )

    assert engine.play() == PLAYBACK_QUIT

    assert focus_guard.focus_calls == 1
    assert ("render", "refocus") in renderer.events


def test_threaded_event_wait_pause_can_resume_with_second_f8() -> None:
    backend = ThreadRecordingBackend()
    renderer = BlockingRenderer(block_s=0.0)
    engine = PlaybackEngine(
        song=Song(name="threaded-pause-resume", notes=()),
        actions=(action(0, "down", 21), action(140_000, "up", 21)),
        backend=backend,
        controls=TimedControls(((0.01, "pause"), (0.20, "pause"))),
        renderer=renderer,
        require_focus=False,
        sleep_policy=SleepPolicy(spin_threshold_us=500, poll_s=0.005),
        use_dispatch_thread=True,
        enable_event_wait=True,
    )

    assert engine.play() == "finished"

    statuses = [value for kind, value in renderer.events if kind == "render"]
    assert "paused" in statuses
    assert statuses.count("playing") >= 1
    assert any(call.kind == "release_all" for call in backend.calls)


def test_threaded_event_wait_focus_restore_resumes_without_refocus_command() -> None:
    backend = ThreadRecordingBackend()
    renderer = BlockingRenderer(block_s=0.0)
    engine = PlaybackEngine(
        song=Song(name="threaded-focus-restore", notes=()),
        actions=(action(0, "down", 21), action(300_000, "up", 21)),
        backend=backend,
        renderer=renderer,
        focus_guard=TimedFocusGuard(inactive_from_s=0.03, inactive_to_s=0.15),
        telemetry_enabled=True,
        require_focus=True,
        sleep_policy=SleepPolicy(spin_threshold_us=500, poll_s=0.005),
        focus_restore_grace_us=1_000,
        use_dispatch_thread=True,
        enable_event_wait=True,
    )

    assert engine.play() == "finished"

    statuses = [value for kind, value in renderer.events if kind == "render"]
    assert "focus_lost" in statuses
    assert "playing" in statuses[statuses.index("focus_lost") + 1 :]
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["playback_pause"]["focus"]["count"] >= 1


def test_threaded_dispatch_real_hud_does_not_read_backend_from_control_thread() -> None:
    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-real-hud-owner", notes=()),
        actions=(
            action(0, "down", 21),
            action(80_000, "up", 21),
            action(160_000, "down", 22),
            action(240_000, "up", 22),
        ),
        backend=backend,
        renderer=ProgressRenderer(),
        require_focus=False,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
    )

    engine.play()

    assert backend.calls
    assert {call.thread_id for call in backend.calls} == {backend.calls[0].thread_id}
    assert any(call.kind == "get_health" for call in backend.calls)


def test_threaded_dispatch_tolerates_short_cpu_bound_render_work() -> None:
    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-cpu-render", notes=()),
        actions=tuple(action(index * 20_000, "down", 30 + index) for index in range(8)),
        backend=backend,
        renderer=CpuBoundRenderer(block_s=0.002),
        require_focus=False,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
    )

    engine.play()

    assert_down_intervals_near(
        backend,
        expected_count=8,
        interval_us=20_000,
        tolerance_us=5_000,
    )


def test_threaded_dispatch_tolerates_short_cpu_bound_focus_checks(monkeypatch) -> None:
    # Sky is foreground (focus_guard is active); model the platform foreground state the
    # threaded pre-down gate now consults via ``is_foreground_cached_hwnd`` (§2.1).
    monkeypatch.setattr(inputs, "is_foreground_cached_hwnd", lambda: True)
    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-cpu-focus", notes=()),
        actions=tuple(action(index * 20_000, "down", 40 + index) for index in range(8)),
        backend=backend,
        focus_guard=CpuBoundFocusGuard(block_s=0.002),
        require_focus=True,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
    )

    engine.play()

    assert_down_intervals_near(
        backend,
        expected_count=8,
        interval_us=20_000,
        tolerance_us=5_000,
    )


def test_threaded_dispatch_tolerates_short_cpu_bound_control_poll() -> None:
    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-cpu-controls", notes=()),
        actions=tuple(action(index * 20_000, "down", 50 + index) for index in range(8)),
        backend=backend,
        controls=CpuBoundControls(block_s=0.002),
        require_focus=False,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
    )

    engine.play()

    assert_down_intervals_near(
        backend,
        expected_count=8,
        interval_us=20_000,
        tolerance_us=5_000,
    )


def test_threaded_dispatch_tolerates_realtime_primitive_fallbacks(monkeypatch) -> None:
    monkeypatch.setattr(inputs, "create_high_resolution_waitable_timer", lambda: None)

    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-fallback", notes=()),
        actions=(action(0, "down", 21),),
        backend=backend,
        require_focus=False,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
    )

    engine.play()

    assert any(call.kind == "down" for call in backend.calls)


def test_threaded_dispatch_ablation_flags_skip_realtime_primitives(monkeypatch) -> None:
    def fail_waitable_timer() -> int | None:
        raise AssertionError("waitable timer should be disabled")

    def fail_timer_guard():
        raise AssertionError("timer guard should be disabled")

    monkeypatch.setattr(inputs, "create_high_resolution_waitable_timer", fail_waitable_timer)
    monkeypatch.setattr(inputs, "high_resolution_timer_scope", fail_timer_guard)

    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-ablation", notes=()),
        actions=(action(0, "down", 21),),
        backend=backend,
        require_focus=False,
        telemetry_enabled=True,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
        enable_timer_guard=False,
        enable_waitable_timer=False,
        enable_gc_pause=False,
        enable_switch_interval_tuning=False,
    )

    engine.play()

    assert any(call.kind == "down" for call in backend.calls)
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_options"]["use_dispatch_thread"] is True
    assert summary["runtime_options"]["timer_guard"] is False
    assert summary["runtime_options"]["waitable_timer"] is False
    assert summary["runtime_options"]["gc_pause"] is False
    assert summary["runtime_options"]["switch_interval_tuning"] is False


def test_threaded_dispatch_priority_ladder_telemetry(monkeypatch) -> None:
    import sys
    monkeypatch.setattr(sys, "platform", "win32")

    mocked_characteristics_called = False
    def mock_set_chars(name: str):
        nonlocal mocked_characteristics_called
        mocked_characteristics_called = True
        return 9999

    monkeypatch.setattr(inputs, "av_set_mm_thread_characteristics", mock_set_chars)
    monkeypatch.setattr(inputs, "av_revert_mm_thread_characteristics", lambda h: None)
    monkeypatch.setattr(inputs, "av_set_mm_thread_priority", lambda h, p: True)

    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-priority", notes=()),
        actions=(action(0, "down", 21),),
        backend=backend,
        require_focus=False,
        telemetry_enabled=True,
        use_dispatch_thread=True,
        rt_priority_mode="mmcss",
    )

    engine.play()

    assert any(call.kind == "down" for call in backend.calls)
    assert mocked_characteristics_called is True

    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_options"]["rt_priority_mode"] == "mmcss"
    assert summary["runtime_options"]["rt_priority_acquired"] == "mmcss:Pro Audio"
    assert summary["runtime_options"]["switch_interval_tuning"] is True


def test_threaded_dispatch_enable_event_wait(monkeypatch) -> None:
    create_called = 0
    close_called = 0
    set_called = 0
    wait_called = 0

    def mock_create_event():
        nonlocal create_called
        create_called += 1
        return 7777

    def mock_close_handle(handle: int):
        nonlocal close_called
        if handle == 7777:
            close_called += 1

    def mock_set_event(handle: int):
        nonlocal set_called
        if handle == 7777:
            set_called += 1
        return True

    def mock_wait_multiple(handles: tuple[int, ...], timeout_ms: int):
        nonlocal wait_called
        wait_called += 1
        return inputs.WAIT_OBJECT_0

    monkeypatch.setattr(inputs, "create_auto_reset_event", mock_create_event)
    monkeypatch.setattr(inputs, "close_handle", mock_close_handle)
    monkeypatch.setattr(inputs, "set_event", mock_set_event)
    monkeypatch.setattr(inputs, "wait_for_multiple_objects", mock_wait_multiple)

    monkeypatch.setattr(
        inputs,
        "create_high_resolution_waitable_timer",
        lambda: 8888
    )

    import sys
    monkeypatch.setattr(sys, "platform", "win32")

    backend = ThreadRecordingBackend()
    engine = PlaybackEngine(
        song=Song(name="threaded-event-wait", notes=()),
        actions=(
            action(0, "down", 21),
            action(100_000, "up", 21),
        ),
        backend=backend,
        require_focus=False,
        telemetry_enabled=True,
        use_dispatch_thread=True,
        enable_event_wait=True,
        sleep_policy=SleepPolicy(spin_threshold_us=100),
    )

    monkeypatch.setattr(
        engine,
        "_should_use_dispatch_thread",
        lambda: True
    )

    engine.play()

    assert create_called >= 1
    assert close_called >= 1
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_options"]["enable_event_wait"] is True


