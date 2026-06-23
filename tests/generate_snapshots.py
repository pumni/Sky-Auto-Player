import json
from pathlib import Path

from sky_music.domain import Song, Note, NoteKey, Millis
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import TimingPolicy, FrameTimingPolicy

def get_golden_songs():
    songs = {}

    # 1. golden_chord_15_keys
    songs["golden_chord_15_keys"] = Song(
        name="Golden Chord 15 Keys",
        notes=tuple(Note(time_ms=Millis(1000), key=NoteKey(f"Key{i}")) for i in range(15))
    )

    # 2. golden_same_key_repeat_15ms
    songs["golden_same_key_repeat_15ms"] = Song(
        name="Golden Same Key Repeat 15ms",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1015), key=NoteKey("Key0")),
        )
    )

    # 3. golden_impossible_repeat_1ms
    songs["golden_impossible_repeat_1ms"] = Song(
        name="Golden Impossible Repeat 1ms",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")),
        )
    )

    # 4. golden_dense_fast_song
    songs["golden_dense_fast_song"] = Song(
        name="Golden Dense Fast Song",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1010), key=NoteKey("Key1")),
            Note(time_ms=Millis(1020), key=NoteKey("Key2")),
            Note(time_ms=Millis(1030), key=NoteKey("Key0")),
            Note(time_ms=Millis(1040), key=NoteKey("Key1")),
            Note(time_ms=Millis(1050), key=NoteKey("Key2")),
        )
    )

    # 5. golden_long_song_3min
    long_notes = []
    for t in range(0, 180000, 500): # Note every 500ms for 3 minutes
        long_notes.append(Note(time_ms=Millis(t), key=NoteKey(f"Key{t % 15}")))
    songs["golden_long_song_3min"] = Song(
        name="Golden Long Song 3min",
        notes=tuple(long_notes)
    )

    # 6. golden_pause_focus_lost
    # Short song with a few notes to check pause execution
    songs["golden_pause_focus_lost"] = Song(
        name="Golden Pause Focus Lost",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(100), key=NoteKey("Key1")),
            Note(time_ms=Millis(200), key=NoteKey("Key2")),
        )
    )

    return songs

def generate_snapshots():
    songs = get_golden_songs()
    snapshots_dir = Path(__file__).parent / "golden_schedules"
    snapshots_dir.mkdir(parents=True, exist_ok=True)

    print(f"Generating golden schedule snapshots to: {snapshots_dir.resolve()}")

    policy = FrameTimingPolicy.from_timing_policy(TimingPolicy.from_dict({}))

    for key, song in songs.items():
        res = build_key_actions(song, policy=policy)
        actions = res.actions
        
        # Serialize actions to match KeyAction data fields
        serialized_actions = []
        for action in actions:
            serialized_actions.append({
                "at_us": action.at_us,
                "scan_codes": list(action.scan_codes),
                "kind": action.kind,
                "reason": action.reason
            })
            
        output_file = snapshots_dir / f"{key}.json"
        with output_file.open("w", encoding="utf-8") as f:
            json.dump(serialized_actions, f, indent=4)
        print(f"  -> Generated: {output_file.name} ({len(serialized_actions)} actions)")

if __name__ == "__main__":
    generate_snapshots()
