"""Benchmark gate for the ``pop_due_pending`` single-pending fast path.

The fast path (``len(pending_by_generation) == 1``) is intended to remove the
list comprehension + sort + tuple conversion from the dominant single-key
release path. This module measures the p50/p99 cost of

    1. ``pop_due_pending`` with one pending release  (fast path)
    2. ``pop_due_pending`` with four pending releases (multi-release path — used
       as the noise ceiling: a real win means the single-release path lands at
       or below the multi-release transaction cost floor)

Run under the free-threaded interpreter:

    uv run --env-file .env pytest tests/bench_pop_due_pending.py -m slow \\
        --benchmark-only --benchmark-warmup=on --benchmark-warmup-iterations=50

Skipped on the fast lane (``pytest -m "not slow"``). Marker ``slow`` is
declared in ``pyproject.toml``.
"""
from __future__ import annotations

import pytest

from sky_music.orchestration.core.coordinator import (
    PendingRelease,
    RuntimeDispatchCoordinator,
    RuntimeSchedule,
)

pytestmark = pytest.mark.slow


def _make_coordinator_with_pending(scan_codes: tuple[int, ...]) -> RuntimeDispatchCoordinator:
    empty = RuntimeSchedule(batches=(), generation_count=len(scan_codes))
    coord = RuntimeDispatchCoordinator(empty, min_hold_us=0)
    for gen_id, sc in enumerate(scan_codes):
        coord.pending_by_generation[gen_id] = PendingRelease(
            generation_id=gen_id,
            scan_code=sc,
            source_action_index=gen_id,
            scheduled_release_us=1_000,
            down_dispatch_started_us=0,
            release_not_before_us=0,
            reason="release",
        )
        coord.pending_scan_codes.add(sc)
    return coord


def _reset_before_bench(coord: RuntimeDispatchCoordinator, scan_codes: tuple[int, ...]) -> None:
    """Re-populate the coordinator's pending tables for each benchmark round.

    ``pop_due_pending`` mutates ``pending_by_generation`` and
    ``pending_scan_codes``; pytest-benchmark calls the inner function repeatedly
    so we must restore state on every roundtrip.
    """
    coord.pending_by_generation.clear()
    coord.pending_scan_codes.clear()
    for gen_id, sc in enumerate(scan_codes):
        coord.pending_by_generation[gen_id] = PendingRelease(
            generation_id=gen_id,
            scan_code=sc,
            source_action_index=gen_id,
            scheduled_release_us=1_000,
            down_dispatch_started_us=0,
            release_not_before_us=0,
            reason="release",
        )
        coord.pending_scan_codes.add(sc)


SINGLE = (0x1E,)
SINGLE_2 = (0x1E,)  # same scan code re-populated — exercises single-pending path
QUAD = (0x1E, 0x1F, 0x20, 0x21)


@pytest.mark.benchmark(group="pop_due_pending")
def test_bench_pop_due_pending_single(benchmark):
    """Single pending release — exercises the len==1 fast path."""
    coord = _make_coordinator_with_pending(SINGLE)
    now_us = 1_000  # equal to effective_release_us → due

    def work():
        _reset_before_bench(coord, SINGLE)
        return coord.pop_due_pending(now_us, 0)

    result = benchmark.pedantic(
        work,
        iterations=1,
        rounds=20_000,
        warmup_rounds=200,
    )
    assert isinstance(result, tuple) and len(result) == 1


@pytest.mark.benchmark(group="pop_due_pending")
def test_bench_pop_due_pending_quad(benchmark):
    """Four pending releases — exercises the multi-release list/sort/tuple path.

    Used as the noise ceiling: a real fast-path win means the single-release
    measurement lands at or below the multi-release transaction cost floor,
    not just below its own baseline noise.
    """
    coord = _make_coordinator_with_pending(QUAD)
    now_us = 1_000

    def work():
        _reset_before_bench(coord, QUAD)
        return coord.pop_due_pending(now_us, 0)

    result = benchmark.pedantic(
        work,
        iterations=1,
        rounds=20_000,
        warmup_rounds=200,
    )
    assert isinstance(result, tuple) and len(result) == 4
