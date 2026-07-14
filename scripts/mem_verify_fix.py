"""Verify post-play() memory: does engine still hold heavy data?"""
from __future__ import annotations

import gc
import sys

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
    def __init__(self):
        self.active = set()

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


def _action(idx, kind, scan):
    return KeyAction(kind=kind, scan_codes=(ScanCode(scan),),
        at_us=Microseconds(idx * 20_000), reason="test")


def main():
    scan_codes_pool = (21, 22, 23)
    actions = []
    # Make the test small — only need to verify state, not actually run for 40 seconds.
    for i in range(10):
        if i % 2 == 0:
            actions.append(_action(i, "down", scan_codes_pool[i % 3]))
        else:
            actions.append(_action(i, "up", scan_codes_pool[(i // 2) % 3]))

    engine = PlaybackEngine(
        song=Song(name="m", notes=()),
        actions=tuple(actions),
        backend=_TinyBackend(),
        require_focus=False,
        sleep_policy=SleepPolicy(poll_s=0.001),
        use_dispatch_thread=False,  # synchronous — no threading
        enable_gc_pause=True,
    )

    print("Engine build complete. totals:")
    print(f"  runtime_schedule  : {sys.getsizeof(engine.runtime_schedule)} bytes (slots see only this)")
    print(f"  runtime_coordinator: {engine._runtime_coordinator}")
    print(f"  compat_loop       : {engine._compat_loop}")

    result = engine.play()
    gc.collect()
    print(f"\nplay() -> {result!r}")
    print("\nAfter play() and gc.collect():")
    print(f"  runtime_schedule   : {engine.runtime_schedule}")
    print(f"  _runtime_coordinator: {engine._runtime_coordinator}")
    print(f"  _compat_loop       : {engine._compat_loop}")
    print()
    print(f"  test_runtime_schedule_is_None? {engine.runtime_schedule is None}")
    print(f"  test_runtime_coordinator_is_None? {engine._runtime_coordinator is None}")
    print(f"  test_compat_loop_is_None? {engine._compat_loop is None}")

    print("\nEngine still has:", end=" ")
    kept = []
    for attr in ("actions", "song", "telemetry", "_health_monitor", "estimator",
                 "backend", "sleeper", "clock", "sleep_policy", "focus_guard",
                 "controls", "renderer", "lead_cache_path"):
        val = getattr(engine, attr, None)
        kept.append(f"{attr}={type(val).__name__}")
    print(", ".join(kept[:6]))


if __name__ == "__main__":
    main()
