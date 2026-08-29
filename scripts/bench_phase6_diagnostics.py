"""Measure the Phase 6 diagnostics observer on the existing UI snapshot path.

This is intentionally a test-backend benchmark: it exercises the exact
``_SnapshotRenderer`` -> ``DesktopDiagnosticsService`` integration without
starting a game or emitting physical input. Native scheduler qualification is
kept in the unchanged Rust regression suite.
"""

from __future__ import annotations

import argparse
import json
import statistics
import time
from typing import Any

from sky_music.orchestration.desktop_diagnostics import DesktopDiagnosticsService
from sky_music.orchestration.desktop_playback import _SnapshotRenderer
from sky_music.orchestration.native_models import BackendHealth, ProgressCounters


def _percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, round(fraction * (len(ordered) - 1)))
    return ordered[index]


def _run(*, enabled: bool, iterations: int, repeats: int) -> dict[str, Any]:
    measurements: list[float] = []
    emitted = 0
    for _ in range(repeats):
        clock = [0.0]
        published: list[tuple[str, dict[str, object]]] = []
        diagnostics = DesktopDiagnosticsService(
            publish_event=lambda name, payload, sink=published: sink.append(
                (name, payload)
            ),
            clock=lambda current_clock=clock: current_clock[0],
        )
        if enabled:
            diagnostics.set_enabled(True)
        renderer = _SnapshotRenderer(
            lambda _name, _payload: None,
            "a" * 32,
            "b" * 32,
            "Benchmark",
            diagnostics=diagnostics,
        )
        counters = ProgressCounters(120, 0, 0, 0, 0, 0, (100, 200, 400, 800))
        backend = BackendHealth(0, 0, 0, None)
        started = time.perf_counter_ns()
        for index in range(iterations):
            # Keep each synthetic UI interval just above the 100 ms gate so
            # floating-point representation cannot accidentally coalesce a
            # sample that the benchmark intended to consume.
            clock[0] = index * 0.101
            renderer.update_counters_batch(counters)
            renderer.render(
                index / 10_000,
                1.0,
                "Benchmark",
                status="playing",
                backend_health=backend,
            )
        elapsed_us = (time.perf_counter_ns() - started) / 1_000
        measurements.append(elapsed_us / iterations)
        emitted += len(published)
    return {
        "enabled": enabled,
        "iterations_per_repeat": iterations,
        "repeats": repeats,
        "observer_wall_us_per_snapshot": {
            "p50": _percentile(measurements, 0.5),
            "p95": _percentile(measurements, 0.95),
            "max": max(measurements),
            "mean": statistics.fmean(measurements),
        },
        "diagnostics_events_emitted": emitted,
        "test_backend": True,
        "physical_input": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=2_000)
    parser.add_argument("--repeats", type=int, default=9)
    args = parser.parse_args()
    if args.iterations < 100 or args.repeats < 3:
        parser.error("iterations must be >=100 and repeats must be >=3")
    off = _run(enabled=False, iterations=args.iterations, repeats=args.repeats)
    on = _run(enabled=True, iterations=args.iterations, repeats=args.repeats)
    delta = {
        key: on["observer_wall_us_per_snapshot"][key]
        - off["observer_wall_us_per_snapshot"][key]
        for key in ("p50", "p95", "max", "mean")
    }
    print(
        json.dumps(
            {"diagnostics_off": off, "diagnostics_on": on, "delta_us": delta}, indent=2
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
