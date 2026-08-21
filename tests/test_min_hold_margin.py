import pytest

from sky_music.domain.domain import Microseconds
from sky_music.domain.scheduler_types import FrameTimingPolicy


def test_default_margin_applies_to_both_hold_outputs() -> None:
    policy = FrameTimingPolicy.from_hold_frames(1.0, 144)

    assert policy.hold_us == policy.min_hold_us == 7_745
    assert policy.min_hold_margin_us == 800
    assert policy.transport_margin_us == 300
    assert policy.min_release_gap_us == 7_745
    assert policy.min_hold_margin_source == "default_transport_300"


def test_calibrated_margin_is_forwarded() -> None:
    policy = FrameTimingPolicy.from_hold_frames(
        1.25, 60, margin_us=800, margin_source="device_cache"
    )

    assert policy.hold_us == policy.min_hold_us == 22_134
    assert policy.min_hold_margin_us == 1_300
    assert policy.transport_margin_us == 800
    assert policy.min_release_gap_us == 17_967
    assert policy.min_hold_margin_source == "device_cache"


def test_zero_margin_is_valid_for_one_frame() -> None:
    policy = FrameTimingPolicy.from_hold_frames(
        1.0, 60, margin_us=0, down_late_grace_us=0
    )

    assert policy.hold_us == policy.min_hold_us == policy.frame_us
    assert policy.min_release_gap_us == policy.frame_us


@pytest.mark.parametrize(
    ("fps", "expected_frame_us", "expected_policy_us"),
    ((60, 16_667, 17_467), (120, 8_334, 9_134)),
)
def test_release_gap_reserves_the_same_static_headroom_as_hold(
    fps: int,
    expected_frame_us: int,
    expected_policy_us: int,
) -> None:
    policy = FrameTimingPolicy.from_hold_frames(1.0, fps)

    assert policy.frame_us == expected_frame_us
    assert policy.min_hold_us == expected_policy_us
    assert policy.min_release_gap_us == expected_policy_us


def test_calibrated_transport_margin_reaches_release_gap() -> None:
    policy = FrameTimingPolicy.from_hold_frames(1.0, 60, margin_us=1_000)

    assert policy.min_hold_us == 18_167
    assert policy.min_release_gap_us == 18_167


def test_calibrated_hold_margin_does_not_change_down_late_grace() -> None:
    policy = FrameTimingPolicy.from_hold_frames(1.0, 60, margin_us=1_800)

    assert policy.min_hold_margin_us == 2_300
    assert policy.transport_margin_us == 1_800
    assert policy.down_late_grace_us == 500


def test_down_late_grace_is_a_floor_for_materialized_hold() -> None:
    a = FrameTimingPolicy.from_hold_frames(
        1.0, 60, margin_us=300, down_late_grace_us=100
    )
    b = FrameTimingPolicy.from_hold_frames(
        1.0, 60, margin_us=300, down_late_grace_us=500
    )

    assert a.min_hold_margin_us == 400
    assert b.min_hold_margin_us == 800
    assert b.min_hold_us > a.min_hold_us


def test_frame_policy_rejects_an_ineffective_margin() -> None:
    with pytest.raises(ValueError, match="min_hold_margin_us"):
        FrameTimingPolicy(
            fps=60,
            frame_us=Microseconds(16_667),
            hold_frames=1.0,
            hold_us=Microseconds(16_967),
            min_hold_us=Microseconds(16_967),
            focus_restore_grace_us=Microseconds(100_000),
            min_hold_margin_us=Microseconds(300),
            down_late_grace_us=Microseconds(500),
        )


def test_calibration_does_not_shift_authored_note_on_timestamps() -> None:
    from sky_music.domain import Millis, Note, NoteKey, Song
    from sky_music.domain.scheduler import build_key_actions
    from sky_music.domain.scheduler_types import ActionKind

    song = Song(
        name="timestamp-invariant",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(1_000), key=NoteKey("Key1")),
        ),
    )
    actions_a = build_key_actions(
        song, policy=FrameTimingPolicy.from_hold_frames(1.0, 60, margin_us=500)
    ).actions
    actions_b = build_key_actions(
        song, policy=FrameTimingPolicy.from_hold_frames(1.0, 60, margin_us=1_800)
    ).actions

    downs_a = [
        (action.scan_codes, int(action.at_us))
        for action in actions_a
        if action.kind is ActionKind.DOWN
    ]
    downs_b = [
        (action.scan_codes, int(action.at_us))
        for action in actions_b
        if action.kind is ActionKind.DOWN
    ]
    assert downs_a == downs_b
