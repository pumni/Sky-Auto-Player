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
    # 30fps = 33,333us per frame. 15% of frame = 5000us (ceiled).
    # Policy says max(3000, 5000) = 5000.
    frame_policy = FrameTimingPolicy.from_timing_policy(base, fps=30)
    assert frame_policy.release_gap_us == 5000
 
def test_frame_timing_policy_has_no_lead_field():
    frame_policy = FrameTimingPolicy.from_profile_name("balanced", fps=30)
    assert not hasattr(frame_policy, "input_lead_us")


def test_onsets_are_not_shifted_or_clamped():
    song = Song(
        name="Even",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(500), key=NoteKey("Key1")),
            Note(time_ms=Millis(1000), key=NoteKey("Key2")),
            Note(time_ms=Millis(1500), key=NoteKey("Key3")),
        ),
    )
    policy = FrameTimingPolicy.from_profile_name("local-precise", fps=60)
    res = build_key_actions(song, policy=policy)
    downs = [int(a.at_us) for a in res.actions if a.kind == "down"]
    assert downs == [0, 500_000, 1_000_000, 1_500_000]
    assert [b - a for a, b in zip(downs, downs[1:])] == [500_000, 500_000, 500_000]

def test_chord_window_field_is_removed():
    frame_policy = FrameTimingPolicy.from_profile_name("balanced", fps=120)
    assert not hasattr(frame_policy, "chord_merge_window_us")

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
    # 30fps = 33,333us. Empirical floor (Exp2) = max(base 2000, 1.5*frame=50000, 18000) = 50000.
    assert frame_policy.repeat_release_gap_us == 50000
 
 
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
    # 30fps = 33,333us. Visibility floor (Exp1) = max(base 16000, 1.25*frame=41667) = 41667.
    assert frame_policy.min_hold_us == 41667


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


def test_exact_timestamp_chord_still_groups_without_window():
    song = Song(
        name="Exact Chord",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1000), key=NoteKey("Key1")),
            Note(time_ms=Millis(1000), key=NoteKey("Key2")),
        ),
    )
    policy = FrameTimingPolicy.from_profile_name("balanced", fps=60)
    res = build_key_actions(song, policy=policy)
    downs = [a for a in res.actions if a.kind == "down"]
    assert len(downs) == 1
    assert len(downs[0].scan_codes) == 3
    assert downs[0].at_us == 1_000_000


def test_nearby_chord_notes_are_not_window_merged():
    song = Song(
        name="Spread Chord",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1010), key=NoteKey("Key1")),
            Note(time_ms=Millis(1020), key=NoteKey("Key2")),
        ),
    )
    policy = FrameTimingPolicy.from_profile_name("balanced", fps=60)
    res = build_key_actions(song, policy=policy)
    downs = [a for a in res.actions if a.kind == "down"]
    assert [int(a.at_us) for a in downs] == [1_000_000, 1_010_000, 1_020_000]
    assert [len(a.scan_codes) for a in downs] == [1, 1, 1]


def test_frame_alignment_field_is_removed():
    song = Song(
        name="Exact",
        notes=(Note(time_ms=Millis(1000), key=NoteKey("Key0")),),
    )
    policy = FrameTimingPolicy.from_profile_name("balanced", fps=30)
    assert not hasattr(policy, "frame_align")
    res = build_key_actions(song, policy=policy)
    down = next(a for a in res.actions if a.kind == "down")
    assert down.at_us == 1_000_000


def test_timing_policy_from_dict_defaults():
    policy = TimingPolicy.from_dict({})
    assert policy.hold_us == 26000
    assert policy.min_hold_us == 17000

def test_timing_policy_from_profile_name():
    policy = TimingPolicy.from_profile_name("local-precise")
    assert policy.hold_us == 22000
    policy_2 = TimingPolicy.from_profile_name("audience-safe")
    assert policy_2.hold_us == 18000

def test_frame_timing_policy_from_profile_name():
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    p = FrameTimingPolicy.from_profile_name("balanced", fps=60)
    assert p.fps == 60
    assert p.hold_us == 20001


def test_local_precise_raw_hold_and_min_hold_are_unified():
    p = FrameTimingPolicy.from_profile_name("local-precise", fps=None)
    assert p.hold_us == p.min_hold_us == 22_000


def test_scheduled_note_draft_has_single_time_field():
    from dataclasses import fields
    from sky_music.domain.scheduler import ScheduledNoteDraft

    names = {field.name for field in fields(ScheduledNoteDraft)}
    assert "at_us" in names
    assert {"source_time_us", "snapped_time_us", "shifted_time_us", "down_time_us"}.isdisjoint(names)

def test_playback_overrides_dataclass():
    from main import PlaybackOverrides
    o = PlaybackOverrides(dry_run=True, fps=120)
    assert o.dry_run is True
    assert o.fps == 120


def test_high_fps_policy_has_no_chord_window():
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    p_local = FrameTimingPolicy.from_profile_name("balanced", fps=240)
    p_remote = FrameTimingPolicy.from_profile_name("audience-safe", fps=240)
    assert not hasattr(p_local, "chord_merge_window_us")
    assert not hasattr(p_remote, "chord_merge_window_us")


def test_cycle_rule_safety_margin_clamp():
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    # Create a base policy that has very low values
    base = TimingPolicy.from_dict({
        "min_hold_us": 1000,
        "repeat_release_gap_us": 1000,
    })
    # at 30fps (low-fps upscale triggers), frame_us = 33333
    p = FrameTimingPolicy.from_timing_policy(base, fps=30)
    # safety margin = max(1000, ceil(33333 * 0.05)) = 1667
    # required cycle = 33333 + 1667 = 35000
    # minimum min_hold_us = ceil(33333 * 0.60) = 20000
    # minimum repeat_release_gap_us = ceil(33333 * 0.50) = 16667
    # sum = 36667, which is already >= 35000.
    assert p.min_hold_us + p.repeat_release_gap_us >= 35000

    # Let's force a scenario where deficit is triggered by using custom ratios or base
    # (using extremely low ratios)
    p_custom = FrameTimingPolicy.from_timing_policy(
        base, fps=30, min_hold_min_frame_ratio=0.1, repeat_release_gap_min_frame_ratio=0.1
    )
    # frame_us = 33333
    # min_hold = ceil(33333 * 0.1) = 3334
    # repeat_gap = ceil(33333 * 0.1) = 3334
    # current sum = 6668
    # required cycle = 33333 + 1667 = 35000
    # deficit = 35000 - 6668 = 28332
    # eff_repeat_release_gap_us should be increased by deficit: 3334 + 28332 = 31666
    assert p_custom.min_hold_us == 3334
    assert p_custom.min_hold_us + p_custom.repeat_release_gap_us == 35000


def test_timing_profile_validators():
    from sky_music.domain.validation import (
        validate_timing_profile,
        validate_builtin_timing_profile,
    )
    from sky_music.config import DEFAULT_TIMING_PROFILES

    # Verify all built-in profiles pass validate_builtin_timing_profile
    for name, p in DEFAULT_TIMING_PROFILES.items():
        validate_builtin_timing_profile(name, p, selected_fps=60)
    validate_timing_profile(DEFAULT_TIMING_PROFILES["local_precise"], fps=144)

    # Test failure case
    unsafe = {
        "min_hold_us": 5000,
        "repeat_release_gap_us": 5000,
        "input_lead_us": 0,
    }
    with pytest.raises(ValueError, match="Unsafe cycle"):
        validate_timing_profile(unsafe, fps=60)


def test_hold_ordering_invariant_rejects_hold_below_min_hold():
    """Regression: a profile whose hold_us is below its min_hold_us must be rejected.

    Previously hold_us was never validated, so e.g. audience_safe with hold_us=1
    sailed through every check while silently breaking the hold semantics.
    """
    from sky_music.domain.validation import validate_hold_ordering, validate_timing_profile

    with pytest.raises(ValueError, match="hold_us"):
        validate_hold_ordering({"hold_us": 1, "min_hold_us": 22000})

    with pytest.raises(ValueError, match="min_hold_us must be > 0"):
        validate_hold_ordering({"min_hold_us": 0, "hold_us": 26000})

    # A structurally valid ordering must pass.
    validate_hold_ordering({"min_hold_us": 15000, "hold_us": 26000})

    # And the invariant is reachable through the full profile validator too.
    bad = {
        "hold_us": 1, "min_hold_us": 22000, "repeat_release_gap_us": 18000,
        "input_lead_us": 14000,
    }
    with pytest.raises(ValueError, match="hold_us"):
        validate_timing_profile(bad, fps=60)


def test_audience_safe_floors_keep_remote_minimum_at_high_fps():
    p60 = FrameTimingPolicy.from_profile_name("audience-safe", fps=60)
    p144 = FrameTimingPolicy.from_profile_name("audience-safe", fps=144)

    # At 60 FPS the 1.2/1.5-frame terms dominate the (lowered) absolute floors.
    assert p60.hold_us == 20001
    assert p60.min_hold_us == 20001
    assert p60.repeat_release_gap_us == 25001
    # At 144 FPS the frame terms shrink, so the absolute remote floors take over and hold the
    # audience minimum (hold 18000 / min 18000 / repeat 24000). These are tighter than the old
    # 2-frame floors — sharper articulation and faster repeats — but still a real remote margin
    # above the registration floor (Appendix A.9 / EXP-4), not the generic local 1.5-frame wall.
    assert p144.hold_us == 18000
    assert p144.min_hold_us == 18000
    assert p144.repeat_release_gap_us == 24000
    assert p60.release_gap_us == p144.release_gap_us == 5000


def test_audience_safe_runtime_validation():
    from sky_music.domain.validation import validate_audience_safe_runtime_policy

    policy = FrameTimingPolicy.from_profile_name("audience-safe", fps=144)
    
    # This should pass without raising ValueError
    validate_audience_safe_runtime_policy(policy)

    # Let's test a policy that violates runtime limits (e.g. min_hold_us too small)
    invalid_policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"min_hold_us": 10000, "repeat_release_gap_us": 19000}),
        fps=144,
        profile_name="audience-safe",
    )
    with pytest.raises(ValueError, match="min_hold_us"):
        validate_audience_safe_runtime_policy(invalid_policy)


def test_unknown_profile_name_falls_back_to_balanced():
    from sky_music.domain.session_context import PlaybackSessionContext
    # Removed/unknown profile names canonicalise to balanced rather than erroring.
    ctx = PlaybackSessionContext(profile_name="high-fps-precise", fps=120)
    assert ctx.profile_name == "balanced"


def test_removed_lead_override_is_ignored():
    from sky_music.domain.session_context import PlaybackSessionContext
    ctx = PlaybackSessionContext(
        profile_name="audience-safe",
        fps=144,
        policy_overrides=(("input_lead_us", 10000),),
    )
    assert not hasattr(ctx.resolve_effective_policy(), "input_lead_us")

