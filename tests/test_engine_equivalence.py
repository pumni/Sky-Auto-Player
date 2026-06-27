from __future__ import annotations

import json
from pathlib import Path

from sky_music.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import (
    BackendHealth,
    InputSendResult,
    ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PLAYBACK_FINISHED, PlaybackEngine


class FakeClock:
    def __init__(self) -> None:
        self.time_us = 0

    def now_us(self) -> int:
        return self.time_us


class GoldenSleeper:
    def __init__(self, clock: FakeClock, stall_at_us: int = 140_000, stall_duration_us: int = 200_000) -> None:
        self.clock = clock
        self.stall_at_us = stall_at_us
        self.stall_duration_us = stall_duration_us
        self.stalled = False

    def sleep(self, seconds: float) -> None:
        if not self.stalled and self.clock.time_us >= self.stall_at_us:
            self.clock.time_us += self.stall_duration_us
            self.stalled = True
            return
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class GoldenControls:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock
        self.paused_called = False
        self.resume_called = False

    def poll(self) -> str | None:
        if not self.paused_called and self.clock.time_us >= 110_000:
            self.paused_called = True
            return "pause"
        if self.paused_called and not self.resume_called and self.clock.time_us >= 130_000:
            self.resume_called = True
            return "pause"
        return None


class TimedCall:
    def __init__(self, kind: str, scan_codes: tuple[int, ...], started_us: int, completed_us: int) -> None:
        self.kind = kind
        self.scan_codes = scan_codes
        self.started_us = started_us
        self.completed_us = completed_us


class TimedBackend:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock
        self.active: set[int] = set()
        self.calls: list[TimedCall] = []

    def _finish(self, kind: str, scan_codes: tuple[int, ...]) -> None:
        started_us = self.clock.time_us
        # Simulate small send duration of 100 us
        self.clock.time_us += 100
        self.calls.append(TimedCall(kind, scan_codes, started_us, self.clock.time_us))

    def key_down(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(scan_code for scan_code in scan_codes if scan_code not in self.active)
        skipped = tuple(scan_code for scan_code in scan_codes if scan_code in self.active)
        if sent:
            self._finish("down", sent)
            self.active.update(sent)
        return InputSendResult(sent=sent, skipped_duplicates=skipped, success=True)

    def key_up(self, scan_codes: tuple[int, ...]) -> InputSendResult:
        sent = tuple(scan_code for scan_code in scan_codes if scan_code in self.active)
        skipped = tuple(scan_code for scan_code in scan_codes if scan_code not in self.active)
        if sent:
            self._finish("up", sent)
            self.active.difference_update(sent)
        return InputSendResult(sent=sent, skipped_duplicates=skipped, success=True)

    def release_all(self) -> ReleaseAllOutcome:
        attempted = tuple(sorted(self.active))
        if attempted:
            self._finish("release_all", attempted)
        self.active.clear()
        return ReleaseAllOutcome(
            attempted=attempted,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active),
            possibly_active_count=0,
            failed_release_count=0,
            last_error=None,
        )

    def get_send_diagnostics(self) -> dict[str, int]:
        return {}


def action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(scan_code) for scan_code in scan_codes),
        at_us=Microseconds(at_us),
        reason="test-equivalence",
    )


def test_playback_equivalence() -> None:
    actions = (
        # 1. Chord
        action(10_000, "down", 21, 22, 23),
        action(20_000, "up", 21, 22, 23),
        
        # 2. Conflict (key 21 down at 30,000, down again at 35,000 before up)
        action(30_000, "down", 21),
        action(35_000, "down", 21),
        action(40_000, "up", 21),
        
        # 3. Deferred release (down at 50,000, up at 55,000 with min_hold_us=10,000)
        action(50_000, "down", 22),
        action(55_000, "up", 22),
        
        # 4. Same-key repeats at/above floor (down at 70,000, up at 80,000, down again at 95,000)
        action(70_000, "down", 23),
        action(80_000, "up", 23),
        action(95_000, "down", 23),
        action(105_000, "up", 23),
        
        # 5. Late burst (down at 150,000, up at 160,000, down at 400,000)
        action(150_000, "down", 21),
        action(160_000, "up", 21),
        action(400_000, "down", 22),
        action(410_000, "up", 22),
    )

    song = Song(name="golden-synthetic", notes=())
    clock = FakeClock()
    backend = TimedBackend(clock)
    sleeper = GoldenSleeper(clock)
    controls = GoldenControls(clock)

    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        controls=controls,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=sleeper,
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        use_dispatch_thread=False,
    )
    # spin_threshold_us=-1 keeps the loop on the sleep ladder, so the busy-spin path is never
    # entered and the fake sleeper alone advances time.
    res = engine.play()
    assert res == PLAYBACK_FINISHED

    # Record the timeline
    timeline = [
        {
            "kind": call.kind,
            "scan_codes": list(call.scan_codes),
            "started_us": call.started_us,
            "completed_us": call.completed_us,
        }
        for call in backend.calls
    ]

    # Record generation status counts from telemetry
    summary = engine.telemetry.get_summary()
    assert summary is not None
    generation_counts = {
        "cancelled_generation_count": summary.get("cancelled_generation_count", 0),
        "dropped_conflict_count": summary.get("dropped_conflict_count", 0),
        "dropped_backend_count": summary.get("dropped_backend_count", 0),
        "released_count": summary.get("released_count", 0),
    }

    current_data = {
        "calls": timeline,
        "generation_status_counts": generation_counts,
    }

    golden_path = Path(__file__).parent / "golden_engine_timeline.json"
    if not golden_path.exists():
        # Write the golden file if it doesn't exist yet (this establishes our pre-refactor baseline!)
        with open(golden_path, "w") as f:
            json.dump(current_data, f, indent=2)
        assert golden_path.exists()
    else:
        # Compare against the golden file
        with open(golden_path) as f:
            golden_data = json.load(f)
        
        assert current_data == golden_data
