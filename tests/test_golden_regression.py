import sys
import json
from pathlib import Path
import pytest

# Add src to sys.path
src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))
sys.path.insert(0, str(Path(__file__).parent))

from sky_music.domain.scheduler import build_key_actions
from generate_snapshots import get_golden_songs

def test_golden_schedules_regression():
    """Verify that the scheduler's output timelines match the frozen baseline snapshots exactly."""
    songs = get_golden_songs()
    snapshots_dir = Path(__file__).parent / "golden_schedules"
    
    assert snapshots_dir.exists(), "Golden schedules directory must exist."
    
    for key, song in songs.items():
        snapshot_file = snapshots_dir / f"{key}.json"
        assert snapshot_file.exists(), f"Snapshot file for {key} must exist."
        
        with snapshot_file.open("r", encoding="utf-8") as f:
            expected_actions = json.load(f)
            
        res = build_key_actions(song)
        actual_actions = res.actions
        
        assert len(actual_actions) == len(expected_actions), f"Action count mismatch for {key}."
        
        for idx, (actual, expected) in enumerate(zip(actual_actions, expected_actions)):
            assert actual.at_us == expected["at_us"], f"Timestamp mismatch at index {idx} in {key}."
            assert list(actual.scan_codes) == expected["scan_codes"], f"Scan codes mismatch at index {idx} in {key}."
            assert actual.kind == expected["kind"], f"Kind mismatch at index {idx} in {key}."
            assert actual.reason == expected["reason"], f"Reason mismatch at index {idx} in {key}."
            
    print("Golden regression tests passed successfully for all snapshots!")
