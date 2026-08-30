"""Qualify diagnostics ON/OFF on the native playback scheduling path.

This is deliberately different from ``bench_phase6_diagnostics.py``.  That
script measures observer overhead in isolation.  This harness runs the
production ``PlaybackEngine`` and ``RustDispatchRuntime`` against the Rust
``TestDispatchSession`` test-support seam.  The native scheduler and timing
report are real, while the Rust backend is deterministic mock transport and
never calls Windows ``SendInput``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
import sys
import tempfile
from collections import deque
from pathlib import Path
from types import ModuleType, SimpleNamespace
from typing import Any

from sky_music.domain.domain import Microseconds, ScanCode, Song
from sky_music.domain.scheduler_types import ActionKind, KeyAction
from sky_music.orchestration.desktop_diagnostics import DesktopDiagnosticsService
from sky_music.orchestration.desktop_playback import _SnapshotRenderer
from sky_music.orchestration.engine import PlaybackEngine

DEFAULT_NOTES = 16
DEFAULT_REPEATS = 7
MIN_HOLD_US = 20_000
MIN_RELEASE_GAP_US = 20_000
GAME_FPS = 60


def _percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, round(fraction * (len(ordered) - 1)))]


def _stats(values: list[float]) -> dict[str, float]:
    return {
        "p50": _percentile(values, 0.50),
        "p95": _percentile(values, 0.95),
        "max": max(values, default=0.0),
        "mean": statistics.fmean(values) if values else 0.0,
    }


def _actions(note_count: int, scan_code: int) -> tuple[KeyAction, ...]:
    result: list[KeyAction] = []
    for index in range(note_count):
        start_us = index * 40_000
        result.extend(
            (
                KeyAction(
                    ActionKind.DOWN,
                    (ScanCode(scan_code),),
                    Microseconds(start_us),
                    "phase6-native-diagnostics-down",
                ),
                KeyAction(
                    ActionKind.UP,
                    (ScanCode(scan_code),),
                    Microseconds(start_us + MIN_HOLD_US),
                    "phase6-native-diagnostics-up",
                ),
            )
        )
    return tuple(result)


def _fingerprint(actions: tuple[KeyAction, ...]) -> str:
    encoded = json.dumps(
        [
            {
                "kind": str(action.kind),
                "scan_codes": [int(code) for code in action.scan_codes],
                "at_us": int(action.at_us),
            }
            for action in actions
        ],
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _test_support_facade(native: Any) -> ModuleType:
    test_session = getattr(native, "TestDispatchSession", None)
    if not callable(test_session):
        raise RuntimeError(
            "native diagnostics qualification requires a test-support native wheel; "
            "build with scripts/build_rust_wheel.py --test-support"
        )

    facade = ModuleType("sky_player_rs")

    class SessionConfig:
        def __init__(self, **values: object) -> None:
            self.__dict__.update(values)

    def dispatch_session(actions: object, *, config: object) -> object:
        values = vars(config)
        session = test_session(
            list(actions),
            list(native.instrument_scan_codes()),
            min_hold_us=int(values["min_hold_us"]),
            game_fps=int(values["game_fps"]),
            telemetry_capacity=4096,
            rt_priority_mode="off",
            enable_waitable_timer=True,
            enable_event_wait=True,
            enable_adaptive_spin=True,
            enable_dispatch_cost_lead=True,
            min_release_gap_us=int(values["min_release_gap_us"]),
            down_late_grace_us=int(values["down_late_grace_us"]),
            wait_policy="legacy_test_wide_spin",
        )

        class SessionAdapter:
            """Expose the production runtime's snapshot_lite shape."""

            def __getattr__(self, name: str) -> object:
                return getattr(session, name)

            def snapshot_lite(self) -> object:
                raw = dict(session.snapshot())
                raw["max_completion_error_us"] = raw.get("max_lateness_us", 0)
                raw["backend_health"] = SimpleNamespace(
                    active_count=raw.get("active_count", 0),
                    possibly_active_count=raw.get("possibly_active_count", 0),
                    failed_release_count=raw.get("failed_release_count", 0),
                    last_error=raw.get("last_error"),
                    keys_dropped=raw.get("keys_dropped", 0),
                    chord_split_events=raw.get("chord_split_events", 0),
                    sendinput_partial_events=raw.get("sendinput_partial_events", 0),
                    sendinput_zero_progress_failures=raw.get(
                        "sendinput_zero_progress_failures", 0
                    ),
                    chords_rejected=raw.get("chords_rejected", 0),
                    authored_conflict_events=raw.get("authored_conflict_events", 0),
                    authored_chords_rejected=raw.get("authored_chords_rejected", 0),
                    authored_keys_rejected=raw.get("authored_keys_rejected", 0),
                    keys_inserted_before_failure=raw.get(
                        "keys_inserted_before_failure", 0
                    ),
                    keys_rolled_back=raw.get("keys_rolled_back", 0),
                    rollback_residue_keys=raw.get("rollback_residue_keys", 0),
                )
                return SimpleNamespace(**raw)

            def session_report(self) -> dict[str, object]:
                return {
                    "snapshot": dict(session.snapshot()),
                    "effective_config": {},
                    "telemetry_json": session.take_telemetry_json(),
                }

        return SessionAdapter()

    facade.SessionConfig = SessionConfig  # type: ignore[attr-defined]
    facade.DispatchSession = dispatch_session  # type: ignore[attr-defined]
    return facade


def _run_once(
    *,
    actions: tuple[KeyAction, ...],
    enabled: bool,
    temp_dir: Path,
    repeat: int,
) -> dict[str, Any]:
    events: deque[tuple[str, dict[str, object]]] = deque(maxlen=64)
    consumed = 0

    def consume_event(name: str, payload: dict[str, object]) -> None:
        nonlocal consumed
        if name == "diagnostics.snapshot":
            # Materialize the fields the UI consumes so the ON leg exercises
            # production publication and consumption rather than a flag-only
            # path.  The deque remains bounded even if the renderer is changed.
            int(payload["seq"])
            float(payload["p95_ms"])
            consumed += 1
        events.append((name, payload))

    diagnostics = DesktopDiagnosticsService(publish_event=consume_event)
    if enabled:
        diagnostics.set_enabled(True)
    renderer = _SnapshotRenderer(
        lambda _name, _payload: None,
        "a" * 32,
        "b" * 32,
        "Phase 6 native diagnostics benchmark",
        diagnostics=diagnostics,
    )
    engine = PlaybackEngine(
        Song(name="Phase 6 native diagnostics benchmark", notes=()),
        actions,
        min_release_gap_us=MIN_RELEASE_GAP_US,
        renderer=renderer,
        telemetry_enabled=True,
        require_focus=False,
        game_fps=GAME_FPS,
        min_hold_us=MIN_HOLD_US,
        down_late_grace_us=500,
    )
    engine.telemetry.log_filepath = temp_dir / f"native-diagnostics-{repeat}.csv"
    outcome = engine.play()
    summary = engine.telemetry.get_summary() or {}
    snapshot = engine._last_snapshot

    def snapshot_int(name: str) -> int:
        value = snapshot.get(name, 0)
        return int(value) if isinstance(value, int) and not isinstance(value, bool) else 0

    lateness = summary.get("lateness_us", {})
    dispatch_start = summary.get("dispatch_start_error_us", {})
    return {
        "outcome": str(outcome),
        "diagnostics_snapshots_consumed": consumed,
        "diagnostics_event_buffer_peak": len(events),
        "max_lateness_us": snapshot_int("max_lateness_us"),
        "late_2ms": snapshot_int("late_2ms"),
        "late_5ms": snapshot_int("late_5ms"),
        "late_10ms": snapshot_int("late_10ms"),
        "keys_dropped": snapshot_int("keys_dropped"),
        "chord_split_events": snapshot_int("chord_split_events"),
        "dispatch_start_error_us": {
            key: float(dispatch_start.get(key, 0.0))
            for key in ("p50_us", "p95_us", "max_us")
        },
        "completion_lateness_us": {
            key: float(lateness.get(key, 0.0))
            for key in ("p50_us", "p95_us", "max_us")
        },
    }


def _aggregate(runs: list[dict[str, Any]], *, enabled: bool) -> dict[str, Any]:
    return {
        "enabled": enabled,
        "runs": len(runs),
        "all_finished": all(run["outcome"] == "finished" for run in runs),
        "diagnostics_snapshots_consumed": {
            "total": sum(run["diagnostics_snapshots_consumed"] for run in runs),
            "per_run": [run["diagnostics_snapshots_consumed"] for run in runs],
        },
        "native_timing": {
            "max_lateness_us": _stats([run["max_lateness_us"] for run in runs]),
            "dispatch_start_error_us": {
                key: _stats([run["dispatch_start_error_us"][key] for run in runs])
                for key in ("p50_us", "p95_us", "max_us")
            },
            "completion_lateness_us": {
                key: _stats([run["completion_lateness_us"][key] for run in runs])
                for key in ("p50_us", "p95_us", "max_us")
            },
        },
        "late_event_counters": {
            key: {
                "total": sum(run[key] for run in runs),
                "max_per_run": max((run[key] for run in runs), default=0),
            }
            for key in ("late_2ms", "late_5ms", "late_10ms")
        },
        "backend_counters": {
            key: {
                "total": sum(run[key] for run in runs),
                "max_per_run": max((run[key] for run in runs), default=0),
            }
            for key in ("keys_dropped", "chord_split_events")
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--notes", type=int, default=DEFAULT_NOTES)
    parser.add_argument("--repeats", type=int, default=DEFAULT_REPEATS)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.notes < 4 or args.notes > 64:
        parser.error("--notes must be between 4 and 64")
    if args.repeats < 3 or args.repeats > 20:
        parser.error("--repeats must be between 3 and 20")

    import sky_player_rs as native

    scan_code = int(native.instrument_scan_codes()[0])
    actions = _actions(args.notes, scan_code)
    native_build = dict(native.build_info())
    facade = _test_support_facade(native)
    original_module = sys.modules.get("sky_player_rs")
    sys.modules["sky_player_rs"] = facade
    try:
        with tempfile.TemporaryDirectory(prefix="phase6-native-diagnostics-") as raw_dir:
            temp_dir = Path(raw_dir)
            off_runs = [
                _run_once(
                    actions=actions,
                    enabled=False,
                    temp_dir=temp_dir,
                    repeat=index,
                )
                for index in range(args.repeats)
            ]
            on_runs = [
                _run_once(
                    actions=actions,
                    enabled=True,
                    temp_dir=temp_dir,
                    repeat=args.repeats + index,
                )
                for index in range(args.repeats)
            ]
    finally:
        if original_module is None:
            del sys.modules["sky_player_rs"]
        else:
            sys.modules["sky_player_rs"] = original_module

    off = _aggregate(off_runs, enabled=False)
    on = _aggregate(on_runs, enabled=True)
    result: dict[str, Any] = {
        "schema_version": 1,
        "evidence_scope": "native_scheduler_test_backend",
        "native_path": "PlaybackEngine -> RustDispatchRuntime -> TestDispatchSession",
        "backend": "sky_player_rs TestDispatchSession / Rust BackendConfig::Mock",
        "physical_input": False,
        "scheduler": "Rust NativeDispatchSession",
        "game_observed": False,
        "native_build": {
            key: native_build.get(key)
            for key in ("rust_core_version", "native_build_commit", "rustc_version", "native_abi")
        },
        "plan": {
            "notes": args.notes,
            "actions": len(actions),
            "game_fps": GAME_FPS,
            "min_hold_us": MIN_HOLD_US,
            "min_release_gap_us": MIN_RELEASE_GAP_US,
            "fingerprint": _fingerprint(actions),
        },
        "diagnostics_off": off,
        "diagnostics_on": on,
        "delta": {
            "max_lateness_us_p50": (
                on["native_timing"]["max_lateness_us"]["p50"]
                - off["native_timing"]["max_lateness_us"]["p50"]
            ),
            "dispatch_start_error_p95_us": (
                on["native_timing"]["dispatch_start_error_us"]["p95_us"]["p95"]
                - off["native_timing"]["dispatch_start_error_us"]["p95_us"]["p95"]
            ),
            "completion_lateness_p95_us": (
                on["native_timing"]["completion_lateness_us"]["p95_us"]["p95"]
                - off["native_timing"]["completion_lateness_us"]["p95_us"]["p95"]
            ),
        },
    }
    encoded = json.dumps(result, indent=2) + "\n"
    print(encoded, end="")
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
