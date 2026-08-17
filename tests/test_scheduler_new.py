import pytest

from sky_music.domain import Millis, Note, NoteKey, Song
from sky_music.domain.domain import Microseconds, ScanCode
from sky_music.domain.hold_timing import HOLD_FRAME_OPTIONS
from sky_music.domain.scheduler import (
    ScheduleBuildError,
    ScheduledNoteDraft,
    build_key_actions,
    normalise_note_drafts,
    plan_same_key_hold,
)
from sky_music.domain.scheduler_types import (
    ActionKind,
    FrameTimingPolicy,
    KeyAction,
    TimingPolicy,
)
from sky_music.domain.validation import validate_key_actions


def _policy(
    hold_frames: float = 1.0,
    fps: int = 60,
    *,
    margin_us: int = 500,
    same_key_conflict_policy: str = "drop_chord",
) -> FrameTimingPolicy:
    return FrameTimingPolicy.from_hold_frames(
        hold_frames,
        fps,
        margin_us=margin_us,
        same_key_conflict_policy=same_key_conflict_policy,  # type: ignore[arg-type]
    )


def test_normalise_note_drafts_deduplicates_same_key_slots() -> None:
    drafts = [
        ScheduledNoteDraft(1_000_000, NoteKey("Key0"), 0x15, 0),
        ScheduledNoteDraft(1_000_000, NoteKey("Key1"), 0x16, 1),
        ScheduledNoteDraft(1_000_000, NoteKey("Key0"), 0x15, 2),
    ]

    normalized = normalise_note_drafts(drafts)
    assert [(draft.at_us, draft.scan_code) for draft in normalized] == [
        (1_000_000, 0x15),
        (1_000_000, 0x16),
    ]


def test_plan_same_key_hold_reports_compression_and_infeasibility() -> None:
    moderate = plan_same_key_hold(
        target_hold_us=20_000, min_hold_us=10_000, effective_delta_us=15_000
    )
    severe = plan_same_key_hold(
        target_hold_us=10_000, min_hold_us=10_000, effective_delta_us=5_000
    )

    assert (moderate.hold_us, moderate.risk, moderate.compressed) == (15_000, "moderate", True)
    assert (severe.hold_us, severe.risk, severe.compressed) == (10_000, "severe", False)


def test_chord_batching_and_deduplication() -> None:
    song = Song(
        "Test Chord",
        notes=(
            Note(Millis(1000), NoteKey("Key0")),
            Note(Millis(1000), NoteKey("Key1")),
            Note(Millis(1000), NoteKey("Key0")),
        ),
    )

    result = build_key_actions(song, policy=_policy())
    downs = [action for action in result.actions if action.kind == "down"]

    assert len(downs) == 1
    assert set(downs[0].scan_codes) == {0x15, 0x16}
    assert result.impossible_same_key_repeats == 0
    assert result.deduplicated_note_count == 2
    assert result.duplicate_note_count == 1


def test_same_key_repeat_diagnostics_use_selected_effective_hold() -> None:
    song = Song(
        "Extreme Speed",
        notes=(
            Note(Millis(1000), NoteKey("Key0")),
            Note(Millis(1001), NoteKey("Key0")),
        ),
    )

    result = build_key_actions(song, policy=_policy(1.0, 60))
    assert result.impossible_same_key_repeats == 1
    assert result.recommended_hold_frames == 1.0
    assert result.recommended_tempo_scale is not None


def test_strict_policy_rejects_impossible_repeat_with_hold_recommendation() -> None:
    song = Song(
        "Strict",
        notes=(
            Note(Millis(1000), NoteKey("Key0")),
            Note(Millis(1001), NoteKey("Key0")),
        ),
    )

    with pytest.raises(ScheduleBuildError) as exc:
        build_key_actions(song, policy=_policy(1.5, 60, same_key_conflict_policy="strict"))

    assert exc.value.recommended_hold_frames == 1.0
    assert exc.value.recommended_tempo_scale is not None


def test_release_precedes_new_down_at_same_timestamp() -> None:
    song = Song(
        "Boundary",
        notes=(
            Note(Millis(1000), NoteKey("Key0")),
            Note(Millis(1010), NoteKey("Key0")),
        ),
    )
    result = build_key_actions(song, policy=_policy(1.0, 100, margin_us=0))
    boundary = [action for action in result.actions if action.at_us == 1_010_000]

    assert [action.kind.value for action in boundary[:2]] == ["up", "down"]


@pytest.mark.parametrize("fps", (30, 60, 90, 120, 144, 165, 240))
@pytest.mark.parametrize("hold_frames", HOLD_FRAME_OPTIONS)
def test_all_hold_choices_materialize_to_equal_hold_and_min_hold(
    fps: int, hold_frames: float
) -> None:
    policy = _policy(hold_frames, fps)
    assert policy.hold_us == policy.min_hold_us
    assert policy.hold_us >= policy.frame_us


def test_zero_margin_one_frame_is_valid_in_schedule_validation() -> None:
    policy = _policy(1.0, 60, margin_us=0)
    actions = (
        KeyAction(ActionKind.DOWN, (ScanCode(0x15),), Microseconds(0)),
        KeyAction(ActionKind.UP, (ScanCode(0x15),), policy.frame_us),
    )

    assert validate_key_actions(actions, policy=policy) == ()


def test_validation_rejects_hold_shorter_than_one_frame() -> None:
    policy = _policy(1.0, 60, margin_us=0)
    actions = (
        KeyAction(ActionKind.DOWN, (ScanCode(0x15),), Microseconds(0)),
        KeyAction(ActionKind.UP, (ScanCode(0x15),), Microseconds(policy.frame_us - 1)),
    )

    violations = validate_key_actions(actions, policy=policy)
    assert any(violation.code == "insufficient_hold" for violation in violations)


def test_timing_policy_construction_is_explicit_and_typed() -> None:
    policy = TimingPolicy(hold_frames=1.25)
    resolved = FrameTimingPolicy.from_timing_policy(policy, 60)

    assert resolved.hold_frames == 1.25
    assert resolved.fps == 60
    assert resolved.hold_us == resolved.min_hold_us == 21_334
