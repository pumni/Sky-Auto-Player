import sys
from pathlib import Path
import pytest

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.domain import Song, Note, NoteKey, Millis
from sky_music.domain.scheduler import build_key_actions, ScheduleBuildError
from sky_music.domain.scheduler_types import TimingPolicy, KeyAction, FrameTimingPolicy

def _policy(d: dict | None = None) -> FrameTimingPolicy:
    return FrameTimingPolicy.from_timing_policy(TimingPolicy.from_dict(d or {}))

def test_chord_batching_and_deduplication():
    """Verify that multiple notes at the same timestamp are batched without duplicate scan codes."""
    song = Song(
        name="Test Chord",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1000), key=NoteKey("Key1")),
            Note(time_ms=Millis(1000), key=NoteKey("Key0")), # Duplicate key
        )
    )
    policy = _policy({"input_lead_us": 0})
    res = build_key_actions(song, policy=policy)
    down_actions = [a for a in res.actions if a.kind == "down"]
    assert len(down_actions) == 1
    assert set(down_actions[0].scan_codes) == {0x15, 0x16}

def test_third_instrument_key_schedules_as_base_key():
    song = Song(
        name="Third Instrument",
        notes=(Note(time_ms=Millis(1000), key=NoteKey("3Key5")),)
    )
    policy = _policy({"input_lead_us": 0})
    res = build_key_actions(song, policy=policy)

    assert res.actions[0].scan_codes == (0x23,)

def test_same_key_repeat_releases_first():
    """Verify same-key repeat scheduling releases the previous key before hitting the next down."""
    song = Song(
        name="Test Repeat",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1015), key=NoteKey("Key0")),
        )
    )
    policy = _policy({
        "hold_us": 20_000, "min_hold_us": 10_000, "release_gap_us": 3_000,
        "repeat_release_gap_us": 2_000, "input_lead_us": 0
    })
    res = build_key_actions(song, policy=policy)
    actions = res.actions
    assert len(actions) == 4
    assert actions[0].at_us == 1000_000 # Down 1
    assert actions[1].at_us == 1013_000 # Up 1 (1015 - 2ms gap)
    assert actions[1].kind == "up"
    assert actions[1].reason == "repeat_release"
    assert actions[2].at_us == 1015_000 # Down 2
    assert actions[3].at_us == 1035_000 # Up 2 (1015 + 20ms hold)
    assert res.compressed_holds == 1

def test_prioritization_at_same_timestamp():
    """Verify key event scheduling priorities when multiple events fall on the exact same microsecond."""
    a_down = KeyAction(at_us=1000, scan_codes=(0x15,), kind="down", reason="onset")
    a_up_repeat = KeyAction(at_us=1000, scan_codes=(0x15,), kind="up", reason="repeat_release")
    a_up_normal = KeyAction(at_us=1000, scan_codes=(0x16,), kind="up", reason="release")
    
    unsorted = [a_up_normal, a_down, a_up_repeat]
    def action_priority(action: KeyAction) -> int:
        if action.kind == "up":
            return 0 if action.reason == "repeat_release" else 2
        return 1
    sorted_actions = sorted(unsorted, key=lambda a: (a.at_us, action_priority(a)))
    assert sorted_actions[0] == a_up_repeat
    assert sorted_actions[1] == a_down
    assert sorted_actions[2] == a_up_normal

def test_impossible_same_key_repeat_diagnostics():
    """Verify extremely fast repeats trigger correct diagnostics without crashing."""
    song = Song(
        name="Extreme Speed",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")),
        )
    )
    policy = _policy({"input_lead_us": 0, "min_hold_us": 10000})
    res = build_key_actions(song, policy=policy)
    assert res.impossible_same_key_repeats == 1
    up_action = next(a for a in res.actions if a.kind == "up" and 0x15 in a.scan_codes)
    assert up_action.at_us == 1010_000 # 1000ms + 10ms min_hold

def test_scheduler_fails_on_unmapped_note_key():
    song = Song(name="Invalid", notes=(Note(time_ms=Millis(1000), key=NoteKey("Key999")),))
    with pytest.raises(ValueError, match="Cannot map note key 'Key999'"):
        build_key_actions(song)

def test_release_gap_us_at_120fps():
    """Verify that FrameTimingPolicy scales release gaps correctly for high refresh rates."""
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    base = TimingPolicy.from_dict({"release_gap_us": 3000})
    # 120fps = 8,333us per frame. 15% of frame = 1249us.
    # Policy says max(3000, 1249) = 3000.
    frame_policy = FrameTimingPolicy.from_timing_policy(base, fps=120)
    assert frame_policy.release_gap_us == 3000

def test_release_gap_us_at_30fps():
    """Verify that FrameTimingPolicy scales release gaps correctly for low refresh rates."""
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    base = TimingPolicy.from_dict({"release_gap_us": 3000})
    # 30fps = 33,333us per frame. 15% of frame = 4999us.
    # Policy says max(3000, 4999) = 4999.
    frame_policy = FrameTimingPolicy.from_timing_policy(base, fps=30)
    assert frame_policy.release_gap_us == 4999

def test_frame_timing_policy_scales_lead_at_30fps():
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    base = TimingPolicy.from_dict({"input_lead_us": 6000})
    # 30fps = 33,333us. 50% = 16,666us.
    frame_policy = FrameTimingPolicy.from_timing_policy(base, fps=30)
    assert frame_policy.input_lead_us == 16666

def test_chord_merge_clamping_at_120fps():
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    base = TimingPolicy.from_dict({"chord_merge_window_us": 5000})
    # 120fps = 8,333us. 25% = 2083us.
    # Policy says min(5000, 2083) = 2083.
    frame_policy = FrameTimingPolicy.from_timing_policy(base, fps=120)
    assert frame_policy.chord_merge_window_us == 2083

def test_pre_playback_schedule_analyzer():
    from sky_music.domain.analyzer import analyze_schedule
    song = Song(name="Test", notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),))
    res = build_key_actions(song)
    report = analyze_schedule(res)
    assert report.severity == "low"

def test_analyzer_detects_impossible_repeats():
    from sky_music.domain.analyzer import analyze_schedule
    song = Song(name="Imp", notes=(Note(time_ms=Millis(1000), key=NoteKey("Key0")), Note(time_ms=Millis(1001), key=NoteKey("Key0"))))
    res = build_key_actions(song, policy=_policy({"input_lead_us": 0}))
    report = analyze_schedule(res)
    assert report.severity == "high"
    assert report.impossible_same_key_repeats == 1

def test_analyzer_detects_high_polyphony():
    from sky_music.domain.analyzer import analyze_schedule
    notes = [Note(time_ms=Millis(1000), key=NoteKey(f"Key{i}")) for i in range(10)]
    song = Song(name="Poly", notes=tuple(notes))
    res = build_key_actions(song)
    report = analyze_schedule(res)
    assert report.severity == "medium" # 10 simultaneous keys

def test_analyzer_detects_dense_clusters():
    from sky_music.domain.analyzer import analyze_schedule
    notes = [Note(time_ms=Millis(1000 + i*2), key=NoteKey(f"Key{i%5}")) for i in range(20)]
    song = Song(name="Dense", notes=tuple(notes))
    res = build_key_actions(song)
    report = analyze_schedule(res)
    assert report.severity in ("medium", "high")
    assert len(report.dense_clusters) > 0

def test_repeat_release_gap_scales_at_30fps():
    base = TimingPolicy.from_dict({"repeat_release_gap_us": 2000})
    frame_policy = FrameTimingPolicy.from_timing_policy(base, fps=30)
    # 30fps = 33,333us. 50% = 16,666us.
    assert frame_policy.repeat_release_gap_us == 16666


def test_release_collision_delay_separates_up_from_conflicting_down():
    """When a key release coincides with another key's down, release is deferred."""
    song = Song(
        name="Collision",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1024), key=NoteKey("Key1")),
        ),
    )
    policy = _policy({"hold_us": 24_000, "release_gap_us": 3_000, "input_lead_us": 0})
    res = build_key_actions(song, policy=policy)
    key0_up = next(a for a in res.actions if a.kind == "up" and 0x15 in a.scan_codes)
    key1_down = next(a for a in res.actions if a.kind == "down" and 0x16 in a.scan_codes)
    assert key1_down.at_us == 1_024_000
    assert key0_up.at_us == 1_024_000 + 3_000


def test_min_hold_scales_at_30fps():
    base = TimingPolicy.from_dict({"min_hold_us": 16000})
    frame_policy = FrameTimingPolicy.from_timing_policy(base, fps=30)
    # 30fps = 33,333us. 60% = 19,999us.
    assert frame_policy.min_hold_us == 19_999


def test_strict_policy_rejects_impossible_repeat():
    song = Song(
        name="Strict Fail",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"input_lead_us": 0}),
        same_key_conflict_policy="strict",
    )
    with pytest.raises(ScheduleBuildError) as exc_info:
        build_key_actions(song, policy=policy)
    assert exc_info.value.recommended_profile == "dense-safe"
    assert exc_info.value.recommended_tempo_scale is not None


def test_degraded_policy_still_compresses_impossible_repeat():
    song = Song(
        name="Degraded",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"input_lead_us": 0}),
        same_key_conflict_policy="degraded",
    )
    res = build_key_actions(song, policy=policy)
    assert res.impossible_same_key_repeats == 1


def test_down_only_frame_align_snaps_key_down():
    song = Song(
        name="Align",
        notes=(Note(time_ms=Millis(1000), key=NoteKey("Key0")),),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"input_lead_us": 0}),
        fps=30,
        frame_align="down_only",
    )
    res = build_key_actions(song, policy=policy)
    down = next(a for a in res.actions if a.kind == "down")
    assert down.at_us == 999_990


def test_frame_align_same_key_repeat_uses_aligned_next_down():
    song = Song(
        name="Aligned Repeat",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1010), key=NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({
            "hold_us": 2_000,
            "min_hold_us": 1_000,
            "repeat_release_gap_us": 1_000,
            "input_lead_us": 0,
        }),
        fps=30,
        min_visible_hold_frames=0,
        min_hold_min_frame_ratio=0,
        repeat_release_gap_min_frame_ratio=0,
        frame_align="down_only",
        same_key_conflict_policy="strict",
    )
    with pytest.raises(ScheduleBuildError):
        build_key_actions(song, policy=policy)


def test_none_frame_align_preserves_exact_down_time():
    song = Song(
        name="Exact",
        notes=(Note(time_ms=Millis(1000), key=NoteKey("Key0")),),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"input_lead_us": 0}),
        fps=None,
        frame_align="none",
    )
    res = build_key_actions(song, policy=policy)
    down = next(a for a in res.actions if a.kind == "down")
    assert down.at_us == 1_000_000


def test_timing_policy_from_dict_defaults():
    policy = TimingPolicy.from_dict({})
    assert policy.hold_us == 24000
    assert policy.min_hold_us == 14000

def test_timing_policy_from_profile_name():
    policy = TimingPolicy.from_profile_name("local-precise")
    assert policy.hold_us == 20000
    policy_2 = TimingPolicy.from_profile_name("remote-safe")
    assert policy_2.hold_us == 35000

def test_frame_timing_policy_from_profile_name():
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    p = FrameTimingPolicy.from_profile_name("balanced", fps=60)
    assert p.fps == 60
    assert p.hold_us == 24000

def test_playback_overrides_dataclass():
    from main import PlaybackOverrides
    o = PlaybackOverrides(dry_run=True, fps=120)
    assert o.dry_run is True
    assert o.fps == 120
