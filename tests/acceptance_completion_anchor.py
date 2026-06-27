"""Acceptance gate for the completion-anchor refactor (docs/completion-anchor-refactor-plan.md).

Reviewer-owned. Independent of the implementation's own telemetry: it pairs raw backend
down/up COMPLETION times and checks the game-observed hold directly. Adversarial latency
(down slow, up fast) is the worst case for the start-anchor leak.

Run: uv run python tests/acceptance_completion_anchor.py
Exit code 0 = PASS. Non-zero = FAIL (lists offending songs).
"""
from __future__ import annotations

import glob
import sys
from collections import defaultdict, deque
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from test_runtime_dispatch import (
    FakeClock,
    FakeSleeper,
    TimedBackend,
    TimedCall,
)

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PlaybackEngine

# Worst-case adversarial host latency: the down injection is slow, the up is fast.
# Under start-anchor this drives observed hold = min_hold + up - down = min_hold - 230 (all below
# frame). Under completion-anchor observed = min_hold + up >= frame (all pass).
DOWN_SEND_US = 250
UP_SEND_US = 20


class AsymmetricBackend(TimedBackend):
    def _finish(self, kind: str, scan_codes: tuple[int, ...]) -> None:
        started = self.clock.time_us
        self.clock.time_us += DOWN_SEND_US if kind == "down" else UP_SEND_US
        self.calls.append(TimedCall(kind, scan_codes, started, self.clock.time_us))


def run_song(path: str, fps: int) -> tuple[int, int, int]:
    """Return (feasible_downs, below_frame, dropped) for one song at one fps."""
    song = parse_song_file(Path(path))
    if not song.notes:
        return (0, 0, 0)
    pol = FrameTimingPolicy.local_precise(fps=fps)
    frame = int(pol.frame_us)
    sched = build_key_actions(song, policy=pol, tempo_scale=1.0)
    clock = FakeClock()
    be = AsymmetricBackend(clock)
    eng = PlaybackEngine(
        song=song,
        actions=sched.actions,
        backend=be,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=int(pol.min_hold_us),
        fps=fps,
        telemetry_enabled=True,
    )
    eng.play()
    summary = eng.telemetry.get_summary() or {}
    dropped = int(summary.get("runtime_conflict_dropped_down_count", 0))

    downq: dict[int, deque[int]] = defaultdict(deque)
    below = 0
    feasible = 0
    for c in be.calls:
        for sc in c.scan_codes:
            if c.kind == "down":
                downq[sc].append(c.completed_us)
            elif downq[sc]:
                feasible += 1
                if c.completed_us - downq[sc].popleft() < frame:
                    below += 1
    return (feasible, below, dropped)


def main() -> int:
    # Windows consoles default to cp1252; song filenames contain non-Latin-1 chars (e.g. Vietnamese).
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
    except Exception:
        pass
    files = sorted(
        glob.glob("songs/*.json") + glob.glob("songs/*.skysheet") + glob.glob("songs/*.txt")
    )
    failures: list[str] = []
    tot_feasible = tot_below = tot_dropped = 0
    for fps in (144, 60):
        for fp in files:
            name = Path(fp).name
            # Synthetic floor probes intentionally sit at/below the feasibility cutoff; skip them —
            # this gate is about real production content playing cleanly.
            if name.startswith("TEST_repeat_floor"):
                continue
            try:
                feasible, below, dropped = run_song(fp, fps)
            except Exception as e:  # pragma: no cover - surfaced as a failure
                failures.append(f"[{fps}] {name}: ERROR {e}")
                continue
            tot_feasible += feasible
            tot_below += below
            tot_dropped += dropped
            if below or dropped:
                failures.append(
                    f"[{fps}] {name}: below_frame={below}/{feasible} dropped={dropped}"
                )

    print(f"songs scanned: {len(files)} x2 fps  feasible_notes={tot_feasible}")
    print(f"observed_hold below 1 frame: {tot_below}   conflict_dropped: {tot_dropped}")
    if failures:
        print(f"\nFAIL ({len(failures)} song/fps cases):")
        for line in failures[:40]:
            print("  " + line)
        if len(failures) > 40:
            print(f"  ... +{len(failures) - 40} more")
        return 1
    print("\nPASS — every real song holds >= 1 frame and drops nothing under adversarial latency.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
