"""Measure memory held by PlaybackEngine after play() returns.

We want to know which sub-object on the engine still pins resident memory
when the song is done / skipped. Helps us pinpoint where to free.
"""
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
            attempted=attempted,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )

    def get_health(self):
        return BackendHealth(
            active_count=len(self.active),
            possibly_active_count=0,
            failed_release_count=0,
            last_error=None,
        )

    def get_send_diagnostics(self):
        return {}


def _action(idx: int, kind: str, scan: int) -> KeyAction:
    return KeyAction(
        kind=kind,
        scan_codes=(ScanCode(scan),),
        at_us=Microseconds(idx * 20_000),
        reason="test",
    )


def _report_top(snapshot_filter_traces, label: str) -> None:
    """Top memory consumers attached to the engine's mutable state."""
    print(f"\n--- {label} top holdouts (tracemalloc snapshot diff) ---")
    snaps = snapshot_filter_traces.most_common(10)
    if not snaps:
        print("  (empty)")
        return
    for stat in snaps:
        print(f"  {stat.traceback._frames[0] if stat.traceback else '?'}: "
              f"{stat.size_diff / 1024:.1f} KiB ({stat.count_diff} objs)")


def main() -> None:
    actions = []
    scan_codes_pool = (21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35)
    for i in range(3000):
        if i % 2 == 0:
            sc = scan_codes_pool[i % len(scan_codes_pool)]
            actions.append(_action(i, "down", sc))
        else:
            sc = scan_codes_pool[(i // 2) % len(scan_codes_pool)]
            actions.append(_action(i, "up", sc))

    backend = _TinyBackend()

    # Reset gc + measure baseline BEFORE engine construction
    gc.collect()
    gc.disable()  # synchronize allocations
    tracemalloc.start()

    # ==== Stage 1: Build engine ====
    snap_pre = tracemalloc.take_snapshot()
    engine = PlaybackEngine(
        song=Song(name="mem-test", notes=()),
        actions=tuple(actions),
        backend=backend,
        require_focus=False,
        sleep_policy=SleepPolicy(poll_s=0.001),
        use_dispatch_thread=True,
        enable_gc_pause=True,
        dispatch_lead_us=0,
    )
    gc.collect()
    snap_engine = tracemalloc.take_snapshot()
    diff = snap_engine.compare_to(snap_pre, "filename")
    print("=== Stage 1: engine.__init__ retention ===")
    total_init = 0
    for stat in diff[:15]:
        if stat.size_diff <= 0:
            continue
        total_init += stat.size_diff
        frames = stat.traceback._frames[:1] if stat.traceback else []
        where = frames[0] if frames else "?"
        print(f"  {where}: {stat.size_diff / 1024:.1f} KiB")
    print(f"  TOTAL init: {total_init / 1024:.1f} KiB")

    # ==== Stage 2: play() returns ====
    snap_pre_play = tracemalloc.take_snapshot()
    result = engine.play()
    print(f"\nplay() returned: {result!r}")
    gc.collect()
    snap_post_play = tracemalloc.take_snapshot()
    diff = snap_post_play.compare_to(snap_pre_play, "filename")
    print("=== Stage 2: net retention after play() finished ===")
    total_play = 0
    for stat in diff[:15]:
        if stat.size_diff <= 0:
            continue
        total_play += stat.size_diff
        frames = stat.traceback._frames[:1] if stat.traceback else []
        where = frames[0] if frames else "?"
        print(f"  {where}: {stat.size_diff / 1024:.1f} KiB")
    print(f"  TOTAL added during play: {total_play / 1024:.1f} KiB")

    # ==== Stage 3: erase engine; measure what survives ====
    # Capture engine's deep footprint before drop
    keep_engine_ref = engine
    snap_held = tracemalloc.take_snapshot()
    id(engine)
    id(engine._runtime_coordinator) if engine._runtime_coordinator else 0
    id(engine.runtime_schedule)

    # Drop it
    engine = None  # type: ignore[assignment]
    gc.collect()
    snap_post_drop = tracemalloc.take_snapshot()
    diff = snap_post_drop.compare_to(snap_held, "filename")
    print("=== Stage 3: net retention after the engine reference was dropped ===")
    total_retained = 0
    for stat in diff[:15]:
        if stat.size_diff <= 0:
            continue
        total_retained += stat.size_diff
        frames = stat.traceback._frames[:1] if stat.traceback else []
        where = frames[0] if frames else "?"
        print(f"  {where}: {stat.size_diff / 1024:.1f} KiB")
    print(f"  TOTAL retained after drop: {total_retained / 1024:.1f} KiB")
    print(f"  keep_engine_ref still alive: {keep_engine_ref!r}")

    gc.enable()


if __name__ == "__main__":
    main()
