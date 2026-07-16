"""Telemetry summary JSON key-path snapshot — guards invariant I9 (schema stability).

Phase 0 of docs/2026-07_core-dispatch-refactor-and-isolation-plan.md.

Asserts recursive key *paths* (not values) of TelemetryLogger.get_summary() after one
telemetry-enabled fake playback match a checked-in list. New keys may be added in later
phases only when intentionally documented; removing keys without a deprecation note fails this.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from sky_music.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PLAYBACK_FINISHED, PlaybackEngine

SCHEMA_PATH = Path(__file__).parent / "golden_schedules" / "telemetry_summary_schema_v1.json"


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


def _action(at_us: int, kind: str, *scan_codes: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=tuple(ScanCode(sc) for sc in scan_codes),
        at_us=Microseconds(at_us),
        reason="telemetry-schema",
    )


def _recursive_key_paths(obj: Any, prefix: str = "") -> list[str]:
    """Collect dotted key paths for nested dicts (values ignored)."""
    paths: list[str] = []
    if isinstance(obj, dict):
        for key in sorted(obj.keys(), key=str):
            path = f"{prefix}.{key}" if prefix else str(key)
            paths.append(path)
            paths.extend(_recursive_key_paths(obj[key], path))
    return paths


def _run_fake_playback_summary() -> dict[str, Any]:
    clock = FakeClock()
    engine = PlaybackEngine(
        song=Song(name="telemetry-schema", notes=()),
        actions=(
            _action(10_000, "down", 21),
            _action(20_000, "up", 21),
            _action(30_000, "down", 22, 23),
            _action(45_000, "up", 22, 23),
        ),
        backend=DryRunBackend(),
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
    summary = engine.telemetry.get_summary()
    assert summary is not None
    return summary


def test_telemetry_summary_schema_key_paths() -> None:
    assert SCHEMA_PATH.exists(), f"Missing schema snapshot {SCHEMA_PATH}"
    with SCHEMA_PATH.open(encoding="utf-8") as f:
        expected: list[str] = json.load(f)
    summary = _run_fake_playback_summary()
    actual = _recursive_key_paths(summary)
    assert actual == expected, (
        "Telemetry summary key paths drifted from the Phase 0 snapshot (I9). "
        "If intentional, update telemetry_summary_schema_v1.json and document the change."
    )
