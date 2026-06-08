import sys
from pathlib import Path
import pytest

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.domain import Song, Note, NoteKey, Millis
from sky_music.domain.scheduler import (
    ScheduledNoteDraft,
    ScheduleBuildError,
    build_key_actions,
    normalise_note_drafts,
    plan_same_key_hold,
)
from sky_music.domain.scheduler_types import TimingPolicy, KeyAction, FrameTimingPolicy, Microseconds

def _policy(d: dict | None = None) -> FrameTimingPolicy:
    return FrameTimingPolicy.from_timing_policy(TimingPolicy.from_dict(d or {}))


def test_normalise_note_drafts_preserves_chords_but_dedupes_same_key_slots():
    drafts = [
        ScheduledNoteDraft(at_us=1_000_000, note_key=NoteKey("Key0"), scan_code=0x15, source_index=0),
        ScheduledNoteDraft(at_us=1_000_000, note_key=NoteKey("Key1"), scan_code=0x16, source_index=1),
        ScheduledNoteDraft(at_us=1_000_000, note_key=NoteKey("Key0"), scan_code=0x15, source_index=2),
    ]

    normalised = normalise_note_drafts(drafts)

    assert [(d.at_us, d.scan_code) for d in normalised] == [
        (1_000_000, 0x15),
        (1_000_000, 0x16),
    ]


def test_plan_same_key_hold_reports_same_key_overlap_compression():
    planned = plan_same_key_hold(
        target_hold_us=20_000,
        min_hold_us=10_000,
        effective_delta_us=15_000,
    )

    assert planned.hold_us == 15_000
    assert planned.risk == "moderate"
    assert planned.compressed is True


def test_plan_same_key_hold_reports_min_hold_violation():
    planned = plan_same_key_hold(
        target_hold_us=10_000,
        min_hold_us=10_000,
        effective_delta_us=5_000,
    )

    assert planned.hold_us == 10_000
    assert planned.risk == "severe"
    assert planned.compressed is False


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
    policy = _policy()
    res = build_key_actions(song, policy=policy)
    down_actions = [a for a in res.actions if a.kind == "down"]
    assert len(down_actions) == 1
    assert set(down_actions[0].scan_codes) == {0x15, 0x16}
    assert res.impossible_same_key_repeats == 0
    assert res.deduplicated_note_count == 2
    assert res.duplicate_note_count == 1


def test_same_key_duplicate_at_same_timestamp_is_not_a_repeat():
    song = Song(
        name="Duplicate Same Key",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
        ),
    )

    res = build_key_actions(song, policy=_policy())
    downs = [a for a in res.actions if a.kind == "down"]

    assert len(downs) == 1
    assert downs[0].scan_codes == (0x15,)
    assert res.impossible_same_key_repeats == 0
    assert res.shortest_same_key_interval_us is None
    assert res.deduplicated_note_count == 1
    assert res.duplicate_note_count == 1

def test_third_instrument_key_schedules_as_base_key():
    song = Song(
        name="Third Instrument",
        notes=(Note(time_ms=Millis(1000), key=NoteKey("3Key5")),)
    )
    policy = _policy()
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
        "hold_us": 20_000, "min_hold_us": 10_000,
    })
    res = build_key_actions(song, policy=policy)
    actions = res.actions
    assert len(actions) == 4
    assert actions[0].at_us == 1000_000 # Down 1
    assert actions[1].at_us == 1015_000 # Up 1 at next same-key down
    assert actions[1].kind == "up"
    assert actions[1].reason == "repeat_release"
    assert actions[2].at_us == 1015_000 # Down 2
    assert actions[3].at_us == 1035_000 # Up 2 (1015 + 20ms hold)
    assert res.compressed_holds == 1
    assert res.same_key_compressed_holds == 1
    assert res.min_same_key_up_gap_us == 0


def test_same_key_repeat_compresses_hold_to_next_down():
    song = Song(
        name="Compression Band",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1015), key=NoteKey("Key0")),
        ),
    )
    policy = _policy({
        "hold_us": 20_000,
        "min_hold_us": 10_000,
    })

    res = build_key_actions(song, policy=policy)
    first_up = next(a for a in res.actions if a.kind == "up" and a.reason == "repeat_release")
    second_down = [a for a in res.actions if a.kind == "down"][1]

    assert first_up.at_us == 1_015_000
    assert second_down.at_us - first_up.at_us == 0
    assert res.compressed_holds == 1
    assert res.same_key_compressed_holds == 1
    assert res.risky_same_key_repeats == 1
    assert res.impossible_same_key_repeats == 0
    assert res.infeasible_same_key_repeats == 0
    assert res.min_same_key_up_gap_us == 0


def test_same_key_repeat_above_min_hold_is_not_impossible():
    song = Song(
        name="No compression band",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1015), key=NoteKey("Key0")),
        ),
    )
    policy = _policy({
        "hold_us": 10_000,
        "min_hold_us": 10_000,
    })

    res = build_key_actions(song, policy=policy)
    first_up = next(a for a in res.actions if a.kind == "up")
    second_down = [a for a in res.actions if a.kind == "down"][1]

    assert first_up.at_us == 1_010_000
    assert second_down.at_us - first_up.at_us == 5_000
    assert res.compressed_holds == 0
    assert res.impossible_same_key_repeats == 0
    assert res.infeasible_same_key_repeats == 0
    assert res.min_same_key_up_gap_us == 5_000
    assert res.diagnostics == ()


def test_prioritization_at_same_timestamp():
    """Verify key event scheduling priorities when multiple events fall on the exact same microsecond."""
    a_down = KeyAction(at_us=1000, scan_codes=(0x15,), kind="down", reason="onset")
    a_up_repeat = KeyAction(at_us=1000, scan_codes=(0x15,), kind="up", reason="repeat_release")
    a_up_normal = KeyAction(at_us=1000, scan_codes=(0x16,), kind="up", reason="release")
    
    unsorted = [a_up_normal, a_down, a_up_repeat]
    def action_priority(action: KeyAction) -> int:
        return 0 if action.kind == "up" else 1
    sorted_actions = sorted(unsorted, key=lambda a: (a.at_us, action_priority(a)))
    assert {sorted_actions[0], sorted_actions[1]} == {a_up_repeat, a_up_normal}
    assert sorted_actions[2] == a_down


def test_normal_release_is_sorted_before_down_at_same_timestamp():
    song = Song(
        name="Release Boundary",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1010), key=NoteKey("Key1")),
        ),
    )
    policy = _policy({"hold_us": 10_000, "min_hold_us": 10_000})

    res = build_key_actions(song, policy=policy)
    boundary = [a for a in res.actions if int(a.at_us) == 1_010_000]

    assert [(a.kind, a.reason) for a in boundary] == [
        ("up", "release"),
        ("down", "onset"),
    ]

def test_impossible_same_key_repeat_diagnostics():
    """Verify extremely fast repeats trigger correct diagnostics without crashing."""
    song = Song(
        name="Extreme Speed",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")),
        )
    )
    policy = _policy({"min_hold_us": 10000})
    res = build_key_actions(song, policy=policy)
    assert res.impossible_same_key_repeats == 1
    assert res.infeasible_same_key_repeats == 1
    assert res.min_same_key_up_gap_us == -9_000
    up_action = next(a for a in res.actions if a.kind == "up" and 0x15 in a.scan_codes)
    assert up_action.at_us == 1010_000 # 1000ms + 10ms min_hold

def test_scheduler_fails_on_unmapped_note_key():
    song = Song(name="Invalid", notes=(Note(time_ms=Millis(1000), key=NoteKey("Key999")),))
    with pytest.raises(ValueError, match="Cannot map note key 'Key999'"):
        build_key_actions(song)

def test_frame_timing_policy_has_no_release_gap_field():
    frame_policy = FrameTimingPolicy.from_profile_name("balanced", fps=30)
    assert not hasattr(frame_policy, "release_gap_us")
    assert not hasattr(frame_policy, "min_visible_hold_frames")

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
    res = build_key_actions(song, policy=_policy())
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

def test_min_hold_scales_at_30fps():
    base = TimingPolicy.from_dict({"min_hold_us": 16000})
    frame_policy = FrameTimingPolicy.from_timing_policy(base, fps=30)
    # 30fps frame = ceil(1e6/30) = 33334us. Visibility floor (Exp1) = max(base 16000,
    # ceil(1.25*frame)=41668) = 41668.
    assert frame_policy.min_hold_us == 41668

def test_strict_policy_rejects_impossible_repeat():
    song = Song(
        name="Strict Fail",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({}),
        same_key_conflict_policy="strict",
    )
    with pytest.raises(ScheduleBuildError) as exc_info:
        build_key_actions(song, policy=policy)
    assert exc_info.value.recommended_profile == "local-precise"
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
        TimingPolicy.from_dict({}),
        same_key_conflict_policy="degraded",
    )
    res = build_key_actions(song, policy=policy)
    assert res.impossible_same_key_repeats == 1


def test_degraded_impossible_repeat_validates_duplicate_down_as_warning():
    from sky_music.domain.validation import validate_key_actions

    song = Song(
        name="Degraded",
        notes=(
            Note(time_ms=Millis(1000), key=NoteKey("Key0")),
            Note(time_ms=Millis(1001), key=NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"hold_us": 10_000, "min_hold_us": 10_000}),
        same_key_conflict_policy="degraded",
    )

    res = build_key_actions(song, policy=policy)
    violations = validate_key_actions(res.actions, policy=policy)
    duplicate_downs = [v for v in violations if v.code == "duplicate_down"]

    assert duplicate_downs
    assert {v.severity for v in duplicate_downs} == {"warning"}


def test_strict_validation_keeps_duplicate_down_fatal():
    from sky_music.domain.validation import validate_key_actions

    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({}),
        same_key_conflict_policy="strict",
    )
    actions = (
        KeyAction(at_us=Microseconds(1_000), scan_codes=(0x15,), kind="down", reason="onset"),
        KeyAction(at_us=Microseconds(1_001), scan_codes=(0x15,), kind="down", reason="onset"),
    )

    violations = validate_key_actions(actions, policy=policy)
    duplicate_downs = [v for v in violations if v.code == "duplicate_down"]

    assert duplicate_downs
    assert {v.severity for v in duplicate_downs} == {"fatal"}

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
    assert policy.hold_us == policy.min_hold_us == 17000


def test_timing_policy_without_hold_declaration_mirrors_min_hold_model():
    policy = TimingPolicy.from_dict({
        "min_hold_frames": 1.1,
        "min_hold_floor_us": 12_000,
        "min_hold_unframed_us": 18_000,
    })

    assert policy.hold_us == policy.min_hold_us == 18_000
    assert policy.hold_frames == policy.min_hold_frames == 1.1
    assert not hasattr(policy, "hold_floor_us")
    assert not hasattr(policy, "min_hold_floor_us")
    assert policy.hold_override_us == policy.min_hold_override_us
    assert policy.hold_uses_frame_model is policy.min_hold_uses_frame_model is True


@pytest.mark.parametrize(
    ("hold_declaration", "expected_raw_hold"),
    [
        ({"hold_us": 24_000}, 24_000),
        ({"hold_frames": 2.0}, 17_000),
        ({"hold_unframed_us": 24_000}, 24_000),
    ],
)
def test_explicit_hold_declarations_remain_escape_hatches(
    hold_declaration, expected_raw_hold
):
    policy = TimingPolicy.from_dict({
        "min_hold_frames": 1.2,
        "min_hold_floor_us": 14_000,
        "min_hold_unframed_us": 17_000,
        **hold_declaration,
    })

    assert policy.hold_us == expected_raw_hold
    assert policy.min_hold_us == 17_000

    if "hold_frames" in hold_declaration:
        effective = FrameTimingPolicy.from_timing_policy(policy, fps=60)
        assert effective.hold_us == 33_334
        assert effective.min_hold_us == 20_000

def test_timing_policy_from_profile_name():
    policy = TimingPolicy.from_profile_name("local-precise")
    assert policy.hold_us == 22000
    policy_2 = TimingPolicy.from_profile_name("audience-safe")
    assert policy_2.hold_us == 18000

def test_frame_timing_policy_from_profile_name():
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    p = FrameTimingPolicy.from_profile_name("balanced", fps=60)
    assert p.fps == 60


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

def test_min_hold_frame_floor_clamp():
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    # Create a base policy that has very low values
    base = TimingPolicy.from_dict({
        "min_hold_us": 1000,
    })
    # at 30fps (low-fps upscale triggers), frame_us = ceil(1e6/30) = 33334
    p = FrameTimingPolicy.from_timing_policy(base, fps=30)
    assert p.min_hold_us == 41668

    p_custom = FrameTimingPolicy.from_timing_policy(
        base, fps=30, min_hold_min_frame_ratio=1.05
    )
    assert p_custom.min_hold_us == 35001

def test_timing_profile_validators():
    from sky_music.domain.validation import (
        validate_timing_profile,
        validate_builtin_timing_profile,
    )
    from sky_music.config import DEFAULT_TIMING_PROFILES

    # Verify all built-in profiles pass validate_builtin_timing_profile
    for name, p in DEFAULT_TIMING_PROFILES.items():
        validate_builtin_timing_profile(name, p, selected_fps=60)
        assert not any(
            key in p
            for key in ("hold_frames", "hold_floor_us", "hold_unframed_us")
        )
    with pytest.raises(ValueError, match="Unsafe min_hold_us"):
        validate_builtin_timing_profile("custom", {"min_hold_us": 20000}, selected_fps=30)
    validate_timing_profile(DEFAULT_TIMING_PROFILES["local_precise"], fps=144)

    # Test failure case
    unsafe = {
        "min_hold_us": 5000,
    }
    with pytest.raises(ValueError, match="Unsafe min_hold_us"):
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
        "hold_us": 1, "min_hold_us": 22000,
    }
    with pytest.raises(ValueError, match="hold_us"):
        validate_timing_profile(bad, fps=60)

def test_removed_floor_keys_are_ignored():
    policy = TimingPolicy.from_dict({
        "min_hold_frames": 1.2,
        "min_hold_floor_us": 99_000,
        "min_hold_unframed_us": 17_000,
    })
    effective = FrameTimingPolicy.from_timing_policy(policy, fps=144)
    assert effective.hold_us == effective.min_hold_us == 8334

def test_unknown_profile_name_falls_back_to_balanced():
    from sky_music.domain.session_context import PlaybackSessionContext
    # Removed/unknown profile names canonicalise to balanced rather than erroring.
    ctx = PlaybackSessionContext(profile_name="high-fps-precise", fps=120)
    assert ctx.profile_name == "balanced"

def test_degraded_same_key_behavior_timeline():
    song = Song(
        name="DegradedTimeline",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(5), key=NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"hold_us": 20_000, "min_hold_us": 10_000}),
        same_key_conflict_policy="degraded",
    )
    res = build_key_actions(song, policy=policy)
    
    actions = res.actions
    assert len(actions) == 4
    
    assert actions[0].kind == "down"
    assert actions[0].at_us == 0
    
    assert actions[1].kind == "down"
    assert actions[1].at_us == 5000
    
    assert actions[2].kind == "up"
    assert actions[2].at_us == 10000
    
    assert actions[3].kind == "up"
    assert actions[3].at_us == 25000
