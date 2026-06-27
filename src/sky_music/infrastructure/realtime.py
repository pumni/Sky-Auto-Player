from __future__ import annotations

import contextlib
import gc
import sys
from dataclasses import dataclass
from types import TracebackType
from typing import ClassVar, Self

from sky_music.infrastructure.timing import Sleeper
from sky_music.platform.win32 import inputs


@dataclass(slots=True)
class WaitableTimerSleeper:
    # Capability flag consumed by HybridWaitStrategy: wakes with sub-millisecond accuracy, so the
    # timer-aware ladder may sleep straight to target - guard. ClassVar, not a dataclass field.
    is_high_resolution: ClassVar[bool] = True

    handle: int
    fallback: Sleeper

    @classmethod
    def create(cls, fallback: Sleeper) -> WaitableTimerSleeper | None:
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


# Cap GIL handoff latency while the Textual dashboard renders in parallel with dispatch
# (the accepted live-dashboard design). Default CPython switch interval is 5 ms — a UI thread mid-
# bytecode can deny the spinning dispatch thread the GIL for up to ~5 ms.
# ctypes WinDLL calls release the GIL during the foreign call, so SendInput itself never blocks
# the UI and vice versa; this knob only shortens bytecode-vs-bytecode handoff.
DISPATCH_SWITCH_INTERVAL_S = 0.001


def _gil_enabled() -> bool:
    """Return True when the GIL is active in the current interpreter.

    ``sys._is_gil_enabled()`` exists from CPython 3.13+ and returns False
    only on free-threaded builds (``python3.14t``).  On older builds the GIL
    is always present, so we default to True.
    """
    probe = getattr(sys, "_is_gil_enabled", None)
    return bool(probe()) if probe is not None else True


class RealtimeProcessScope:
    """Pause cyclic GC for the duration of dispatch, reverting on exit.

    No process-wide priority class or MMCSS boost is touched, so other apps and the OS are not
    starved.  One source of jitter we can address in Python is cyclic garbage collection firing on
    the dispatch thread mid-send, so we collect accumulated picker-era garbage once up front and
    then pause GC until playback ends.

    This scope also tunes the CPython GIL switch interval (if enabled) to minimize handoff latency
    between the Textual UI/dashboard thread and the spinning dispatch thread.

    GC is re-enabled and GIL switch interval restored in ``__exit__`` so the picker/idle phases
    keep normal behaviour.
    """

    __slots__ = (
        "_enable_switch_interval_tuning",
        "_enabled",
        "_gc_was_enabled",
        "_old_switch_interval",
    )

    def __init__(
        self,
        *,
        enabled: bool = True,
        enable_switch_interval_tuning: bool = True,
    ) -> None:
        self._enabled = enabled
        self._enable_switch_interval_tuning = enable_switch_interval_tuning
        self._gc_was_enabled = False
        self._old_switch_interval: float | None = None

    def __enter__(self) -> Self:
        # 1. GC Pause
        if self._enabled:
            self._gc_was_enabled = gc.isenabled()
            if self._gc_was_enabled:
                with contextlib.suppress(Exception):
                    gc.collect()
                gc.disable()
                inputs.debug_log("[realtime] cyclic GC paused for dispatch")
        else:
            inputs.debug_log("[realtime] cyclic GC pause disabled for dispatch")

        # 2. GIL Switch-Interval Tuning
        if self._enable_switch_interval_tuning and _gil_enabled():
            self._old_switch_interval = sys.getswitchinterval()
            sys.setswitchinterval(DISPATCH_SWITCH_INTERVAL_S)
            inputs.debug_log(f"[realtime] GIL switch interval tuned to {DISPATCH_SWITCH_INTERVAL_S}s")
        elif self._enable_switch_interval_tuning:
            inputs.debug_log("[realtime] free-threaded build: switch-interval tuning skipped (no GIL)")
        else:
            inputs.debug_log("[realtime] GIL switch interval tuning disabled")

        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        # 1. Restore GC
        if self._gc_was_enabled:
            gc.enable()
            self._gc_was_enabled = False

        # 2. Restore GIL Switch-Interval
        if self._old_switch_interval is not None:
            sys.setswitchinterval(self._old_switch_interval)
            self._old_switch_interval = None


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
