from __future__ import annotations

import gc
from dataclasses import dataclass
from types import TracebackType
from typing import Self

from sky_music.infrastructure.timing import Sleeper
from sky_music.platform.win32 import inputs


@dataclass(slots=True)
class WaitableTimerSleeper:
    handle: int
    fallback: Sleeper

    @classmethod
    def create(cls, fallback: Sleeper) -> "WaitableTimerSleeper | None":
        handle = inputs.create_high_resolution_waitable_timer()
        if handle is None:
            return None
        return cls(handle=handle, fallback=fallback)

    def sleep(self, seconds: float) -> None:
        if seconds <= 0:
            self.fallback.sleep(seconds)
            return
        delay_us = max(1, int(seconds * 1_000_000))
        if not inputs.set_waitable_timer_relative_us(self.handle, delay_us):
            self.fallback.sleep(seconds)
            return
        inputs.wait_for_timer(self.handle)

    def close(self) -> None:
        if self.handle:
            inputs.close_handle(self.handle)
            self.handle = 0

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        self.close()


class RealtimeProcessScope:
    """Pause cyclic GC for the duration of dispatch, reverting on exit.

    No process-wide priority class or MMCSS boost is touched, so other apps and the OS are not
    starved.  One source of jitter we can address in Python is cyclic garbage collection firing on
    the dispatch thread mid-send, so we collect accumulated picker-era garbage once up front and
    then pause GC until playback ends.

    GC is re-enabled in ``__exit__`` so the picker/idle phases keep normal behaviour.
    """

    __slots__ = ("_enabled", "_gc_was_enabled")

    def __init__(self, *, enabled: bool = True) -> None:
        self._enabled = enabled
        self._gc_was_enabled = False

    def __enter__(self) -> Self:
        if not self._enabled:
            inputs.debug_log("[realtime] cyclic GC pause disabled for dispatch")
            return self

        # Collect picker-era garbage first so a full collection cannot fire in the middle of the
        # precise dispatch loop, then pause GC for the run.
        self._gc_was_enabled = gc.isenabled()
        if self._gc_was_enabled:
            try:
                gc.collect()
            except Exception:
                pass
            gc.disable()
            inputs.debug_log("[realtime] cyclic GC paused for dispatch")
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        if self._gc_was_enabled:
            gc.enable()
            self._gc_was_enabled = False


def create_realtime_sleeper(fallback: Sleeper) -> Sleeper:
    try:
        sleeper = WaitableTimerSleeper.create(fallback)
    except Exception as exc:
        inputs.debug_log(f"[realtime] high-resolution waitable timer unavailable: {exc}")
        sleeper = None
    if sleeper is None:
        inputs.debug_log("[realtime] using existing precise sleeper fallback")
        return fallback
    inputs.debug_log("[realtime] using high-resolution waitable timer")
    return sleeper
