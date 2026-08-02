"""Comparable Rust/Python dispatch benchmark on one song corpus.

Both implementations receive the same ``build_key_actions`` output, FPS and
timing profile.  The default backend is deliberately a mock: this benchmark
does not emit game input.  It measures dispatch-worker/scheduler cost and OS
wait behavior, not end-to-end ``SendInput`` latency.

Usage::

    uv run python scripts/bench_rust_vs_python.py --repeats 2
    uv run python scripts/bench_rust_vs_python.py --song "songs/All Of Me.json" \
        --fps 60 --profile balanced --repeats 10 --output bench.json
"""
from __future__ import annotations

import argparse
import json
import math
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import PerfCounterClock, RealSleeper, SleepPolicy
from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
from sky_music.layouts import SKY_15_KEY_PROFILE, SKY_15_SCAN_CODES
from sky_music.orchestration.core.coordinator import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)
from sky_music.orchestration.core.loop import (
    DispatchHealthMonitor,
    DispatchLoop,
    PlaybackState,
)
from sky_music.orchestration.telemetry import TelemetryLogger


class _Focus:
    def is_active(self) -> bool:
        return True

    def focus(self) -> bool:
        return True


class _Commands:
    def poll(self) -> None:
        return None


class _Progress:
    def publish(self, **kwargs: object) -> None:
        return None

    def finish(self, message: str) -> None:
        return None


class _CountingWait:
    """Count Python wait returns using the same wait strategy as production."""

    def __init__(self, deadline_ns: int | None = None) -> None:
        self._delegate = HybridWaitStrategy(enable_event_wait=False)
        self.wake_count = 0
        self._deadline_ns = deadline_ns

    def _check_budget(self) -> None:
        if self._deadline_ns is not None and time.perf_counter_ns() >= self._deadline_ns:
            raise TimeoutError("Rust/Python benchmark exceeded its hard budget")

    def spin_until_us(self, target_system_us: int, clock: Any) -> None:
        self._check_budget()
        self._delegate.spin_until_us(target_system_us, clock)

    def wait_until_us(
        self,
        target_system_us: int,
        clock: Any,
        sleeper: Any,
        spin_threshold_us: int,
        policy: Any,
        command_event: int | None = None,
    ) -> bool:
        self._check_budget()
        result = self._delegate.wait_until_us(
            target_system_us,
            clock,
            sleeper,
            spin_threshold_us,
            policy,
            command_event,
        )
        self.wake_count += 1
        return result


@dataclass(frozen=True, slots=True)
class _Sample:
    wall_us: int
    process_cpu_us: int
    send_duration_us: tuple[int, ...]
    visible_lateness_us: tuple[int, ...]
    idle_wake_count: int
    keys_dropped: int
    stuck_key_count: int
    possibly_active_count: int
    failed_release_count: int
    outcome: str | None


def _stats(values: list[int]) -> dict[str, float | int]:
    if not values:
        return {"n": 0, "p50": 0, "p95": 0, "p99": 0, "max": 0}
    ordered = sorted(values)
    last = len(ordered) - 1
    return {
        "n": len(ordered),
        "p50": ordered[round(0.50 * last)],
        "p95": ordered[round(0.95 * last)],
        "p99": ordered[round(0.99 * last)],
        "max": ordered[-1],
    }


def _native_sample(actions: tuple[Any, ...], *, timeout_ms: int = 60_000) -> _Sample:
    import sky_player_rs

    native_actions = [
        (
            index,
            str(action.kind),
            int(action.at_us),
            [int(scan_code) for scan_code in action.scan_codes],
            action.reason,
        )
        for index, action in enumerate(actions)
    ]
    session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        native_actions,
        list(SKY_15_SCAN_CODES),
        profile="mock_test",
        min_hold_us=50_000,
        max_lead_us=2_000,
        require_focus=False,
        telemetry_mode="ring",
        telemetry_capacity=1_024,
        rt_priority_mode="off",
        enable_waitable_timer=True,
        enable_event_wait=False,
        enable_adaptive_spin=False,
        enable_adaptive_lead=False,
    )
    process_cpu_start = time.process_time_ns()
    wall_start = time.perf_counter_ns()
    session.start()
    if not session.join(timeout_ms=timeout_ms):
        session.panic_release()
        session.quit()
        session.join(timeout_ms=5_000)
        raise TimeoutError("Rust benchmark session exceeded its hard budget")
    wall_us = (time.perf_counter_ns() - wall_start) // 1_000
    process_cpu_us = (time.process_time_ns() - process_cpu_start) // 1_000
    snapshot = dict(session.snapshot())
    output = json.loads(session.take_telemetry_json())
    records = output.get("records", [])
    release = snapshot.get("release_outcome") or {}
    return _Sample(
        wall_us=wall_us,
        process_cpu_us=process_cpu_us,
        send_duration_us=tuple(int(r["send_duration_us"]) for r in records),
        visible_lateness_us=tuple(int(r["visible_lateness_us"]) for r in records),
        idle_wake_count=int(snapshot.get("idle_wake_count", 0)),
        keys_dropped=int(snapshot.get("keys_dropped", 0)),
        stuck_key_count=len(release.get("stuck_keys", ())),
        possibly_active_count=int(snapshot.get("possibly_active_count", 0)),
        failed_release_count=int(snapshot.get("failed_release_count", 0)),
        outcome=snapshot.get("outcome"),
    )


def _python_sample(
    actions: tuple[Any, ...], total_us: int, *, deadline_ns: int | None = None
) -> _Sample:
    clock = PerfCounterClock()
    backend = DryRunBackend()
    backend.set_clock(clock)
    telemetry = TelemetryLogger(
        "bench-python",
        enabled=True,
        retain_records_after_save=True,
    )
    health_monitor = DispatchHealthMonitor(
        backend,
        clock,
        _Focus(),
        require_focus=False,
    )
    wait_strategy = _CountingWait(deadline_ns)
    loop = DispatchLoop(
        coordinator=RuntimeDispatchCoordinator(
            compile_runtime_intents(actions),
            50_000,
        ),
        clock=clock,
        sleeper=RealSleeper(),
        wait_strategy=wait_strategy,
        backend=backend,
        telemetry=telemetry,
        sleep_policy=SleepPolicy(poll_s=0.002),
        health_monitor=health_monitor,
        min_hold_us=50_000,
        spin_threshold_us=150,
    )
    process_cpu_start = time.process_time_ns()
    wall_start = time.perf_counter_ns()
    outcome = loop.run(
        PlaybackState(start_perf=clock.now_us()),
        _Commands(),
        _Focus(),
        _Progress(),
        total_time_us=total_us,
    )
    wall_us = (time.perf_counter_ns() - wall_start) // 1_000
    process_cpu_us = (time.process_time_ns() - process_cpu_start) // 1_000
    records = [dict(record.items()) for record in telemetry.records]
    release = telemetry.release_outcome
    health = telemetry.backend_health or backend.get_health()
    return _Sample(
        wall_us=wall_us,
        process_cpu_us=process_cpu_us,
        send_duration_us=tuple(int(r["send_duration_us"]) for r in records),
        visible_lateness_us=tuple(int(r["visible_lateness_us"]) for r in records),
        idle_wake_count=wait_strategy.wake_count,
        keys_dropped=health.keys_dropped,
        stuck_key_count=len(release.stuck_keys) if release is not None else 0,
        possibly_active_count=health.possibly_active_count,
        failed_release_count=health.failed_release_count,
        outcome=outcome,
    )


def _aggregate(samples: list[_Sample]) -> dict[str, Any]:
    return {
        "wall_us": _stats([sample.wall_us for sample in samples]),
        "process_cpu_us": _stats([sample.process_cpu_us for sample in samples]),
        "send_duration_us": _stats(
            [value for sample in samples for value in sample.send_duration_us]
        ),
        "visible_lateness_us": _stats(
            [value for sample in samples for value in sample.visible_lateness_us]
        ),
        "idle_wake_count": _stats([sample.idle_wake_count for sample in samples]),
        "keys_dropped": sum(sample.keys_dropped for sample in samples),
        "stuck_key_count": sum(sample.stuck_key_count for sample in samples),
        "possibly_active_count": sum(sample.possibly_active_count for sample in samples),
        "failed_release_count": sum(sample.failed_release_count for sample in samples),
        "outcomes": sorted({sample.outcome for sample in samples}),
    }


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--song",
        type=Path,
        default=Path("songs/Arthur Warrell - We Wish You A Merry Christmas.json"),
    )
    parser.add_argument("--fps", type=int, default=60)
    parser.add_argument(
        "--profile",
        choices=("local_precise", "balanced", "audience_safe"),
        default="balanced",
    )
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument(
        "--budget-seconds",
        type=float,
        default=120.0,
        help="hard whole-command budget in seconds (1..120; default: 120)",
    )
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    if args.fps <= 0 or args.repeats <= 0:
        raise SystemExit("--fps and --repeats must be positive")
    if (
        isinstance(args.budget_seconds, bool)
        or not math.isfinite(args.budget_seconds)
        or not 1.0 <= args.budget_seconds <= 120.0
    ):
        raise SystemExit("--budget-seconds must be between 1 and 120 seconds")
    if not args.song.is_file():
        raise SystemExit(f"song not found: {args.song}")

    policy = FrameTimingPolicy.from_profile_name(args.profile, fps=args.fps)
    song = parse_song_file(args.song, SKY_15_KEY_PROFILE)
    metadata = build_key_actions(song, policy=policy)
    actions = tuple(metadata.actions)
    print(
        f"corpus={args.song} notes={len(song.notes)} actions={len(actions)} "
        f"duration_us={metadata.playback_duration_us} profile={args.profile} fps={args.fps} "
        f"repeats={args.repeats} backend=mock"
    )

    deadline_ns = time.perf_counter_ns() + int(args.budget_seconds * 1_000_000_000)
    rust_samples = []
    python_samples = []
    for _ in range(args.repeats):
        remaining_ms = (deadline_ns - time.perf_counter_ns()) // 1_000_000 - 5_000
        if remaining_ms <= 0:
            raise TimeoutError("Rust/Python benchmark exceeded its hard budget")
        rust_samples.append(_native_sample(actions, timeout_ms=min(60_000, int(remaining_ms))))
        python_samples.append(
            _python_sample(
                actions,
                int(metadata.playback_duration_us),
                deadline_ns=deadline_ns,
            )
        )
    report: dict[str, Any] = {
        "corpus": str(args.song),
        "notes": len(song.notes),
        "actions": len(actions),
        "duration_us": int(metadata.playback_duration_us),
        "profile": args.profile,
        "fps": args.fps,
        "repeats": args.repeats,
        "budget_seconds": args.budget_seconds,
        "backend": "mock",
        "rust": _aggregate(rust_samples),
        "python": _aggregate(python_samples),
    }
    print(json.dumps(report, indent=2))
    if args.output is not None:
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"report={args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
