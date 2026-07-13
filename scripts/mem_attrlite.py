"""Trace which engine attributes are heavy after play() returns."""
from __future__ import annotations

import gc
import tracemalloc

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
        return BackendHealth(active_count=len(self.active), possibly_active_count=0,
            failed_release_count=0, last_error=None)

    def get_send_diagnostics(self):
        return {}


def _action(idx: int, kind: str, scan: int) -> KeyAction:
    return KeyAction(kind=kind, scan_codes=(ScanCode(scan),),
        at_us=Microseconds(idx * 20_000), reason="test")


def main() -> None:
    scan_codes_pool = (21, 22, 23, 24, 25, 26, 27, 28, 29, 30)
    actions = []
    for i in range(2000):
        if i % 2 == 0:
            actions.append(_action(i, "down", scan_codes_pool[i % 10]))
        else:
            actions.append(_action(i, "up", scan_codes_pool[(i // 2) % 10]))

    tracemalloc.start()
    snap0 = tracemalloc.take_snapshot()
    engine = PlaybackEngine(
        song=Song(name="m", notes=()),
        actions=tuple(actions),
        backend=_TinyBackend(),
        require_focus=False,
        sleep_policy=SleepPolicy(poll_s=0.001),
        use_dispatch_thread=True,
        enable_gc_pause=True,
    )
    snap1 = tracemalloc.take_snapshot()
    stat_lines = []
    for stat in reversed(sorted(snap1.compare_to(snap0, "lineno"), key=lambda s: -s.size_diff)[:15]):
        if stat.size_diff > 0:
            stat_lines.append((stat.traceback[0], stat.size_diff))
    print("After engine.__init__ (top allocators):")
    for filename, size in stat_lines:
        print(f"  {filename[1]:30s} : {size/1024:>8.1f} KiB")

    caller_ref = engine
    snap2 = tracemalloc.take_snapshot()
    result = engine.play()
    gc.collect()
    snap3 = tracemalloc.take_snapshot()
    print(f"\nplay() -> {result!r}")
    print("\nTop allocations during/after play() (net diff tracemalloc sees):")
    for stat in sorted(snap3.compare_to(snap2, "lineno"), key=lambda s: -s.size_diff)[:15]:
        if stat.size_diff > 0:
            print(f"  {stat.traceback[0][1]:30s} : {stat.size_diff/1024:>8.1f} KiB")

    print("\n--- Engine attribute inspection ---")
    for attr in (
        "_runtime_coordinator", "runtime_schedule", "_health_monitor",
        "estimator", "telemetry", "_compat_loop",
    ):
        v = getattr(engine, attr)
        print(f"  has {attr:>22}? {v is not None}  type={type(v).__name__}")

    print()
    coord = engine._runtime_coordinator
    if coord is not None:
        print(f"  coord.status_by_generation keys:    {len(coord.status_by_generation)}")
        print(f"  coord.schedule.batches:            {len(coord.schedule.batches)}")

    tracemalloc.stop()


if __name__ == "__main__":
    main()
