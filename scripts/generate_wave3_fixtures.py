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

from sky_music.domain.analyzer import analyze_schedule
from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy

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


def _schedule_case(name: str, raw: object, suffix: str, *, hold: float, tempo: float, fps: int) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="sky-wave3-schedule-") as directory:
        path = Path(directory) / f"fixture{suffix}"
        path.write_text(json.dumps(raw), encoding="utf-8")
        song = parse_song_file(path)
    policy = FrameTimingPolicy.from_hold_frames(hold, fps)
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
        },
        "risk": asdict(risk),
    }


def main() -> None:
    parser_cases = []
    for name, raw, suffix in VALID_CASES + INVALID_CASES:
        parser_cases.append({"name": name, "raw": raw.decode() if isinstance(raw, bytes) else raw, **_capture_parse(raw, suffix)})

    schedule_cases = [
        _schedule_case("basic_schedule", VALID_CASES[0][1], ".json", hold=1.0, tempo=1.0, fps=60),
        _schedule_case("tempo_rounding", VALID_CASES[3][1], ".json", hold=1.25, tempo=0.95, fps=120),
        _schedule_case("empty_schedule", VALID_CASES[2][1], ".txt", hold=1.0, tempo=1.0, fps=60),
        _schedule_case("dense_risk", VALID_CASES[5][1], ".json", hold=1.0, tempo=1.0, fps=60),
    ]
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(
        json.dumps(
            {"schema": 1, "parser_cases": parser_cases, "schedule_cases": schedule_cases},
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
