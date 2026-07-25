"""Deterministic dispatch-fidelity microbenchmark for plan §16.3.

Measures structural-timing fidelity (zero cumulative drift, IOI preserved
end-to-end) and the per-batch Python CPU budget of the dispatch path driven
by a deterministic fake clock.  Outputs raw samples, median, p95, p99, max
so the acceptance reviewer can compare before/after distributions.

Usage:
    uv run python scripts/bench_dispatch_fidelity.py [repeats] [song.json]

Generates no game input and no real SendInput — it measures the orchestration
core alone (loop + coordinator + fake backend). The numbers therefore
isolate Python-level dispatch cost and structural fidelity, leaving absolute
OS-scheduler jitter to a separate live-OS study.
"""
from __future__ import annotations

import statistics
import sys
import time
from pathlib import Path

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.layouts import SKY_15_KEY_PROFILE
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


class FakeClock:
    def __init__(self, start_us: int = 0):
        self.time_us = start_us

    def now_us(self) -> int:
        return self.time_us


class FakeSleeper:
    def __init__(self, clock: FakeClock):
        self.clock = clock
        self.is_high_resolution = False
        self.handle = None

    def sleep(self, seconds: float) -> None:
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class _NullFocusSignal:
    def is_active(self) -> bool:
        return True


class _NullFocusGuard:
    def is_active(self) -> bool:
        return True

    def focus(self) -> bool:
        return True


class _NullCommandSource:
    def poll(self) -> str | None:
        return None


class _NullProgressSink:
    def publish(self, **kwargs) -> None:
        pass

    def finish(self, message: str) -> None:
        pass

    # Pre-refactor ``run()`` may also call ``update_counters`` inline per
    # dispatch; define as a no-op so the null sink supports both pre- and
    # post-refactor code paths.
    def update_counters(self, lateness_us: int, kind: str = "down") -> None:
        pass


class _NoOpWaitStrategy:
    """Drives the fake clock straight to the deadline without sleeping."""

    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock

    def spin_until_us(self, target_system_us: int, clock: FakeClock) -> None:
        self.clock.time_us = max(self.clock.time_us, target_system_us)

    def wait_until_us(
        self,
        target_system_us: int,
        clock: FakeClock,
        sleeper,
        spin_threshold_us: int,
        policy,
        command_event=None,
    ) -> bool:
        # Fast-forward the fake clock to (target - guard) so the loop ends
        # the wait window immediately, then let the spin_until_us call do
        # the final advance.
        if target_system_us > self.clock.time_us:
            self.clock.time_us = target_system_us
        return False


def _stats(values: list[float]) -> dict:
    if not values:
        return {"min": 0.0, "median": 0.0, "p95": 0.0, "p99": 0.0, "max": 0.0, "mean": 0.0}
    s = sorted(values)
    n = len(s)
    return {
        "min": s[0],
        "median": statistics.median(s),
        "p95": s[min(n - 1, round(0.95 * (n - 1)))],
        "p99": s[min(n - 1, round(0.99 * (n - 1)))],
        "max": s[-1],
        "mean": statistics.fmean(s),
    }


def _run_one(song, policy: FrameTimingPolicy) -> tuple[float, float, list[int]]:
    """Run a full playback through the orchestration core.

    Returns (wall_seconds, thread_cpu_seconds, visible_lateness_samples_us).
    """
    m = build_key_actions(song, policy=policy)
    intents = compile_runtime_intents(m.actions)
    coord = RuntimeDispatchCoordinator(intents, int(policy.min_hold_us))
    backend = DryRunBackend()
    clock = FakeClock(0)
    sleeper = FakeSleeper(clock)
    telemetry = TelemetryLogger("bench", enabled=True, retain_records_after_save=True)
    health = DispatchHealthMonitor(
        backend, clock, _NullFocusGuard(), require_focus=False,
    )
    loop = DispatchLoop(
        coordinator=coord, clock=clock, sleeper=sleeper,
        wait_strategy=_NoOpWaitStrategy(clock),
        backend=backend, telemetry=telemetry,
        sleep_policy=SleepPolicy(poll_s=0.002),
        health_monitor=health,
        min_hold_us=int(policy.min_hold_us),
        spin_threshold_us=1_000,
    )
    # Drive the loop directly via run() with the full supervisor seam mocked.
    state = PlaybackState(start_perf=clock.now_us())
    t0 = time.perf_counter()
    cpu0 = time.thread_time_ns()
    loop.run(
        state, _NullCommandSource(), _NullFocusSignal(), _NullProgressSink(),
        total_time_us=int(m.playback_duration_us),
    )
    wall = time.perf_counter() - t0
    thread_cpu = (time.thread_time_ns() - cpu0) / 1_000_000_000
    lateness_us = [
        getattr(r, "visible_lateness_us", 0)
        for r in telemetry.records
        if r.kind == "down"
    ]
    return wall, thread_cpu, lateness_us


def main() -> int:
    repeats = int(sys.argv[1]) if len(sys.argv) > 1 and sys.argv[1].isdigit() else 5
    song_path = (
        Path(sys.argv[2]) if len(sys.argv) > 2 and not sys.argv[2].isdigit()
        else Path("songs/Arthur Warrell - We Wish You A Merry Christmas.json")
    )
    if not song_path.exists():
        print(f"song not found: {song_path}")
        return 1

    profile = SKY_15_KEY_PROFILE
    policy = FrameTimingPolicy.balanced(fps=60)
    song = parse_song_file(song_path, profile)
    print(f"Song: {song.name}   notes={len(song.notes)}")
    print(f"Profile: balanced @60fps   min_hold_us={int(policy.min_hold_us)}")
    print(f"Repeats: {repeats}")
    print("=" * 72)

    wall_samples: list[float] = []
    cpu_samples: list[float] = []
    lateness_all: list[int] = []
    drift_all: list[int] = []
    for i in range(repeats):
        wall, thread_cpu, lat = _run_one(song, policy)
        wall_samples.append(wall)
        cpu_samples.append(thread_cpu)
        lateness_all.extend(lat)
        # Structural-fidelity check: each down record's lateness drift is its
        # cumulative timing error. Drift = max - min.
        if lat:
            drift_all.append(max(lat) - min(lat))
        print(
            f"  run {i + 1}: wall={wall * 1000:7.2f} ms  "
            f"thread_cpu={thread_cpu * 1000:7.2f} ms  "
            f"n_down={len(lat):4d}  "
            f"lat_med={statistics.median(lat):7.1f} us  "
            f"lat_max={max(lat):7.1f} us"
        )

    print("=" * 72)
    print("Wall-clock (ms):")
    wall_ms = [s * 1000 for s in wall_samples]
    print(f"  raw={wall_ms}")
    wstats = _stats(wall_ms)
    print(
        f"  min={wstats['min']:.2f}  median={wstats['median']:.2f}  "
        f"p95={wstats['p95']:.2f}  p99={wstats['p99']:.2f}  max={wstats['max']:.2f}  "
        f"mean={wstats['mean']:.2f}"
    )
    print("Thread CPU time (ms, direct benchmark thread):")
    print([round(sample * 1000, 3) for sample in cpu_samples])
    print("Visible lateness (us):")
    lstats = _stats([float(v) for v in lateness_all])
    print(
        f"  n={len(lateness_all)}  min={lstats['min']:.1f}  median={lstats['median']:.1f}  "
        f"p95={lstats['p95']:.1f}  p99={lstats['p99']:.1f}  max={lstats['max']:.1f}"
    )
    print(f"Cumulative drift per run (us):  raw={drift_all}")
    if drift_all:
        dstats = _stats([float(v) for v in drift_all])
        print(
            f"  min={dstats['min']:.1f}  median={dstats['median']:.1f}  "
            f"p95={dstats['p95']:.1f}  p99={dstats['p99']:.1f}  max={dstats['max']:.1f}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
