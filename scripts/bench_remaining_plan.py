"""Evidence harness for the gated refinement-plan candidates.

This module deliberately benchmarks prototypes and current contracts only. It
does not change production telemetry, prewarm policy, lead-cache schema, or
power policy.

Run with::

    uv run --env-file .env python scripts/bench_remaining_plan.py
"""

from __future__ import annotations

import argparse
import gc
import json
import os
import statistics
import sys
import tempfile
import time
import tracemalloc
from collections import Counter
from pathlib import Path
from typing import Any

from sky_music.domain.scheduler_types import ActionKind
from sky_music.orchestration.engine import SendLatencyEstimator
from sky_music.orchestration.telemetry import TelemetryLogger

DISPATCHES = 100_000
TELEMETRY_ROUNDS = 3


class BoundedSummaryPrototype:
    """Fixed-memory summary candidate used only for Phase 7 evidence."""

    __slots__ = ("count", "histogram", "lateness_max", "lateness_min", "lateness_sum")

    def __init__(self) -> None:
        self.count = 0
        self.lateness_sum = 0
        self.lateness_min = 2**63 - 1
        self.lateness_max = -(2**63)
        self.histogram = [0] * 16

    def record(self, lateness_us: int) -> None:
        self.count += 1
        self.lateness_sum += lateness_us
        self.lateness_min = min(self.lateness_min, lateness_us)
        self.lateness_max = max(self.lateness_max, lateness_us)
        self.histogram[min(max(lateness_us, 0) // 50, len(self.histogram) - 1)] += 1


def _dispatch_values(index: int) -> tuple[int, int, int]:
    lateness = (index * 37) % 801
    send_duration = 20 + ((index * 13) % 81)
    return lateness, send_duration, index % 2


def _measure_telemetry(mode: str) -> dict[str, int | None]:
    gc.collect()
    tracemalloc.start()
    started_wall = time.perf_counter_ns()
    started_process = time.process_time_ns()
    started_thread = time.thread_time_ns()
    retained_records = 0

    if mode == "off":
        logger: TelemetryLogger | None = TelemetryLogger("bench", enabled=False)
        summary: BoundedSummaryPrototype | None = None
    elif mode == "full":
        logger = TelemetryLogger("bench", enabled=True, run_id="plan-bench")
        summary = None
    else:
        logger = None
        summary = BoundedSummaryPrototype()

    for index in range(DISPATCHES):
        lateness, send_duration, kind_index = _dispatch_values(index)
        kind = "down" if kind_index == 0 else "up"
        if logger is not None:
            logger.record(
                event_index=index,
                kind=kind,
                scheduled_us=index * 1_000,
                actual_us=index * 1_000 + lateness,
                lateness_us=lateness,
                send_duration_us=send_duration,
                scan_codes=(21 + kind_index,),
                reason="bench",
            )
        else:
            assert summary is not None
            summary.record(lateness)

    retained_records = len(logger.records) if logger is not None else 0
    current_bytes, peak_bytes = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    return {
        "wall_us": (time.perf_counter_ns() - started_wall) // 1_000,
        "process_cpu_us": (time.process_time_ns() - started_process) // 1_000,
        "thread_cpu_us": (time.thread_time_ns() - started_thread) // 1_000,
        "tracemalloc_current_bytes": current_bytes,
        "tracemalloc_peak_bytes": peak_bytes,
        "retained_records": retained_records,
        "working_set_bytes": None,
    }


def run_telemetry_benchmark() -> dict[str, dict[str, int | None]]:
    def median_value(samples: list[dict[str, int | None]], key: str) -> int | None:
        values = [sample[key] for sample in samples]
        numeric_values = [value for value in values if isinstance(value, int)]
        return round(statistics.median(numeric_values)) if numeric_values else None

    # Full mode creates its log directory in the current directory. Isolate
    # that side effect in a temporary directory, and do not call save().
    original_cwd = Path.cwd()
    with tempfile.TemporaryDirectory(prefix="sky-plan-telemetry-") as temp_dir:
        os.chdir(temp_dir)
        try:
            measurements = {
                mode: [_measure_telemetry(mode) for _ in range(TELEMETRY_ROUNDS)]
                for mode in ("off", "summary", "full")
            }
            return {
                mode: {
                    key: median_value(measurements_for_mode, key)
                    for key in measurements_for_mode[0]
                }
                for mode, measurements_for_mode in measurements.items()
            }
        finally:
            os.chdir(original_cwd)


def run_single_telemetry_mode(mode: str) -> dict[str, int | None]:
    original_cwd = Path.cwd()
    with tempfile.TemporaryDirectory(prefix="sky-plan-telemetry-") as temp_dir:
        os.chdir(temp_dir)
        try:
            return _measure_telemetry(mode)
        finally:
            os.chdir(original_cwd)


def _corpora() -> dict[str, list[tuple[tuple[int, ...], bool]]]:
    few_shapes = [((21,), False), ((21,), True), ((21, 22), False), ((21, 22), True)]
    few = [few_shapes[index % len(few_shapes)] for index in range(100_000)]
    unique = [((index,), index % 2 == 1) for index in range(10_000)]
    hot = [((21,), False), ((21,), True), ((22, 23), False), ((22, 23), True)]
    cold = [((100 + index,), index % 2 == 1) for index in range(8_000)]
    mixed = [hot[index % len(hot)] for index in range(92_000)] + cold
    return {"few_repeated": few, "thousands_unique": unique, "mixed_hot_cold": mixed}


def _budget_selection(
    events: list[tuple[tuple[int, ...], bool]], slot_budget: int
) -> tuple[int, int, int]:
    frequencies = Counter(events)
    ordered = sorted(
        frequencies,
        key=lambda shape: (len(shape[0]) != 1, -frequencies[shape], shape),
    )
    selected: set[tuple[tuple[int, ...], bool]] = set()
    slots = 0
    for shape in ordered:
        shape_slots = len(shape[0])
        if slots + shape_slots > slot_budget:
            continue
        selected.add(shape)
        slots += shape_slots
    first_hit_misses = sum(shape not in selected for shape in frequencies)
    total_events_missed = sum(1 for shape in events if shape not in selected)
    return len(selected), slots, first_hit_misses + total_events_missed


def run_prewarm_benchmark() -> dict[str, dict[str, int | float]]:
    from sky_music.platform.win32 import inputs

    results: dict[str, dict[str, int | float]] = {}
    for name, events in _corpora().items():
        inputs.clear_array_cache()
        started = time.perf_counter_ns()
        inputs.prewarm_input_arrays(set(events))
        duration_us = (time.perf_counter_ns() - started) // 1_000
        diagnostics = inputs.get_prewarm_diagnostics()
        diagnostic_slots = diagnostics["total_input_slots"]
        assert isinstance(diagnostic_slots, int)
        entries = len(inputs._ARRAY_CACHE)
        slots = sum(len(value) for value in inputs._ARRAY_CACHE.values())
        budgeted = _budget_selection(events, slot_budget=2_048)
        results[name] = {
            "event_count": len(events),
            "unique_shapes": len(set(events)),
            "prewarm_duration_us": duration_us,
            "diagnostic_slots": diagnostic_slots,
            "cache_entries_after_cap": entries,
            "cache_slots_after_cap": slots,
            "budgeted_entries_2048_slots": budgeted[0],
            "budgeted_slots_2048": budgeted[1],
            "budgeted_event_misses_2048": budgeted[2],
        }
        inputs.clear_array_cache()
    return results


def _seed_estimator(down: int, up: int) -> SendLatencyEstimator:
    estimator = SendLatencyEstimator(max_poly=1)
    for _ in range(5):
        estimator.update(ActionKind.DOWN, down)
        estimator.update(ActionKind.UP, up)
    return estimator


def _lead_errors(estimator: SendLatencyEstimator, down: int, up: int) -> list[int]:
    errors: list[int] = []
    for _ in range(10):
        errors.append(down - estimator.get_lead_us(ActionKind.DOWN))
        errors.append(up - estimator.get_lead_us(ActionKind.UP))
        estimator.update(ActionKind.DOWN, down)
        estimator.update(ActionKind.UP, up)
    return errors


def _percentile(values: list[int], fraction: float) -> int:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(fraction * len(ordered)))]


def run_lead_cache_benchmark() -> dict[str, Any]:
    contexts = {
        "ac_high_priority": (800, 400),
        "battery_fallback_priority": (1_600, 900),
        "timer_fallback": (1_200, 700),
        "low_latency_power_state": (200, 100),
    }
    results: dict[str, Any] = {}
    source = _seed_estimator(*contexts["ac_high_priority"])
    serialized = source.export_state()
    for name, (down, up) in contexts.items():
        stale = SendLatencyEstimator(max_poly=1)
        assert stale.import_state(serialized)
        stale_errors = _lead_errors(stale, down, up)
        cold_errors = _lead_errors(SendLatencyEstimator(max_poly=1), down, up)
        results[name] = {
            "stale_p50_abs_error_us": _percentile([abs(v) for v in stale_errors], 0.50),
            "stale_p99_abs_error_us": _percentile([abs(v) for v in stale_errors], 0.99),
            "stale_max_abs_error_us": max(abs(v) for v in stale_errors),
            "cold_p50_abs_error_us": _percentile([abs(v) for v in cold_errors], 0.50),
            "cold_p99_abs_error_us": _percentile([abs(v) for v in cold_errors], 0.99),
            "cold_max_abs_error_us": max(abs(v) for v in cold_errors),
            "stale_is_worse": sum(abs(v) for v in stale_errors) > sum(abs(v) for v in cold_errors),
        }
    return results


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--telemetry-mode", choices=("off", "summary", "full"))
    args = parser.parse_args()
    telemetry = (
        {args.telemetry_mode: run_single_telemetry_mode(args.telemetry_mode)}
        if args.telemetry_mode is not None
        else run_telemetry_benchmark()
    )
    payload: dict[str, Any] = {
        "python": sys.version,
        "gil_disabled": getattr(sys, "_is_gil_enabled", lambda: True)() is False,
        "telemetry_100k": telemetry,
    }
    if args.telemetry_mode is None:
        payload["prewarm_corpora"] = run_prewarm_benchmark()
        payload["lead_cache_synthetic_cross_mode"] = run_lead_cache_benchmark()
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
