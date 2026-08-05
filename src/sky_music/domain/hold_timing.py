"""Pure hold-frame selection and materialization helpers."""

from __future__ import annotations

import math

DEFAULT_HOLD_FRAMES: float = 1.0
HOLD_FRAME_OPTIONS: tuple[float, ...] = (1.0, 1.25, 1.5)


def validate_hold_frames(value: object) -> float:
    """Validate and normalize one of the supported hold-frame ratios."""
    if isinstance(value, (bool, str)):
        raise ValueError("hold_frames must be one of 1.0, 1.25, or 1.5")
    try:
        candidate = float(value)  # type: ignore[arg-type]
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError("hold_frames must be one of 1.0, 1.25, or 1.5") from exc
    if not math.isfinite(candidate) or candidate not in HOLD_FRAME_OPTIONS:
        raise ValueError("hold_frames must be one of 1.0, 1.25, or 1.5")
    return candidate


def normalize_hold_frames(value: object, default: float = DEFAULT_HOLD_FRAMES) -> float:
    """Return a supported value for persisted, potentially malformed data."""
    try:
        return validate_hold_frames(value)
    except ValueError:
        try:
            return validate_hold_frames(default)
        except ValueError:
            return DEFAULT_HOLD_FRAMES


def nearest_hold_frames(value: float) -> float:
    """Return the nearest supported ratio, breaking ties toward the larger one."""
    if isinstance(value, bool):
        raise ValueError("value must be finite")
    try:
        candidate = float(value)
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError("value must be finite") from exc
    if not math.isfinite(candidate):
        raise ValueError("value must be finite")
    return max(HOLD_FRAME_OPTIONS, key=lambda option: (-abs(option - candidate), option))


def frame_duration_us(fps: int) -> int:
    """Return the ceil-rounded duration of one game frame."""
    if isinstance(fps, bool) or not isinstance(fps, int) or fps <= 0:
        raise ValueError("fps must be a positive integer")
    return math.ceil(1_000_000 / fps)


def materialize_hold_us(hold_frames: float, fps: int, margin_us: int = 0) -> int:
    """Materialize a selected ratio using the canonical frame formula."""
    ratio = validate_hold_frames(hold_frames)
    frame_us = frame_duration_us(fps)
    return round(ratio * frame_us) + max(0, int(margin_us))


def format_hold_frames(hold_frames: float) -> str:
    """Format a stable compact label for status and telemetry output."""
    return f"hold {validate_hold_frames(hold_frames):.2f}f"
