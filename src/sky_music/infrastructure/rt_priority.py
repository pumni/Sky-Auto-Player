from __future__ import annotations

import contextlib
import sys
from dataclasses import dataclass
from types import TracebackType
from typing import Self

from sky_music.config import RtPriorityMode as RtPriorityMode
from sky_music.platform.win32 import inputs


@dataclass(frozen=True, slots=True)
class RtPriorityOutcome:
    requested_mode: RtPriorityMode
    acquired: str          # e.g. 'mmcss:Pro Audio', 'thread:time_critical', 'off'
    detail: str | None     # failure notes for telemetry/debug

class DispatchThreadPriorityScope:
    """Boost the calling (dispatch) thread's scheduling priority.

    Context manager applied ON the dispatch thread (inside dispatch_target),
    reverted in __exit__ even on exceptions.

    MMCSS task names only select a Windows scheduling profile; they don't require audio work.
    This is the OS-sanctioned per-thread mechanism — no process priority class is touched (I7),
    so the system and other apps are not starved. It interacts with nothing game-side (I5).
    """

    __slots__ = (
        "_mmcss_handle",
        "_old_priority",
        "_thread_handle",
        "mode",
        "outcome",
        "power_throttling_disabled",
    )

    def __init__(self, mode: RtPriorityMode = "auto") -> None:
        self.mode: RtPriorityMode = mode
        self.outcome: RtPriorityOutcome | None = None
        self.power_throttling_disabled: bool = False
        self._mmcss_handle: int | None = None
        self._thread_handle: int | None = None
        self._old_priority: int | None = None

    def _try_disable_power_throttling(self) -> None:
        """Best-effort EcoQoS opt-out so spin deadlines are not stretched by power saving."""
        with contextlib.suppress(Exception):
            self.power_throttling_disabled = bool(inputs.disable_thread_power_throttling())

    def __enter__(self) -> Self:
        if sys.platform != "win32" or self.mode == "off":
            self.outcome = RtPriorityOutcome(requested_mode=self.mode, acquired="off", detail="Disabled or non-win32 platform")
            # Still try to disable power throttling even when priority boost is off — EcoQoS
            # can stretch spin deadlines independently of thread priority class.
            if sys.platform == "win32":
                self._try_disable_power_throttling()
            return self

        errors = []
        self._try_disable_power_throttling()

        # Ladder 1: MMCSS
        if self.mode in ("auto", "mmcss"):
            for name in ("Pro Audio", "Low Latency", "Audio", "Games"):
                try:
                    handle = inputs.av_set_mm_thread_characteristics(name)
                    if handle is not None:
                        self._mmcss_handle = handle
                        self.outcome = RtPriorityOutcome(
                            requested_mode=self.mode,
                            acquired=f"mmcss:{name}",
                            detail=None
                        )
                        # Try to bind priority to High
                        with contextlib.suppress(Exception):
                            inputs.av_set_mm_thread_priority(handle, 1) # AVRT_PRIORITY_HIGH = 1
                        return self
                except Exception as err:
                    errors.append(f"MMCSS {name} failed: {err}")

            if self.mode == "mmcss":
                self.outcome = RtPriorityOutcome(
                    requested_mode=self.mode,
                    acquired="off",
                    detail=f"MMCSS failed: {'; '.join(errors)}"
                )
                return self

        # Ladder 2: THREAD_PRIORITY_TIME_CRITICAL
        if self.mode in ("auto", "time_critical"):
            try:
                h_thread = inputs.get_current_thread()
                old_priority = inputs.get_thread_priority(h_thread)
                if old_priority != 0x7fffffff: # THREAD_PRIORITY_ERROR_RETURN
                    if inputs.set_thread_priority(h_thread, 15): # THREAD_PRIORITY_TIME_CRITICAL = 15
                        self._thread_handle = h_thread
                        self._old_priority = old_priority
                        self.outcome = RtPriorityOutcome(
                            requested_mode=self.mode,
                            acquired="thread:time_critical",
                            detail=None
                        )
                        return self
                    errors.append("SetThreadPriority TIME_CRITICAL returned False")
                else:
                    errors.append("GetThreadPriority returned error code")
            except Exception as err:
                errors.append(f"TIME_CRITICAL boost failed: {err}")

            if self.mode == "time_critical":
                self.outcome = RtPriorityOutcome(
                    requested_mode=self.mode,
                    acquired="off",
                    detail=f"TIME_CRITICAL failed: {'; '.join(errors)}"
                )
                return self

        # Ladder 3: THREAD_PRIORITY_HIGHEST
        if self.mode in ("auto", "highest"):
            try:
                h_thread = inputs.get_current_thread()
                old_priority = inputs.get_thread_priority(h_thread)
                if old_priority != 0x7fffffff:
                    if inputs.set_thread_priority(h_thread, 2): # THREAD_PRIORITY_HIGHEST = 2
                        self._thread_handle = h_thread
                        self._old_priority = old_priority
                        self.outcome = RtPriorityOutcome(
                            requested_mode=self.mode,
                            acquired="thread:highest",
                            detail=None
                        )
                        return self
                    errors.append("SetThreadPriority HIGHEST returned False")
                else:
                    errors.append("GetThreadPriority returned error code")
            except Exception as err:
                errors.append(f"HIGHEST boost failed: {err}")

            if self.mode == "highest":
                self.outcome = RtPriorityOutcome(
                    requested_mode=self.mode,
                    acquired="off",
                    detail=f"HIGHEST failed: {'; '.join(errors)}"
                )
                return self

        # Ladder 4: off
        self.outcome = RtPriorityOutcome(
            requested_mode=self.mode,
            acquired="off",
            detail=f"Auto fall-through: {'; '.join(errors)}" if errors else "Auto-off fallback"
        )
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        if self._mmcss_handle is not None:
            try:
                inputs.av_revert_mm_thread_characteristics(self._mmcss_handle)
            except Exception as err:
                inputs.debug_log(f"[rt_priority] Failed to revert MMCSS: {err}")
            finally:
                self._mmcss_handle = None

        if self._thread_handle is not None and self._old_priority is not None:
            try:
                inputs.set_thread_priority(self._thread_handle, self._old_priority)
            except Exception as err:
                inputs.debug_log(f"[rt_priority] Failed to restore thread priority: {err}")
            finally:
                self._thread_handle = None
                self._old_priority = None
