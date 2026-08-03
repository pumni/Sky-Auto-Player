"""Interpreter admission checks retained at application startup."""

from __future__ import annotations

import gc
import sys
import time


def collect_gc_with_stats(phase: str) -> dict[str, int | str]:
    started_ns = time.perf_counter_ns()
    return {
        "phase": phase,
        "duration_us": (time.perf_counter_ns() - started_ns) // 1_000,
        "collected": gc.collect(),
    }


def _gil_enabled() -> bool:
    probe = getattr(sys, "_is_gil_enabled", None)
    return bool(probe()) if probe is not None else True


class FreeThreadedRuntimeError(RuntimeError):
    """Raised when playback is started on an unsupported interpreter."""


def assert_free_threaded_runtime() -> None:
    import sysconfig

    if sysconfig.get_config_var("Py_GIL_DISABLED") != 1:
        raise FreeThreadedRuntimeError(
            "Sky Auto Player requires a free-threaded CPython build (Py_GIL_DISABLED == 1)."
        )
    if _gil_enabled():
        raise FreeThreadedRuntimeError(
            "Sky Auto Player requires the GIL to be disabled at runtime; it may have been re-enabled."
        )
