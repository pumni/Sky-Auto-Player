from sky_music.domain.scheduler_types import FrameTimingPolicy


def test_default_margin_applies_to_both_hold_outputs() -> None:
    policy = FrameTimingPolicy.from_hold_frames(1.0, 144)

    assert policy.hold_us == policy.min_hold_us == 7_445
    assert policy.min_hold_margin_us == 500
    assert policy.min_hold_margin_source == "default_500"


def test_calibrated_margin_is_forwarded() -> None:
    policy = FrameTimingPolicy.from_hold_frames(
        1.25, 60, margin_us=800, margin_source="device_cache"
    )

    assert policy.hold_us == policy.min_hold_us == 21_634
    assert policy.min_hold_margin_us == 800
    assert policy.min_hold_margin_source == "device_cache"


def test_zero_margin_is_valid_for_one_frame() -> None:
    policy = FrameTimingPolicy.from_hold_frames(1.0, 60, margin_us=0)

    assert policy.hold_us == policy.min_hold_us == policy.frame_us


def test_calibrated_hold_margin_does_not_change_down_late_grace() -> None:
    policy = FrameTimingPolicy.from_hold_frames(1.0, 60, margin_us=1_800)

    assert policy.min_hold_margin_us == 1_800
    assert policy.down_late_grace_us == 500


def test_down_late_grace_does_not_change_materialized_hold() -> None:
    a = FrameTimingPolicy.from_hold_frames(
        1.0, 60, margin_us=800, down_late_grace_us=100
    )
    b = FrameTimingPolicy.from_hold_frames(
        1.0, 60, margin_us=800, down_late_grace_us=500
    )

    assert a.min_hold_us == b.min_hold_us


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
