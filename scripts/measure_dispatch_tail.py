from __future__ import annotations

import random
import sys
import time
import threading
from pathlib import Path

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.infrastructure.backend import _TrackedKeyState, ReleaseAllOutcome, BackendHealth
from sky_music.infrastructure.timing import PerfCounterClock, RealSleeper, SleepPolicy
from sky_music.layouts import SKY_15_KEY_PROFILE
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.playback_supervisor import PLAYBACK_FINISHED

class SyntheticLatencyBackend(_TrackedKeyState):
    __slots__ = ("clock", "history")

    def __init__(self, clock):
        self.clock = clock
        self.active_keys = set()
        self.possibly_active_keys = set()
        self.failed_release_keys = set()
        self.last_error = None
        self.history = []

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=self.last_error
        )

    def _emit(self, scan_codes: tuple[int, ...], *, key_up: bool) -> int | None:
        self.history.append(("up" if key_up else "down", tuple(sorted(scan_codes))))
        
        # Simulating telemetry distribution: p50 ≈ 477µs, p99 ≈ 953µs, max ≈ 1695µs
        r = random.random()
        if r < 0.50:
            duration_us = 477
        elif r < 0.99:
            duration_us = int(random.uniform(477, 953))
        elif r < 0.999:
            duration_us = int(random.uniform(953, 1300))
        else:
            duration_us = int(random.uniform(1300, 1695))

        # Busy-spin to simulate CPU-blocking SendInput
        t0 = self.clock.now_us()
        while self.clock.now_us() - t0 < duration_us:
            pass

        return self.clock.now_us()

    def release_all(self) -> ReleaseAllOutcome:
        to_release = self.active_keys | self.possibly_active_keys | self.failed_release_keys
        release_tuple = tuple(sorted(to_release))
        if to_release:
            self.history.append(("up", release_tuple))
            self.active_keys.clear()
            self.possibly_active_keys.clear()
            self.failed_release_keys.clear()
        return ReleaseAllOutcome(
            attempted=release_tuple,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False
        )

class UILoadThread(threading.Thread):
    def __init__(self, frequency_hz: float = 60.0):
        super().__init__(name="ui-load-sim", daemon=True)
        self.frequency = frequency_hz
        self.stop_event = threading.Event()

    def run(self):
        interval = 1.0 / self.frequency
        while not self.stop_event.is_set():
            t0 = time.perf_counter()
            d = {}
            for i in range(1000):
                d[f"key_{i}"] = i * 2.0
                _ = f"format_{d[f'key_{i}']}"
            # Yield and sleep to maintain frequency
            time.sleep(max(0, interval - (time.perf_counter() - t0)))

def run_experiment(song_path: Path, use_ui_load: bool, switch_interval_ms: int | None) -> dict[str, float]:
    # Set sys switch interval if requested
    old_switch_interval = sys.getswitchinterval()
    if switch_interval_ms is not None:
        sys.setswitchinterval(switch_interval_ms / 1000.0)

    # Start UI load simulator if requested
    ui_thread = None
    if use_ui_load:
        ui_thread = UILoadThread(frequency_hz=60.0)
        ui_thread.start()

    try:
        profile = SKY_15_KEY_PROFILE
        policy = FrameTimingPolicy.balanced(fps=60)
        song = parse_song_file(song_path, profile)
        sched = build_key_actions(song, policy=policy)
        # Limit to the first 15 seconds to finish the benchmark quickly
        actions = tuple(act for act in sched.actions if act.at_us <= 15_000_000)

        clock = PerfCounterClock()
        backend = SyntheticLatencyBackend(clock)
        sleeper = RealSleeper()

        # Build PlaybackEngine
        engine = PlaybackEngine(
            song=song,
            actions=actions,
            backend=backend,
            telemetry_enabled=True,
            require_focus=False,
            clock=clock,
            sleeper=sleeper,
            sleep_policy=SleepPolicy(),
            use_dispatch_thread=True,
            enable_adaptive_lead=True,
        )

        res = engine.play()
        if res != PLAYBACK_FINISHED:
            raise RuntimeError(f"Playback finished with code {res}")

        summary = engine.telemetry.get_summary()
        assert summary is not None
        lat = summary.get("lateness_us", {})
        vis = summary.get("visible_lateness_us", {})
        disp = summary.get("dispatch_lateness_us", {})

        return {
            "p50_lateness": lat.get("p50_us", 0.0),
            "p95_lateness": lat.get("p95_us", 0.0),
            "p99_lateness": lat.get("p99_us", 0.0),
            "p99.9_lateness": lat.get("p99.9_us", 0.0) if "p99.9_us" in lat else lat.get("p99_us", 0.0),
            "max_lateness": lat.get("max_us", 0.0),
            "p50_visible": vis.get("p50_us", 0.0),
            "p99_visible": vis.get("p99_us", 0.0),
            "max_visible": vis.get("max_us", 0.0),
            "p50_dispatch": disp.get("p50_us", 0.0),
            "p99_dispatch": disp.get("p99_us", 0.0),
            "max_dispatch": disp.get("max_us", 0.0),
        }
    finally:
        if ui_thread is not None:
            ui_thread.stop_event.set()
            ui_thread.join(timeout=1.0)
        sys.setswitchinterval(old_switch_interval)

def main():
    song_path = Path("songs/Renai Circulation.json")
    if not song_path.exists():
        print(f"Error: Song file {song_path} not found.")
        sys.exit(1)

    print("Running dispatch tail latency experiments (truncated to 15s)...")
    print("Matrix configuration:")
    print("1. stock 3.14 (GIL) | UI load off | default switch-interval")
    print("2. stock 3.14 (GIL) | UI load on  | 1ms switch-interval")
    print("3. stock 3.14 (GIL) | UI load on  | 5ms switch-interval")
    print("-" * 80)

    # 1. UI Load Off, default switch interval
    print("\n[Run 1/3] GIL | UI Load: OFF | default switch-interval...")
    r1 = run_experiment(song_path, use_ui_load=False, switch_interval_ms=None)
    
    # 2. UI Load On, 1ms switch interval
    print("\n[Run 2/3] GIL | UI Load: ON  | 1ms switch-interval...")
    r2 = run_experiment(song_path, use_ui_load=True, switch_interval_ms=1)

    # 3. UI Load On, 5ms switch interval
    print("\n[Run 3/3] GIL | UI Load: ON  | 5ms switch-interval...")
    r3 = run_experiment(song_path, use_ui_load=True, switch_interval_ms=5)

    # Print results table
    print("\n" + "=" * 80)
    print(f"{'Metric':<25} | {'Load Off / Def':<15} | {'Load On / 1ms':<15} | {'Load On / 5ms':<15}")
    print("-" * 80)
    for key in r1.keys():
        print(f"{key:<25} | {r1[key]:>13.1f} | {r2[key]:>13.1f} | {r3[key]:>13.1f}")
    print("=" * 80)

if __name__ == "__main__":
    main()
