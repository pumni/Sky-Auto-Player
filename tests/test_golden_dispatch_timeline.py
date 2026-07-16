"""Golden dispatch timeline — primary "behavior didn't move" instrument for the core refactor.

Phase 0 of docs/2026-07_core-dispatch-refactor-and-isolation-plan.md.

Uses FakeClock + DryRunBackend (clock-stamped) + direct (non-threaded) mode so later
phases can assert byte-identical backend call histories for three synthetic schedules:
single-note melody, chords with same-key repeats, and chord-stagger enabled.
"""

from __future__ import annotations

import json
from pathlib import Path

from sky_music.domain import Song
from sky_music.domain.scheduler import apply_chord_stagger
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PLAYBACK_FINISHED, PlaybackEngine

GOLDEN_PATH = Path(__file__).parent / "golden_schedules" / "dispatch_timeline_v1.json"


class FakeClock:
    def __init__(self) -> None:
        self.time_us = 0

    def now_us(self) -> int:
        return self.time_us


class FakeSleeper:
    def __init__(self, clock: FakeClock) -> None:
        self.clock = clock

    def sleep(self, seconds: float) -> None:
        self.clock.time_us += max(1, int(seconds * 1_000_000))


class NullControls:
    def poll(self) -> str | None:
        return None


class RecordingDryRunBackend(DryRunBackend):
    """DryRunBackend that stamps each emit with FakeClock.now_us() for golden snapshots.

    Ownership: test-only; single-threaded (direct dispatch mode).
    """

    def __init__(self, clock: FakeClock) -> None:
        super().__init__()
        self.clock = clock
        self.timed_history: list[tuple[str, tuple[int, ...], int]] = []

    def _emit(
        self, scan_codes: tuple[int, ...], *, key_up: bool
    ) -> tuple[tuple[int, ...], int | None]:
        kind = "up" if key_up else "down"
        started = self.clock.now_us()
        sent, completed = super()._emit(scan_codes, key_up=key_up)
        self.timed_history.append((kind, tuple(sorted(scan_codes)), started))
        return sent, completed

    def release_all(self):  # type: ignore[override]
        to_release = self.active_keys | self.possibly_active_keys | self.failed_release_keys
        if to_release:
            release_tuple = tuple(sorted(to_release))
            self.timed_history.append(("up", release_tuple, self.clock.now_us()))
        return super().release_all()


def _action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(sc) for sc in scan_codes),
        at_us=Microseconds(at_us),
        reason="golden-dispatch-timeline",
    )


def _run_schedule(name: str, actions: list[KeyAction] | tuple[KeyAction, ...]) -> dict:
    clock = FakeClock()
    backend = RecordingDryRunBackend(clock)
    engine = PlaybackEngine(
        song=Song(name=name, notes=()),
        actions=tuple(actions),
        backend=backend,
        controls=NullControls(),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        use_dispatch_thread=False,
        dispatch_lead_us=0,
    )
    assert engine.play() == PLAYBACK_FINISHED
    return {
        "calls": [
            {"kind": kind, "scan_codes": list(codes), "started_us": started_us}
            for kind, codes, started_us in backend.timed_history
        ],
        "generation_status_counts": dict(engine.telemetry.generation_status_counts),
    }


def _build_schedules() -> dict[str, dict]:
    melody = [
        _action(10_000, "down", 21),
        _action(20_000, "up", 21),
        _action(30_000, "down", 22),
        _action(40_000, "up", 22),
        _action(50_000, "down", 23),
        _action(60_000, "up", 23),
    ]
    chords = [
        _action(10_000, "down", 21, 22, 23),
        _action(25_000, "up", 21, 22, 23),
        _action(40_000, "down", 21),
        _action(50_000, "up", 21),
        _action(55_000, "down", 21),
        _action(70_000, "up", 21),
        _action(80_000, "down", 22, 24),
        _action(95_000, "up", 22, 24),
    ]
    base_chord = [
        _action(10_000, "down", 21, 22, 23, 24),
        _action(40_000, "up", 21, 22, 23, 24),
        _action(60_000, "down", 21, 22),
        _action(80_000, "up", 21, 22),
    ]
    staggered = apply_chord_stagger(base_chord, 2_000, 15_000)
    return {
        "single_note_melody": _run_schedule("single_note_melody", melody),
        "chords_same_key_repeats": _run_schedule("chords_same_key_repeats", chords),
        "chord_stagger_enabled": _run_schedule("chord_stagger_enabled", staggered),
    }


def _capture_current() -> dict:
    return {"version": 1, "schedules": _build_schedules()}


def test_golden_dispatch_timeline_matches_snapshot() -> None:
    assert GOLDEN_PATH.exists(), (
        f"Missing golden snapshot {GOLDEN_PATH}; regenerate via Phase 0 of the "
        "core-dispatch-refactor plan."
    )
    with GOLDEN_PATH.open(encoding="utf-8") as f:
        expected = json.load(f)
    actual = _capture_current()
    assert actual == expected


def test_golden_dispatch_timeline_is_deterministic() -> None:
    """Run twice back-to-back; both captures must match the committed golden (and each other)."""
    first = _capture_current()
    second = _capture_current()
    assert first == second
    with GOLDEN_PATH.open(encoding="utf-8") as f:
        expected = json.load(f)
    assert first == expected
    assert second == expected
