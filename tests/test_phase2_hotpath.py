"""Phase 2 hot-path hygiene: telemetry flush off RT, cheap focus, unfocused hook.

See docs/2026-07_core-dispatch-refactor-and-isolation-plan.md §5.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any
from unittest.mock import MagicMock

from sky_music.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.dispatch_loop import DispatchHealthMonitor, PlaybackState
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.telemetry import (
    _TELEMETRY_FLUSH_CHUNK,
    TelemetryLogger,
)


class FakeClock:
    def __init__(self, time_us: int = 0) -> None:
        self.time_us = time_us

    def now_us(self) -> int:
        return self.time_us


class FakeSleeper:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock

    def sleep(self, seconds: float) -> None:
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class NullControls:
    def poll(self) -> str | None:
        return None


class SequenceControls:
    def __init__(self, commands: list[str | None]) -> None:
        self.commands = list(commands)

    def poll(self) -> str | None:
        if not self.commands:
            return None
        return self.commands.pop(0)


def _action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(sc) for sc in scan_codes),
        at_us=Microseconds(at_us),
        reason="phase2",
    )


def _many_events(n: int) -> tuple[KeyAction, ...]:
    """Build n down/up pairs with 10 ms spacing and 5 ms hold (enough for min_hold)."""
    actions: list[KeyAction] = []
    for i in range(n):
        t = 10_000 + i * 20_000
        sc = 21 + (i % 5)
        actions.append(_action(t, "down", sc))
        actions.append(_action(t + 10_000, "up", sc))
    return tuple(actions)


# --- A3: telemetry flush off hot path ---------------------------------------


def test_record_does_not_soft_flush_25k_events_until_save(
    tmp_path: Path, monkeypatch: Any
) -> None:
    """25k events, no pause: record() does not soft-flush; save() writes all rows."""
    from sky_music.orchestration import telemetry as tel_mod

    flush_calls: list[str] = []
    original_flush = TelemetryLogger._flush_records_to_csv

    def counting_flush(self: TelemetryLogger, *, clear: bool = True) -> None:
        flush_calls.append("flush")
        return original_flush(self, clear=clear)

    monkeypatch.setattr(TelemetryLogger, "_flush_records_to_csv", counting_flush)

    tel = TelemetryLogger("tel-flush", enabled=True)
    tel.log_filepath = tmp_path / "tel.csv"
    n_events = 25_000
    for i in range(n_events):
        tel.record(
            event_index=i,
            kind="down",
            scheduled_us=i * 1000,
            actual_us=i * 1000,
            lateness_us=0,
            send_duration_us=50,
            scan_codes=(21,),
            reason="t",
        )
    # Soft chunk is 10k; without pause, record() must not flush below hard cap (200k).
    assert len(flush_calls) == 0, "record() must not soft-flush mid-stream under hard cap"
    assert len(tel.records) == n_events
    tel.save()
    assert len(flush_calls) == 1, "save() must flush remaining records exactly once"
    assert (tmp_path / "tel.csv").exists()
    assert tel_mod._TELEMETRY_FLUSH_CHUNK == 10_000


def test_pause_path_flush_fires_when_buffer_large(tmp_path: Path, monkeypatch: Any) -> None:
    """With a mid-song pause after soft-chunk events, pause-path flush runs."""
    flush_calls: list[str] = []
    original_flush = TelemetryLogger._flush_records_to_csv

    def counting_flush(self: TelemetryLogger, *, clear: bool = True) -> None:
        flush_calls.append("flush")
        return original_flush(self, clear=clear)

    monkeypatch.setattr(TelemetryLogger, "_flush_records_to_csv", counting_flush)

    clock = FakeClock()
    tel = TelemetryLogger("pause-flush", enabled=True)
    tel.log_filepath = tmp_path / "pause.csv"
    for i in range(_TELEMETRY_FLUSH_CHUNK + 100):
        tel.record(
            event_index=i,
            kind="down",
            scheduled_us=i * 1000,
            actual_us=i * 1000,
            lateness_us=0,
            send_duration_us=100,
            scan_codes=(21,),
            reason="t",
        )
    assert len(flush_calls) == 0, "record() must not soft-flush under hard cap"
    tel.record_pause("manual", 1_000)
    assert len(flush_calls) >= 1, "record_pause must soft-flush when buffer large"

    # Also exercise DispatchLoop paused branch flush_if_large via process_wait_states.
    state = PlaybackState(start_perf=0)
    state.enter_pause("manual", 0)
    engine = PlaybackEngine(
        song=Song(name="pause-branch", notes=()),
        actions=(_action(10_000, "down", 21), _action(20_000, "up", 21)),
        backend=DryRunBackend(),
        controls=SequenceControls([]),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1, poll_s=0.001),
        min_hold_us=5_000,
        use_dispatch_thread=False,
    )
    engine.telemetry.log_filepath = tmp_path / "branch.csv"
    for i in range(_TELEMETRY_FLUSH_CHUNK + 50):
        engine.telemetry.record(
            event_index=i,
            kind="down",
            scheduled_us=i,
            actual_us=i,
            lateness_us=0,
            send_duration_us=0,
            scan_codes=(21,),
            reason="t",
        )
    before = len(flush_calls)
    engine._process_wait_states(state, True, 1.0)
    assert len(flush_calls) > before, "manual-pause wait branch must call flush_if_large"


# --- A4: focus path uses runtime signal, not focus_guard from dispatch -----


def test_health_monitor_prefers_runtime_signal_over_guard() -> None:
    guard = MagicMock()
    guard.is_active.return_value = False
    clock = FakeClock(0)
    mon = DispatchHealthMonitor(
        backend=DryRunBackend(),
        clock=clock,
        focus_guard=guard,
        require_focus=True,
    )
    signal = MagicMock()
    signal.is_active.return_value = True
    mon.set_runtime_signal(signal)
    assert mon.focus_is_active() is True
    signal.is_active.assert_called()
    guard.is_active.assert_not_called()


def test_unfocused_send_hook_called_instead_of_platform_import() -> None:
    """DispatchLoop must bump the counter via injected hook, not a hot-path import."""
    from sky_music.orchestration.dispatch_loop import DispatchLoop
    from sky_music.orchestration.runtime_dispatch import (
        RuntimeDispatchCoordinator,
        RuntimeSchedule,
    )

    clock = FakeClock()
    hook_calls: list[int] = []

    class _AlwaysUnfocused:
        def is_active(self) -> bool:
            return False

    mon = DispatchHealthMonitor(
        backend=DryRunBackend(),
        clock=clock,
        focus_guard=_AlwaysUnfocused(),  # type: ignore[arg-type]
        require_focus=False,
    )
    mon.set_runtime_signal(_AlwaysUnfocused())  # type: ignore[arg-type]

    class _NullWait:
        def spin_until_us(self, *a: object, **k: object) -> None:
            return

        def wait_until_us(self, *a: object, **k: object) -> bool:
            return False

    loop = DispatchLoop(
        coordinator=RuntimeDispatchCoordinator(
            RuntimeSchedule((), generation_count=0), min_hold_us=0
        ),
        clock=clock,
        sleeper=FakeSleeper(clock),
        wait_strategy=_NullWait(),  # type: ignore[arg-type]
        backend=DryRunBackend(),
        telemetry=TelemetryLogger("hook", enabled=False),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        health_monitor=mon,
        min_hold_us=0,
        spin_threshold_us=-1,
        unfocused_send_hook=lambda: hook_calls.append(1),
    )
    state = PlaybackState(start_perf=0)
    action = _action(0, "down", 21)
    loop._execute_action(0, action, state)
    assert hook_calls == [1]


def test_dispatch_loop_has_no_hot_path_platform_import() -> None:
    """Grep-equivalent: platform import only allowed in TYPE_CHECKING / finally diag."""
    import ast
    from pathlib import Path

    src = Path("src/sky_music/orchestration/dispatch_loop.py").read_text(encoding="utf-8")
    tree = ast.parse(src)
    forbidden_in_funcs = {"_execute_action"}
    hits: list[str] = []

    class Visitor(ast.NodeVisitor):
        def __init__(self) -> None:
            self._stack: list[str] = []

        def visit_FunctionDef(self, node: ast.FunctionDef) -> None:
            self._stack.append(node.name)
            self.generic_visit(node)
            self._stack.pop()

        def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef) -> None:
            self._stack.append(node.name)
            self.generic_visit(node)
            self._stack.pop()

        def visit_ImportFrom(self, node: ast.ImportFrom) -> None:
            mod = node.module or ""
            if (
                "sky_music.platform" in mod
                and self._stack
                and self._stack[-1] in forbidden_in_funcs
            ):
                hits.append(f"{self._stack[-1]}:{node.lineno}")
            self.generic_visit(node)

    Visitor().visit(tree)
    assert not hits, f"platform import still inside hot path: {hits}"
