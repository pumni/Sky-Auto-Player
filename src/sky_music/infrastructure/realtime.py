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


class MmcssRegistration:
    """Boost the calling (dispatch) thread's scheduling via MMCSS.

    The task name only selects a Windows scheduling profile; it does not require the thread to do
    audio.  We try task profiles strongest-first and register under the best one the machine
    actually has, so the dispatch thread always gets the highest real-time scheduling guarantee
    available.  ``Pro Audio`` (High scheduling category) is the strongest and is present on
    essentially every install; the rest are a safety net for stripped-down systems.  Empirically,
    ``Low Latency`` is absent on some installs, hence the chain rather than a single name.
    """

    DEFAULT_TASK_NAMES: tuple[str, ...] = ("Pro Audio", "Low Latency", "Audio", "Games")

    def __init__(self, task_names: tuple[str, ...] = DEFAULT_TASK_NAMES) -> None:
        self.task_names = task_names
        self.task_name: str | None = None
        self.handle: int | None = None

    def __enter__(self) -> Self:
        for name in self.task_names:
            try:
                handle = inputs.av_set_mm_thread_characteristics(name)
            except Exception as exc:
                inputs.debug_log(f"[realtime] MMCSS registration failed for {name!r}: {exc}")
                continue
            if handle is not None:
                self.handle = handle
                self.task_name = name
                inputs.debug_log(f"[realtime] MMCSS thread registered as {name!r}")
                return self
        inputs.debug_log("[realtime] MMCSS registration unavailable; dispatch runs unboosted")
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        if self.handle is None:
            return
        try:
            inputs.av_revert_mm_thread_characteristics(self.handle)
        except Exception as exc:
            inputs.debug_log(f"[realtime] MMCSS revert failed: {exc}")
        finally:
            self.handle = None


class RealtimeProcessScope:
    """Pause cyclic GC for the duration of dispatch, reverting on exit.

    Thread *scheduling* is handled the Windows-sanctioned way by ``MmcssRegistration`` (best
    available task profile, strongest-first), which raises only the dispatch thread — no
    process-wide priority class is touched, so other apps and the OS are not starved.  The one
    source of jitter MMCSS cannot address is Python's cyclic
    garbage collector firing on the dispatch thread mid-send (which can drop notes), so we collect
    accumulated picker-era garbage once up front and then pause GC until playback ends.

    GC is re-enabled in ``__exit__`` so the picker/idle phases keep normal behaviour.
    """

    __slots__ = ("_gc_was_enabled",)

    def __enter__(self) -> Self:
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
