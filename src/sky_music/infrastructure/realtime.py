from __future__ import annotations

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
    def __init__(self, task_name: str = "Pro Audio") -> None:
        self.task_name = task_name
        self.handle: int | None = None

    def __enter__(self) -> Self:
        try:
            self.handle = inputs.av_set_mm_thread_characteristics(self.task_name)
        except Exception as exc:
            inputs.debug_log(f"[realtime] MMCSS registration failed: {exc}")
            self.handle = None
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
