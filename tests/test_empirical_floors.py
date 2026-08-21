import pytest

from sky_music.domain.hold_timing import HOLD_FRAME_OPTIONS, materialize_hold_us
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy


@pytest.mark.parametrize("fps", (30, 60, 90, 120, 144, 165, 240))
@pytest.mark.parametrize("hold_frames", HOLD_FRAME_OPTIONS)
def test_production_hold_is_frame_relative(fps: int, hold_frames: float) -> None:
    policy = FrameTimingPolicy.from_hold_frames(hold_frames, fps)

    assert policy.hold_us == policy.min_hold_us
    assert policy.hold_us == materialize_hold_us(hold_frames, fps, 800)
    assert policy.hold_us >= policy.frame_us


def test_zero_margin_one_frame_equals_one_frame() -> None:
    policy = FrameTimingPolicy.from_hold_frames(
        1.0, 60, margin_us=0, down_late_grace_us=0
    )

    assert policy.hold_us == policy.min_hold_us == policy.frame_us == 16_667


def test_negative_margin_is_rejected() -> None:
    with pytest.raises(ValueError):
        FrameTimingPolicy.from_timing_policy(
            TimingPolicy(hold_frames=1.0, min_hold_margin_us=-500), fps=60  # type: ignore[arg-type]
        )


def test_frame_policy_requires_positive_fps() -> None:
    with pytest.raises(ValueError):
        FrameTimingPolicy.from_timing_policy(TimingPolicy(), fps=0)
