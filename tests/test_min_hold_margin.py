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
