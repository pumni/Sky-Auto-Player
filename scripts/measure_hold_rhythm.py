"""Sweep hold (min_hold cố định 1 frame) và đo phân bố lateness của ONSET (down).

H-CONTENTION đúng  <=> p95/p99 lateness của down tăng đơn điệu theo hold.
H-CONTENTION sai   <=> lateness của down phẳng theo hold (=> nguyên nhân ở phía game).

Backend mô phỏng độ trễ SendInput theo phân bố telemetry thật (p50~477us, p99~953us)
để đường đơn luồng có chi phí gửi giống thực tế. KHÔNG phụ thuộc game/window.
"""
from __future__ import annotations

import math
import random
import statistics
import sys
from pathlib import Path

from sky_music.domain.domain import Millis, Note, NoteKey, Song
from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy
from sky_music.infrastructure.backend import (
    BackendHealth,
    ReleaseAllOutcome,
    _TrackedKeyState,
)
from sky_music.infrastructure.timing import (
    Clock,
    PerfCounterClock,
    RealSleeper,
    SleepPolicy,
)
from sky_music.layouts import SKY_15_KEY_PROFILE
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.playback_supervisor import PLAYBACK_FINISHED

FPS = 60
FRAME_US = math.ceil(1_000_000 / FPS)
HOLD_FRAMES_SWEEP = [1.0, 1.25, 1.5, 2.0]
TRUNCATE_US = 30_000_000  # 30s đầu mỗi bài cho nhanh; tăng nếu cần độ phủ.
SEED = 12345  # cố định để các lần chạy so sánh được

class FakeClock:
    def __init__(self, start_us: int = 0):
        self.time_us = start_us
    def now_us(self) -> int:
        return self.time_us
    def sleep_us(self, duration_us: int):
        self.time_us += duration_us

class FakeSleeper:
    def __init__(self, clock: FakeClock):
        self.clock = clock
    def sleep(self, seconds: float):
        advance = max(1, int(seconds * 1_000_000))
        self.clock.sleep_us(advance)
    def sleep_us(self, duration_us: int):
        self.clock.sleep_us(max(1, duration_us))

class AdvancingReadClock(FakeClock):
    """Simulation clock that advances during busy-wait clock reads."""
    def __init__(self, start_us: int = 0, read_step_us: int = 10):
        super().__init__(start_us)
        self.read_step_us = read_step_us

    def now_us(self) -> int:
        current_us = self.time_us
        self.time_us += self.read_step_us
        return current_us


class SyntheticLatencyBackend(_TrackedKeyState):
    __slots__ = ("clock", "rng")

    def __init__(self, clock: Clock, rng: random.Random) -> None:
        super().__init__()
        self.clock = clock
        self.rng = rng

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=self.last_error,
        )

    def _emit(self, scan_codes: tuple[int, ...], *, key_up: bool) -> int | None:
        r = self.rng.random()
        if r < 0.50:
            duration_us = 477
        elif r < 0.99:
            duration_us = int(self.rng.uniform(477, 953))
        elif r < 0.999:
            duration_us = int(self.rng.uniform(953, 1300))
        else:
            duration_us = int(self.rng.uniform(1300, 1695))
        t0 = self.clock.now_us()
        while self.clock.now_us() - t0 < duration_us:
            pass
        return self.clock.now_us()

    def release_all(self) -> ReleaseAllOutcome:
        to_release = self.active_keys | self.possibly_active_keys | self.failed_release_keys
        release_tuple = tuple(sorted(to_release))
        self.active_keys.clear()
        self.possibly_active_keys.clear()
        self.failed_release_keys.clear()
        return ReleaseAllOutcome(
            attempted=release_tuple,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )


def policy_for(hold_frames: float) -> FrameTimingPolicy:
    base = TimingPolicy.from_dict({
        "min_hold_frames": 1.0,
        "min_hold_unframed_us": 22_000,
        "hold_frames": hold_frames,
    })
    return FrameTimingPolicy.from_timing_policy(base, fps=FPS)


def pct(values: list[int], p: float) -> float:
    if not values:
        return 0.0
    s = sorted(values)
    return float(s[round(p * (len(s) - 1))])


def run_one_song(song: Song, hold_frames: float, real_mode: bool = False) -> dict[str, float]:
    rng = random.Random(SEED)
    policy = policy_for(hold_frames)
    sched = build_key_actions(song, policy=policy)
    
    truncate_us = 3_000_000 if real_mode else TRUNCATE_US
    actions = tuple(a for a in sched.actions if int(a.at_us) <= truncate_us)

    if real_mode:
        clock = PerfCounterClock()
        sleeper = RealSleeper()
        sleep_policy = SleepPolicy()
    else:
        clock = AdvancingReadClock()
        sleeper = FakeSleeper(clock)
        sleep_policy = SleepPolicy(spin_threshold_us=-1)

    backend = SyntheticLatencyBackend(clock, rng)
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        sleep_policy=sleep_policy,
        use_dispatch_thread=True,
        enable_adaptive_lead=True,
    )
    res = engine.play()
    if res != PLAYBACK_FINISHED:
        raise RuntimeError(f"{song.name}: playback code {res}")

    recs = engine.telemetry.records
    down_lat = [
        int(r.lateness_us)
        for r in recs
        if r.kind == "down" and r.runtime_outcome != "deferred_release"
    ]
    return {
        "down_n": len(down_lat),
        "down_lat_p50": pct(down_lat, 0.50),
        "down_lat_p95": pct(down_lat, 0.95),
        "down_lat_p99": pct(down_lat, 0.99),
        "down_lat_max": float(max(down_lat, default=0)),
        "down_lat_std": float(statistics.pstdev(down_lat)) if len(down_lat) > 1 else 0.0,
    }


def dense_alt(n: int = 400, ioi_ms: int = 18) -> Song:
    keys = ["Key0", "Key1", "Key2", "Key3", "Key4"]
    notes = tuple(
        Note(time_ms=Millis(i * ioi_ms), key=NoteKey(keys[i % len(keys)]))
        for i in range(n)
    )
    return Song(name="DENSE-ALT", notes=notes)


def sparse(n: int = 400, ioi_ms: int = 200) -> Song:
    keys = ["Key0", "Key1", "Key2", "Key3", "Key4"]
    notes = tuple(
        Note(time_ms=Millis(i * ioi_ms), key=NoteKey(keys[i % len(keys)]))
        for i in range(n)
    )
    return Song(name="SPARSE", notes=notes)


def main() -> None:
    try:
        sys.stdout.reconfigure(encoding='utf-8')  # type: ignore
    except Exception:
        pass
    args = sys.argv[1:]
    real_mode = False
    if "--real" in args:
        real_mode = True
        args.remove("--real")

    songs_paths = [Path(p) for p in args]
    if not songs_paths:
        # Default to 5 representative real songs if none specified
        songs_paths = sorted(Path("songs").glob("*.json"))[:5]

    truncate_desc = "3s" if real_mode else "30s"
    mode_desc = "REAL TIMING" if real_mode else "SIMULATION"
    print(f"MODE={mode_desc}  FPS={FPS} frame_us={FRAME_US}  TRUNCATE={truncate_desc}  seed={SEED}")
    print("Interpretation: p95/p99 down-lateness INCREASING with hold => H-CONTENTION holds.\n")
    
    # Run Phase 1 on Real Songs
    print("=== PHASE 1: REAL SONGS ===")
    for sp in songs_paths:
        print(f"== {sp.name} ==")
        header = f"{'hold(f)':>8} {'down_n':>7} {'p50':>8} {'p95':>8} {'p99':>8} {'max':>8} {'std':>8}"
        print(header)
        for hf in HOLD_FRAMES_SWEEP:
            try:
                song = parse_song_file(sp, SKY_15_KEY_PROFILE)
                s = run_one_song(song, hf, real_mode=real_mode)
            except Exception as e:
                print(f"  [skip] hold={hf}: {e}")
                continue
            print(
                f"{hf:>8} {int(s['down_n']):>7} {s['down_lat_p50']:>8.0f} "
                f"{s['down_lat_p95']:>8.0f} {s['down_lat_p99']:>8.0f} "
                f"{s['down_lat_max']:>8.0f} {s['down_lat_std']:>8.0f}"
            )
        print()

    # Run Phase 2 on Synthetic Songs
    print("=== PHASE 2: SYNTHETIC CONTROLLED BENCHMARKS ===")
    synth_songs = [dense_alt(), sparse()]
    for song in synth_songs:
        print(f"== {song.name} ==")
        header = f"{'hold(f)':>8} {'down_n':>7} {'p50':>8} {'p95':>8} {'p99':>8} {'max':>8} {'std':>8}"
        print(header)
        for hf in HOLD_FRAMES_SWEEP:
            try:
                s = run_one_song(song, hf, real_mode=real_mode)
            except Exception as e:
                print(f"  [skip] hold={hf}: {e}")
                continue
            print(
                f"{hf:>8} {int(s['down_n']):>7} {s['down_lat_p50']:>8.0f} "
                f"{s['down_lat_p95']:>8.0f} {s['down_lat_p99']:>8.0f} "
                f"{s['down_lat_max']:>8.0f} {s['down_lat_std']:>8.0f}"
            )
        print()


if __name__ == "__main__":
    main()
