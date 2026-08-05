"""Compare sender-side native playback artifacts across hold profiles.

This reporter deliberately does not inspect a game process or claim that the
foreground game accepted a key. It only classifies evidence emitted by the
native sender and records that boundary in the output.

Example::

    uv run python scripts/report_native_diagnostic.py \
        --run 1.0=run-1f.json \
        --run 1.5=run-1_5f.json \
        --run 1.25=run-1_25f.json \
        --output native-diagnostic.json
"""

from __future__ import annotations

import argparse
import json
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from sky_music.orchestration.native_diagnostics import diagnose_native_playback

SUPPORTED_HOLDS = (1.0, 1.25, 1.5)


def _parse_run(value: str) -> tuple[float, Path]:
    label, separator, raw_path = value.partition("=")
    if not separator or not raw_path:
        raise argparse.ArgumentTypeError("--run must use HOLD=PATH")
    try:
        hold = float(label)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("hold must be one of 1.0, 1.25, or 1.5") from exc
    if hold not in SUPPORTED_HOLDS:
        raise argparse.ArgumentTypeError("hold must be one of 1.0, 1.25, or 1.5")
    return hold, Path(raw_path)


def _read_artifact(path: Path) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"cannot read diagnostic artifact {path}: {exc}") from exc
    if not isinstance(payload, dict):
        raise ValueError(f"diagnostic artifact {path} must contain a JSON object")
    snapshot = payload.get("snapshot")
    if isinstance(snapshot, Mapping):
        merged = dict(payload)
        merged.update(snapshot)
        return merged
    return payload


def build_report(runs: list[tuple[float, Path]]) -> dict[str, Any]:
    if not runs:
        raise ValueError("at least one --run artifact is required")
    if len({hold for hold, _ in runs}) != len(runs):
        raise ValueError("each hold profile may appear only once")

    rendered: list[dict[str, Any]] = []
    for hold, path in runs:
        snapshot = _read_artifact(path)
        diagnosis = diagnose_native_playback(snapshot)
        rendered.append(
            {
                "hold_frames": hold,
                "artifact": str(path),
                "outcome": snapshot.get("outcome", snapshot.get("outcomes")),
                "status": snapshot.get("status"),
                "diagnosis": diagnosis.category,
                "evidence": list(diagnosis.evidence),
                "backend_counters": {
                    name: int(snapshot.get(name, 0) or 0)
                    for name in (
                        "keys_dropped",
                        "chords_rejected",
                        "authored_keys_rejected",
                        "sendinput_partial_events",
                        "sendinput_zero_progress_failures",
                    )
                },
                "latency_flags": {
                    name: bool(snapshot.get(name, False))
                    for name in (
                        "sendinput_path_degraded",
                        "bookkeeping_degraded",
                        "wait_path_degraded",
                    )
                },
                "wake_error_p99_us": int(snapshot.get("wake_error_p99_us", 0) or 0),
                "late_5ms": int(snapshot.get("late_5ms", 0) or 0),
                "late_10ms": int(snapshot.get("late_10ms", 0) or 0),
                "lead_saturation": int(
                    snapshot.get("positive_residual_at_cap", 0) or 0
                ),
            }
        )

    return {
        "report_schema_version": 1,
        "evidence_scope": "sender_completion",
        "game_input_observed": False,
        "interpretation_boundary": (
            "A clean SendInput result does not prove that the game observed the key."
        ),
        "runs": rendered,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="append", type=_parse_run, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        report = build_report(args.run)
    except ValueError as exc:
        raise SystemExit(str(exc)) from exc
    encoded = json.dumps(report, indent=2) + "\n"
    print(encoded, end="")
    if args.output is not None:
        args.output.write_text(encoded, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
