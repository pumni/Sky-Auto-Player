"""Independent audit benchmark for the song -> SendInput pipeline.

Measures the cost of every *preparation* stage (which must finish before the
playback clock starts) and the per-batch CPU cost + structural timing fidelity
of the dispatch path driven by a deterministic fake clock.

Run:  uv run python scripts/audit_pipeline_bench.py [song.json]

Note: absolute lateness vs OS-scheduler jitter can only be measured on a live
Windows run (real PerfCounterClock + SendInput). This bench instead proves the
*structural* timing invariants deterministically (zero cumulative drift, IOI
preserved end-to-end) and the raw Python CPU budget per dispatch batch.
"""
from __future__ import annotations

import statistics
import sys
import time
from pathlib import Path

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.domain.song_repository import SongRepository
from sky_music.domain.validation import validate_key_actions
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.layouts import SKY_15_KEY_PROFILE
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.runtime_dispatch import compile_runtime_intents

# Reuse the deterministic fakes from the test suite.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tests"))
from test_runtime_dispatch import FakeClock, FakeSleeper, TimedBackend  # noqa: E402


def _bench(fn, repeats: int) -> float:
    best = float("inf")
    for _ in range(repeats):
        t0 = time.perf_counter()
        fn()
        best = min(best, time.perf_counter() - t0)
    return best * 1_000.0  # ms


def main() -> None:
    song_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("songs/Renai Circulation.json")
    profile = SKY_15_KEY_PROFILE
    policy: FrameTimingPolicy = FrameTimingPolicy.balanced(fps=60)

    song = parse_song_file(song_path, profile)
    sched = build_key_actions(song, policy=policy)
    actions = sched.actions

    print(f"Song: {song.name}   notes={len(song.notes)}   actions={len(actions)}")
    print(f"Profile: balanced @60fps  min_hold_us={int(policy.min_hold_us)}  hold_us={int(policy.hold_us)}")
    print("=" * 70)

    # ---- Preparation-stage timings (must all be pre-clock) ----
    repo = SongRepository()
    cold_repo = SongRepository()

    print("Preparation stage (best of N, ms):")
    print(f"  parse_song_file          : {_bench(lambda: parse_song_file(song_path, profile), 5):8.3f}")
    repo.load(song_path, profile)  # warm
    print(f"  repo.load (cached)       : {_bench(lambda: repo.load(song_path, profile), 50):8.3f}")
    print(f"  repo.load (cold/cleared) : {_bench(lambda: (cold_repo.clear(), cold_repo.load(song_path, profile))[1], 5):8.3f}")
    print(f"  build_key_actions        : {_bench(lambda: build_key_actions(song, policy=policy), 5):8.3f}")
    print(f"  validate_key_actions     : {_bench(lambda: validate_key_actions(actions, policy=policy), 10):8.3f}")
    print(f"  compile_runtime_intents  : {_bench(lambda: compile_runtime_intents(actions), 20):8.3f}")
    print("=" * 70)

    # ---- Pure dispatch CPU: drain the coordinator directly (no sleep loop) ----
    # Driving the full engine under a fake clock inflates wall time with modelled
    # sleep-stepping, so it is NOT a pure-CPU figure. Instead time the coordinator's
    # own pop/split/activate/release path at the authored deadlines.
    def drain_coordinator() -> None:
        from sky_music.orchestration.runtime_dispatch import RuntimeDispatchCoordinator
        sched_rt = compile_runtime_intents(actions)
        coord = RuntimeDispatchCoordinator(sched_rt, int(policy.min_hold_us))
        while not coord.is_finished():
            now = coord.next_deadline_us()
            if now is None:
                break
            for rel in coord.pop_due_pending(now):
                coord.complete_releases((rel,), (rel.scan_code,))
            for batch in coord.pop_due_authored(now):
                if batch.kind == "up":
                    coord.request_releases(batch.intents)
                    for rel in coord.pop_due_pending(coord.next_deadline_us() or now):
                        coord.complete_releases((rel,), (rel.scan_code,))
                else:
                    playable, _ = coord.split_down_intents(batch.intents)
                    coord.activate_sent_downs(
                        playable,
                        tuple(i.scan_code for i in playable),
                        dispatch_started_us=now,
                        dispatch_completed_us=now,
                    )

    drain_ms = _bench(drain_coordinator, 5)
    n_batches = len(actions)
    print("Dispatch path — pure coordinator CPU (no sleep modelling):")
    print(f"  full timeline drain      : {drain_ms:8.3f} ms over {n_batches} batches")
    print(f"  avg CPU per batch        : {drain_ms * 1000.0 / n_batches:8.3f} us/batch")

    # ---- Structural fidelity under the full engine (fake clock) ----
    clock = FakeClock()
    backend = TimedBackend(clock, send_duration_us=0)
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=int(policy.min_hold_us),
    )
    engine.play()
    summary = engine.telemetry.get_summary()
    assert summary is not None

    downs = [c for c in backend.calls if c.kind == "down"]
    down_times = [c.started_us for c in downs]
    iois = [b - a for a, b in zip(down_times, down_times[1:])]
    print("Structural fidelity (full engine, fake clock):")
    print(f"  down events dispatched   : {len(downs)}")
    if iois:
        print(f"  down IOI median (whole)  : {statistics.median(iois):.0f} us "
              f"(min={min(iois)}, max={max(iois)}; reflects the SONG's note density, not drift)")
    print(f"  down_timeline_drift_us   : {summary['down_timeline_drift_us']}  (0 == no cumulative slowdown)")

    lat = summary["lateness_us"]
    print(f"  lateness p50/p95/p99/max : "
          f"{lat['p50_us']:.0f} / {lat['p95_us']:.0f} / {lat['p99_us']:.0f} / {lat['max_us']:.0f} us")
    print(f"  late >2/5/10ms           : {lat['over_2ms']} / {lat['over_5ms']} / {lat['over_10ms']}")
    print(f"  runtime conflict drops   : {summary['runtime_conflict_dropped_down_count']}")
    print(f"  confirmed hold shortfall : {summary['confirmed_hold_shortfall_count']}")
    print(f"  sent down/up             : {summary['sent_down_count']} / {summary['sent_up_count']}")
    print("=" * 70)
    print("NOTE: lateness here reflects only fake-clock model error; real OS-scheduler")
    print("jitter requires a live Windows run. Drift==0 and stable IOI confirm the")
    print("absolute-timeline invariant holds structurally (no rebase, no slowdown).")


if __name__ == "__main__":
    main()
