import math

import pytest

from sky_music.config import VALID_FPS
from sky_music.domain.hold_timing import (
    HOLD_FRAME_OPTIONS,
    frame_base_hold_us,
    frame_duration_us,
    materialize_hold_us,
    nearest_hold_frames,
    normalize_hold_frames,
    validate_hold_frames,
)


@pytest.mark.parametrize("value", HOLD_FRAME_OPTIONS)
def test_supported_hold_values(value: float) -> None:
    assert validate_hold_frames(value) == value


@pytest.mark.parametrize("value", [True, False, 1.1, 2.0, math.nan, math.inf, "1.0"])
def test_invalid_hold_values_are_rejected(value: object) -> None:
    with pytest.raises(ValueError):
        validate_hold_frames(value)


def test_persisted_normalization_falls_back() -> None:
    assert normalize_hold_frames("bad") == 1.0
    assert normalize_hold_frames(2.0, default=1.5) == 1.5


@pytest.mark.parametrize(("value", "expected"), [(1.02, 1.0), (1.12, 1.0), (1.20, 1.25), (1.375, 1.5), (1.49, 1.5)])
def test_nearest_hold_frames(value: float, expected: float) -> None:
    assert nearest_hold_frames(value) == expected


def test_frame_duration_and_zero_margin() -> None:
    assert frame_duration_us(60) == 16_667
    assert materialize_hold_us(1.0, 60, 0) == frame_duration_us(60)
    assert materialize_hold_us(1.0, 60, -100) == frame_duration_us(60)


@pytest.mark.parametrize("fps", VALID_FPS)
@pytest.mark.parametrize("hold", HOLD_FRAME_OPTIONS)
def test_materialization_matrix(fps: int, hold: float) -> None:
    value = materialize_hold_us(hold, fps, 500)
    assert value == frame_base_hold_us(hold, fps) + 500
