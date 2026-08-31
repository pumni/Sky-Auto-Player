"""Generate immutable Wave 3 song/planning oracle fixtures from Python.

The committed output is evidence for the current Python parser, scheduler, and
analyzer. CI consumes the JSON; it never regenerates it. Run this script only
when intentionally refreshing the migration corpus after reviewing the Python
behavioral change.
"""

from __future__ import annotations

import json
import tempfile
from dataclasses import asdict
from pathlib import Path
from unittest.mock import patch

from sky_music.config import AppConfig
from sky_music.domain.analyzer import analyze_schedule
from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.infrastructure.calibration_loader import (
    SOURCE_DEFAULT_TRANSPORT_300,
    SOURCE_DEVICE_CACHE,
    SOURCE_INCOMPATIBLE_HOST_TRANSPORT_300,
    SOURCE_INVALID_CACHE_TRANSPORT_300,
    CalibrationLoadResult,
    CalibrationStatus,
)
from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "tests" / "fixtures" / "wave3" / "song_planning.json"


VALID_CASES: tuple[tuple[str, object, str], ...] = (
    (
        "object_basic",
        {
            "name": "Demo",
            "songNotes": [
                {"time": 20, "key": "Key1"},
                {"time": 0, "key": "Key0"},
            ],
        },
        ".json",
    ),
    (
        "root_list_first_item",
        [
            {
                "name": "Legacy",
                "songNotes": [{"time": "12", "key": "Key2"}],
            },
            {"name": "Ignored", "songNotes": []},
        ],
        ".skysheet",
    ),
    (
        "empty_song",
        {"name": "Empty", "songNotes": []},
        ".txt",
    ),
    (
        "numeric_string_and_chord",
        {
            "name": "Unicode Élan",
            "songNotes": [
                {"time": "10", "key": "Key3"},
                {"time": 10, "key": "Key4"},
                {"time": 25, "key": "Key3"},
            ],
        },
        ".json",
    ),
    (
        "duplicate_notes",
        {
            "name": "Duplicates",
            "songNotes": [
                {"time": 0, "key": "Key0"},
                {"time": 0, "key": "Key0"},
                {"time": 0, "key": "Key1"},
            ],
        },
        ".json",
    ),
    (
        "dense_chord",
        {
            "name": "Dense",
            "songNotes": [
                {"time": index, "key": f"Key{index}"} for index in range(7)
            ],
        },
        ".json",
    ),
)

INVALID_CASES: tuple[tuple[str, object, str], ...] = (
    ("malformed_json", b"{not json", ".json"),
    ("root_list_empty", [], ".json"),
    ("root_not_object", ["not a song"], ".json"),
    ("missing_song_notes", {"name": "Missing"}, ".json"),
    (
        "missing_note_time",
        {"songNotes": [{"key": "Key0"}]},
        ".json",
    ),
    (
        "invalid_note_key",
        {"songNotes": [{"time": 0, "key": "NotAKey"}]},
        ".json",
    ),
    (
        "negative_time",
        {"songNotes": [{"time": -1, "key": "Key0"}]},
        ".json",
    ),
)


def _capture_parse(raw: object, suffix: str) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="sky-wave3-fixture-") as directory:
        path = Path(directory) / f"fixture{suffix}"
        if isinstance(raw, bytes):
            path.write_bytes(raw)
        else:
            path.write_text(json.dumps(raw), encoding="utf-8")
        try:
            song = parse_song_file(path)
        except Exception as error:
            return {"status": "error", "error_type": type(error).__name__}
        return {
            "status": "ok",
            "song": {
                "name": song.name,
                "notes": [
                    {
                        "time_ms": int(note.time_ms),
                        "key": str(note.key),
                    }
                    for note in song.notes
                ],
            },
        }


def _resolved_policy(*, hold: float, fps: int, margin: int, source: str) -> FrameTimingPolicy:
    """Resolve through the production calibration-aware entry point.

    The loader result is patched only to make each committed case deterministic;
    the resolver and domain materialization are the same production functions
    used by the desktop player.
    """
    result = CalibrationLoadResult(
        status=(
            CalibrationStatus.VALID
            if source == SOURCE_DEVICE_CACHE
            else CalibrationStatus.UNCALIBRATED
        ),
        resolved_margin_us=margin,
        margin_source=source,
        summary=None,
    )
    with patch(
        "sky_music.orchestration.calibrated_policy.load_calibration_resolution",
        return_value=result,
    ):
        return resolve_calibrated_policy(
            PlaybackSessionContext.default(hold_frames=hold, fps=fps),
            AppConfig(),
        )


def _schedule_case(
    name: str,
    raw: object,
    suffix: str,
    *,
    hold: float,
    tempo: float,
    fps: int,
    margin: int = 300,
    margin_source: str = SOURCE_DEFAULT_TRANSPORT_300,
) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="sky-wave3-schedule-") as directory:
        path = Path(directory) / f"fixture{suffix}"
        path.write_text(json.dumps(raw), encoding="utf-8")
        song = parse_song_file(path)
    policy = _resolved_policy(
        hold=hold,
        fps=fps,
        margin=margin,
        source=margin_source,
    )
    schedule = build_key_actions(song, policy=policy, tempo_scale=tempo)
    risk = analyze_schedule(
        schedule,
        raw_notes=song.notes,
        current_hold_frames=hold,
        current_tempo_scale=tempo,
    )
    return {
        "name": name,
        "raw": raw,
        "hold_frames": hold,
        "tempo_scale": tempo,
        "fps": fps,
        "transport_margin_us": int(policy.transport_margin_us or 0),
        "transport_margin_source": policy.min_hold_margin_source,
        "schedule": {
            "actions": [
                {
                    "at_us": int(action.at_us),
                    "scan_codes": [int(code) for code in action.scan_codes],
                    "kind": str(action.kind),
                    "reason": action.reason,
                }
                for action in schedule.actions
            ],
            "source_duration_us": int(schedule.source_duration_us),
            "playback_duration_us": int(schedule.playback_duration_us),
            "duration_us": int(schedule.duration_us),
            "note_count": schedule.note_count,
            "deduplicated_note_count": schedule.deduplicated_note_count,
            "duplicate_note_count": schedule.duplicate_note_count,
            "compressed_holds": schedule.compressed_holds,
            "impossible_same_key_repeats": schedule.impossible_same_key_repeats,
            "risky_same_key_repeats": schedule.risky_same_key_repeats,
            "max_polyphony": schedule.max_polyphony,
            "shortest_same_key_interval_us": schedule.shortest_same_key_interval_us,
            "min_same_key_up_gap_us": schedule.min_same_key_up_gap_us,
            "recommended_hold_frames": schedule.recommended_hold_frames,
            "recommended_tempo_scale": schedule.recommended_tempo_scale,
        },
        "risk": asdict(risk),
    }


def _policy_case(name: str, *, margin: int, source: str) -> dict[str, object]:
    policy = _resolved_policy(hold=1.25, fps=120, margin=margin, source=source)
    return {
        "name": name,
        "hold_frames": 1.25,
        "fps": 120,
        "transport_margin_us": int(policy.transport_margin_us or 0),
        "transport_margin_source": policy.min_hold_margin_source,
        "frame_us": int(policy.frame_us),
        "frame_base_hold_us": int(policy.frame_base_hold_us or 0),
        "down_late_grace_us": int(policy.down_late_grace_us),
        "min_hold_us": int(policy.min_hold_us),
        "min_release_gap_us": int(policy.min_release_gap_us or 0),
        "focus_restore_grace_us": int(policy.focus_restore_grace_us),
    }


def main() -> None:
    parser_cases = []
    for name, raw, suffix in VALID_CASES + INVALID_CASES:
        parser_cases.append({"name": name, "raw": raw.decode() if isinstance(raw, bytes) else raw, **_capture_parse(raw, suffix)})

    schedule_cases = [
        _schedule_case("basic_schedule", VALID_CASES[0][1], ".json", hold=1.0, tempo=1.0, fps=60),
        _schedule_case("tempo_rounding", VALID_CASES[3][1], ".json", hold=1.25, tempo=0.95, fps=120),
        _schedule_case(
            "calibrated_margin",
            VALID_CASES[0][1],
            ".json",
            hold=1.0,
            tempo=1.0,
            fps=60,
            margin=777,
            margin_source=SOURCE_DEVICE_CACHE,
        ),
        _schedule_case("empty_schedule", VALID_CASES[2][1], ".txt", hold=1.0, tempo=1.0, fps=60),
        _schedule_case("dense_risk", VALID_CASES[5][1], ".json", hold=1.0, tempo=1.0, fps=60),
    ]
    timing_policy_cases = [
        _policy_case(
            "no_cache_fallback",
            margin=300,
            source=SOURCE_DEFAULT_TRANSPORT_300,
        ),
        _policy_case("valid_device_cache", margin=777, source=SOURCE_DEVICE_CACHE),
        _policy_case(
            "unhealthy_cache_fallback",
            margin=300,
            source=SOURCE_INVALID_CACHE_TRANSPORT_300,
        ),
        _policy_case(
            "stale_cache_fallback",
            margin=300,
            source=SOURCE_INCOMPATIBLE_HOST_TRANSPORT_300,
        ),
    ]
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(
        json.dumps(
            {
                "schema": 1,
                "parser_cases": parser_cases,
                "schedule_cases": schedule_cases,
                "timing_policy_cases": timing_policy_cases,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
