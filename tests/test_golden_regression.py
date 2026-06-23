import json
from pathlib import Path

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import TimingPolicy, FrameTimingPolicy

def get_golden_songs():
    # Map song name keywords to their parsed Song objects
    mapping = {
        "golden_chord_15_keys": "chord_15_keys.json",
        "golden_dense_fast_song": "dense_fast_song.json",
        "golden_impossible_repeat_1ms": "impossible_repeat_1ms.json",
        "golden_long_song_3min": "long_song_3min.json",
        "golden_pause_focus_lost": "pause_focus_lost.json",
        "golden_same_key_repeat_15ms": "same_key_repeat_15ms.json",
    }
    
    songs = {}
    for key, filename in mapping.items():
        song_path = Path(__file__).parent.parent / "songs" / filename
        if song_path.exists():
            songs[key] = parse_song_file(song_path)
    return songs

def test_golden_schedules_regression():
    """Verify that the scheduler's output timelines match the frozen baseline snapshots exactly."""
    songs = get_golden_songs()
    snapshots_dir = Path(__file__).parent / "golden_schedules"
    
    assert snapshots_dir.exists(), "Golden schedules directory must exist."
    
    # Use a default timing policy to match the old golden snapshots
    policy = FrameTimingPolicy.from_timing_policy(TimingPolicy.from_dict({}))

    for key, song in songs.items():
        snapshot_file = snapshots_dir / f"{key}.json"
        assert snapshot_file.exists(), f"Snapshot file for {key} must exist."
        
        with snapshot_file.open("r", encoding="utf-8") as f:
            expected_actions = json.load(f)
            
        res = build_key_actions(song, policy=policy)
        actual_actions = res.actions
        
        assert len(actual_actions) == len(expected_actions), f"Action count mismatch for {key}."
        
        for idx, (actual, expected) in enumerate(zip(actual_actions, expected_actions)):
            assert actual.at_us == expected["at_us"], f"Timestamp mismatch at index {idx} in {key}."
            assert actual.kind == expected["kind"], f"Kind mismatch at index {idx} in {key}."
            assert list(actual.scan_codes) == expected["scan_codes"], f"Scan code mismatch at index {idx} in {key}."
