from __future__ import annotations

import threading
import time
from dataclasses import dataclass

from sky_music.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import BackendHealth, InputSendResult, ReleaseAllOutcome
from sky_music.platform.win32 import inputs
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.ui.hud import ProgressRenderer
from sky_music.orchestration.engine import PLAYBACK_QUIT, PlaybackEngine


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

    def get_health(self) -> BackendHealth:
        self._record("get_health", ())
        return BackendHealth(
            active_count=len(self.active),
            possibly_active_count=0,
            failed_release_count=0,
            last_error=None,
        )


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

    def update_counters(self, lateness_us: int) -> None:
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

    def update_counters(self, lateness_us: int) -> None:
        self.events.append(("counter", str(lateness_us)))


class BlockingFocusGuard:
    def __init__(self, block_s: float) -> None:
        self.block_s = block_s

    def is_active(self) -> bool:
        time.sleep(self.block_s)
        return True

    def focus(self) -> bool:
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
        for left, right in zip(down_calls, down_calls[1:])
    ]
    assert max(abs(actual_us - interval_us) for actual_us in intervals_us) < tolerance_us


def test_threaded_dispatch_isolates_onsets_from_slow_ui_and_focus() -> None:
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


def test_threaded_dispatch_tolerates_short_cpu_bound_focus_checks() -> None:
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

    def fail_mmcss(task_name: str) -> int | None:
        raise OSError("MMCSS unavailable")

    monkeypatch.setattr(inputs, "av_set_mm_thread_characteristics", fail_mmcss)

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
