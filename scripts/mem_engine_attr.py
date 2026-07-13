"""Verify what engine attributes still hold memory after play() returns."""
from __future__ import annotations

import gc
import sys
import tracemalloc
from pathlib import Path

from sky_music.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import (
    BackendHealth,
    InputSendResult,
    ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PlaybackEngine


class _TinyBackend:
    def __init__(self) -> None:
        self.active: set[int] = set()

    def key_down(self, scan_codes):
        self.active.update(scan_codes)
        return InputSendResult(sent=scan_codes, skipped_duplicates=(), success=True)

    def key_up(self, scan_codes):
        self.active.difference_update(scan_codes)
        return InputSendResult(sent=scan_codes, skipped_duplicates=(), success=True)

    def release_all(self):
        attempted = tuple(sorted(self.active))
        self.active.clear()
        return ReleaseAllOutcome(
            attempted=attempted, released_successfully=True,
            stuck_keys=(), verification_inconclusive=False,
        )

    def get_health(self):
        return BackendHealth(
            active_count=len(self.active), possibly_active_count=0,
            failed_release_count=0, last_error=None,
        )

    def get_send_diagnostics(self):
        return {}


def _action(idx: int, kind: str, scan: int) -> KeyAction:
    return KeyAction(
        kind=kind, scan_codes=(ScanCode(scan),),
        at_us=Microseconds(idx * 20_000), reason="test",
    )


def _deep_size(obj) -> int:
    seen: set[int] = set()
    total = 0
    stack = [obj]
    while stack:
        item = stack.pop()
        if id(item) in seen:
            continue
        seen.add(id(item))
        total += sys.getsizeof(item)
        if isinstance(item, dict):
            stack.extend(item.keys())
            stack.extend(item.values())
        elif isinstance(item, (list, tuple, set, frozenset)):
            stack.extend(item)
        elif hasattr(item, "__dict__"):
            stack.append(item.__dict__)
    return total


def main() -> None:
    scan_codes_pool = (21, 22, 23, 24, 25, 26, 27, 28, 29, 30)
    actions = []
    for i in range(2000):
        if i % 2 == 0:
            sc = scan_codes_pool[i % len(scan_codes_pool)]
            actions.append(_action(i, "down", sc))
        else:
            sc = scan_codes_pool[(i // 2) % len(scan_codes_pool)]
            actions.append(_action(i, "up", sc))

    engine = PlaybackEngine(
        song=Song(name="m", notes=()),
        actions=tuple(actions),
        backend=_TinyBackend(),
        require_focus=False,
        sleep_policy=SleepPolicy(poll_s=0.001),
        use_dispatch_thread=True,
        enable_gc_pause=True,
    )

    caller_ref = engine
    result = engine.play()
    gc.collect()
    print(f"play() -> {result!r}\n")

    print("Per-attribute costs while caller keeps engine referenced:")
    for attr in (
        "_runtime_coordinator", "runtime_schedule", "_health_monitor",
        "estimator", "telemetry", "backend", "_compat_loop",
    ):
        val = getattr(engine, attr)
        sz = _deep_size(val)
        print(f"  {attr:>22}: {sz / 1024:>8.1f} KiB  (type={type(val).__name__})")

    if engine._runtime_coordinator is not None:
        coord = engine._runtime_coordinator
        print("\n  coordinator internals:")
        for attr in ("status_by_generation", "schedule", "active_by_scan_code",
                     "pending_by_generation", "pending_scan_codes"):
            val = getattr(coord, attr)
            sz = _deep_size(val)
            extra = f" len={len(val)}" if hasattr(val, "__len__") else ""
            print(f"    {attr:<28}{extra}: {sz / 1024:>8.1f} KiB")

    print(f"\n  Total caller_referenced engine: {_deep_size(caller_ref) / 1024:.1f} KiB")


if __name__ == "__main__":
    main()
