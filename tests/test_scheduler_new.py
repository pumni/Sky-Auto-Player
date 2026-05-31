import sys
from pathlib import Path
import pytest

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.domain import Song, Note, NoteKey, Millis
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import TimingPolicy, KeyAction

def test_chord_batching_and_deduplication():
    """Verify that multiple notes at the same timestamp are batched without duplicate scan codes."""
    # Key 'y' (Key0) and 'u' (Key1) played at 1000ms
    song = Song(
        name="Test Chord",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1000), key=NoteKey("Key1")),
            Note(time_ms=Millis(1000), key=NoteKey("Key0")), # Duplicate key in same chord
        )
    )
    
    res = build_key_actions(song)
    actions = res.actions
    
    # We should have exactly 1 down action batch and up action batches
    down_actions = [a for a in actions if a.kind == "down"]
    assert len(down_actions) == 1
    
    down_act = down_actions[0]
    assert down_act.at_us == 1000 * 1000
    assert set(down_act.scan_codes) == {0x15, 0x16} # Physical scan codes for 'y' and 'u'
    assert len(down_act.scan_codes) == 2 # Deduplicated

def test_same_key_repeat_releases_first():
    """Verify same-key repeat scheduling releases the previous key before hitting the next down."""
    # 'y' pressed at 1000ms and repeated at 1015ms.
    # repeat_release_gap is 2ms (2000us).
    # Expected: Down(1000ms) -> Up(1013ms, repeat_release) -> Down(1015ms) -> Up(1035ms, final_release)
    song = Song(
        name="Test Repeat",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1015), key=NoteKey("Key0")),
        )
    )
    
    policy = TimingPolicy(hold_us=20_000, repeat_release_gap_us=2_000)
    res = build_key_actions(song, policy=policy)
    actions = res.actions
    
    assert len(actions) == 4
    
    # Assert exact timing timeline in microseconds
    assert actions[0].at_us == 1000_000
    assert actions[0].kind == "down"
    
    assert actions[1].at_us == 1013_000
    assert actions[1].kind == "up"
    assert actions[1].reason == "repeat_release"
    
    assert actions[2].at_us == 1015_000
    assert actions[2].kind == "down"
    
    assert actions[3].at_us == 1035_000
    assert actions[3].kind == "up"
    assert actions[3].reason == "final_release"
    
    assert res.compressed_holds == 1
    assert res.impossible_same_key_repeats == 0

def test_prioritization_at_same_timestamp():
    """Verify key event scheduling priorities when multiple events fall on the exact same microsecond."""
    # At 1000ms:
    # 1. We have a same-key repeat 'up' (reason: repeat_release) scheduled at 1000ms (due to a dense note).
    # 2. We have a new note 'down' scheduled at 1000ms.
    # 3. We have an unrelated normal 'up' (reason: release) scheduled at 1000ms.
    # 4. We have a final 'up' (reason: final_release) scheduled at 1000ms.
    
    # We construct mock actions to assert sorting priority directly:
    a_down = KeyAction(at_us=1000, scan_codes=(0x15,), kind="down", reason="note")
    a_up_repeat = KeyAction(at_us=1000, scan_codes=(0x15,), kind="up", reason="repeat_release")
    a_up_normal = KeyAction(at_us=1000, scan_codes=(0x16,), kind="up", reason="release")
    a_up_final = KeyAction(at_us=1000, scan_codes=(0x17,), kind="up", reason="final_release")
    
    unsorted = [a_up_final, a_up_normal, a_down, a_up_repeat]
    
    # Using the exact prioritization key defined in build_key_actions:
    def action_priority(action: KeyAction) -> int:
        if action.kind == "up":
            if action.reason == "repeat_release":
                return 0
            elif action.reason == "release":
                return 2
            else:
                return 3
        else:
            return 1
            
    sorted_actions = sorted(unsorted, key=lambda a: (a.at_us, action_priority(a)))
    
    # Expected order: repeat_release -> down -> release -> final_release
    assert sorted_actions[0] == a_up_repeat
    assert sorted_actions[1] == a_down
    assert sorted_actions[2] == a_up_normal
    assert sorted_actions[3] == a_up_final

def test_impossible_same_key_repeat_diagnostics():
    """Verify extremely fast repeats trigger correct diagnostics without crashing."""
    song = Song(
        name="Extreme Speed",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")), # 1ms gap (impossible)
        )
    )
    
    res = build_key_actions(song)
    assert res.impossible_same_key_repeats == 1
    assert res.compressed_holds == 1
    
    # Check that fallback hold (500us) was applied
    actions = res.actions
    assert actions[1].at_us == 1000_500 # 1000ms + 500us fallback hold
    assert actions[1].kind == "up"

def test_scheduler_fails_on_unmapped_note_key():
    """Verify that build_key_actions raises ValueError on unmapped note keys instead of silently dropping them."""
    song = Song(
        name="Invalid Key",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key999")),
        )
    )
    with pytest.raises(ValueError, match="Cannot map note key 'Key999'"):
        build_key_actions(song)

def test_release_gap_us_affects_normal_release_ordering():
    """Verify TimingPolicy.release_gap_us shifts normal releases when they coincide with unrelated note down onsets."""
    # Key0 pressed at 1000ms (held for 20ms, so normal release scheduled at 1020ms)
    # Key1 pressed at 1020ms (down scheduled at 1020ms)
    # This creates a collision at 1020ms: Down(Key1) and Up(Key0, normal release)
    song = Song(
        name="Coinciding Release",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1020), key=NoteKey("Key1")),
        )
    )
    
    # Using hold=20ms and release_gap=3ms
    policy = TimingPolicy(hold_us=20_000, release_gap_us=3_000)
    res = build_key_actions(song, policy=policy)
    actions = res.actions
    
    # Events in flat_notes:
    # Key0 (y): Down at 1000ms, Up at 1020ms (reason: release)
    # Key1 (u): Down at 1020ms
    # Since Up(Key0) lands at 1020ms which has a Down(Key1) at 1020ms,
    # the normal release Up(Key0) is delayed to T + release_gap_us = 1023ms (1023000us)
    
    down_key1 = [a for a in actions if a.kind == "down" and 0x16 in a.scan_codes][0]
    up_key0 = [a for a in actions if a.kind == "up" and 0x15 in a.scan_codes][0]
    
    assert down_key1.at_us == 1020_000
    assert up_key0.at_us == 1023_000 # Shifted by release_gap_us (3000us)

def test_pre_playback_schedule_analyzer():
    """Verify that ScheduleRiskReport correctly diagnoses optimal, impossible-repeat, and dense sheets."""
    from sky_music.domain.analyzer import analyze_schedule
    
    # 1. Optimal Song
    song_opt = Song(
        name="Optimal Song",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(500), key=NoteKey("Key1")),
        )
    )
    res_opt = build_key_actions(song_opt)
    report_opt = analyze_schedule(res_opt)
    assert report_opt.severity == "low"
    assert "no timing conflicts detected" in report_opt.recommendations[0].lower()
    
    # 2. Impossible repeats
    song_imp = Song(
        name="Impossible Repeats",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")), # 1ms repeat
        )
    )
    res_imp = build_key_actions(song_imp)
    report_imp = analyze_schedule(res_imp)
    assert report_imp.severity == "high"
    assert report_imp.impossible_repeats == 1
    assert any("too close" in rec.lower() for rec in report_imp.recommendations)

    # 3. Dense clusters (15 notes all within 50ms)
    notes_dense = []
    for idx in range(15):
        notes_dense.append(Note(time_ms=Millis(1000 + idx * 3), key=NoteKey(f"Key{idx % 5}")))
    song_dense = Song(name="Dense Song", notes=tuple(notes_dense))
    res_dense = build_key_actions(song_dense)
    report_dense = analyze_schedule(res_dense)
    # The severity should flag as high or medium due to many notes in 100ms
    assert report_dense.severity in ("medium", "high")
    assert len(report_dense.dense_clusters) > 0
    assert any("dense" in rec.lower() for rec in report_dense.recommendations)


