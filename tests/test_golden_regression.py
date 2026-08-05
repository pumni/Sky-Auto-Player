import json
from pathlib import Path

from sky_music.domain import Millis, Note, NoteKey, Song
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy


def get_golden_songs() -> dict[str, Song]:
    """Build the deterministic songs used by the snapshot generator."""
    return {
        "golden_chord_15_keys": Song(
            name="Golden Chord 15 Keys",
            notes=tuple(Note(time_ms=Millis(1000), key=NoteKey(f"Key{i}")) for i in range(15)),
        ),
        "golden_same_key_repeat_15ms": Song(
            name="Golden Same Key Repeat 15ms",
            notes=(
                Note(time_ms=Millis(1000), key=NoteKey("Key0")),
                Note(time_ms=Millis(1015), key=NoteKey("Key0")),
            ),
        ),
        "golden_impossible_repeat_1ms": Song(
            name="Golden Impossible Repeat 1ms",
            notes=(
                Note(time_ms=Millis(1000), key=NoteKey("Key0")),
                Note(time_ms=Millis(1001), key=NoteKey("Key0")),
            ),
        ),
        "golden_dense_fast_song": Song(
            name="Golden Dense Fast Song",
            notes=(
                Note(time_ms=Millis(1000), key=NoteKey("Key0")),
                Note(time_ms=Millis(1010), key=NoteKey("Key1")),
                Note(time_ms=Millis(1020), key=NoteKey("Key2")),
                Note(time_ms=Millis(1030), key=NoteKey("Key0")),
                Note(time_ms=Millis(1040), key=NoteKey("Key1")),
                Note(time_ms=Millis(1050), key=NoteKey("Key2")),
            ),
        ),
        "golden_long_song_3min": Song(
            name="Golden Long Song 3min",
            notes=tuple(
                Note(time_ms=Millis(time_ms), key=NoteKey(f"Key{time_ms % 15}"))
                for time_ms in range(0, 180000, 500)
            ),
        ),
        "golden_pause_focus_lost": Song(
            name="Golden Pause Focus Lost",
            notes=tuple(
                Note(time_ms=Millis(time_ms), key=NoteKey(f"Key{time_ms // 100}"))
                for time_ms in (0, 100, 200)
            ),
        ),
    }

def test_golden_schedules_regression():
    """Verify that the scheduler's output timelines match the frozen baseline snapshots exactly."""
    songs = get_golden_songs()
    snapshots_dir = Path(__file__).parent / "golden_schedules"
    
    assert snapshots_dir.exists(), "Golden schedules directory must exist."
    
    # Golden candidates use the new explicit default: one frame at 60 FPS.
    policy = FrameTimingPolicy.from_hold_frames(1.0, 60)

    for key, song in songs.items():
        snapshot_file = snapshots_dir / f"{key}.json"
        assert snapshot_file.exists(), f"Snapshot file for {key} must exist."
        
        with snapshot_file.open("r", encoding="utf-8") as f:
            expected_actions = json.load(f)
            
        res = build_key_actions(song, policy=policy)
        actual_actions = res.actions
        
        assert len(actual_actions) == len(expected_actions), f"Action count mismatch for {key}."
        
        for idx, (actual, expected) in enumerate(zip(actual_actions, expected_actions, strict=False)):
            assert actual.at_us == expected["at_us"], f"Timestamp mismatch at index {idx} in {key}."
            assert actual.kind == expected["kind"], f"Kind mismatch at index {idx} in {key}."
            assert list(actual.scan_codes) == expected["scan_codes"], f"Scan code mismatch at index {idx} in {key}."
