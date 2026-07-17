"""min_hold_margin_us: constant device-delivery margin on the frame model (round-2 plan Phase 3).

Pins the four semantic rules of the margin:
  1. frame-model materialisation adds the margin to BOTH hold_us and min_hold_us;
  2. margin 0 restores the pure ratio model bit-for-bit;
  3. explicit overrides win verbatim (no margin);
  4. the unframed fallback (fps unknown) gets no margin.
Plus the validation-mirror consistency check (domain/validation.py must materialise the
same values as FrameTimingPolicy.from_timing_policy).
"""

from __future__ import annotations

from sky_music.config import DEFAULT_TIMING_PROFILES
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy
from sky_music.domain.validation import _min_hold_us, validate_timing_profile


def test_margin_default_applies_to_frame_model() -> None:
    # local_precise @144: ceil(1e6/144)=6945, frames=1.0, margin default 500.
    policy = FrameTimingPolicy.from_profile_name("local_precise", fps=144)
    assert int(policy.min_hold_us) == 6_945 + 500
    assert int(policy.hold_us) == 6_945 + 500  # both sides get margin (ordering invariant)
    assert int(policy.min_hold_margin_us) == 500

    # local_precise @60: ceil(1e6/60)=16667.
    policy_60 = FrameTimingPolicy.from_profile_name("local_precise", fps=60)
    assert int(policy_60.min_hold_us) == 16_667 + 500


def test_margin_zero_restores_pure_ratio_model() -> None:
    timing = TimingPolicy.from_dict({"min_hold_frames": 1, "min_hold_margin_us": 0})
    policy = FrameTimingPolicy.from_timing_policy(timing, fps=144)
    assert int(policy.min_hold_us) == 6_945
    assert int(policy.hold_us) == 6_945
    assert int(policy.min_hold_margin_us) == 0


def test_explicit_override_wins_verbatim_without_margin() -> None:
    timing = TimingPolicy.from_dict({"min_hold_frames": 1.0, "min_hold_us": 9_000})
    policy = FrameTimingPolicy.from_timing_policy(timing, fps=144)
    assert int(policy.min_hold_us) == 9_000
    assert int(policy.hold_us) == 9_000
    assert int(policy.min_hold_margin_us) == 0


def test_unframed_fallback_gets_no_margin() -> None:
    timing = TimingPolicy.from_dict(
        {"min_hold_frames": 1, "min_hold_unframed_us": 22_000}
    )
    policy = FrameTimingPolicy.from_timing_policy(timing, fps=None)
    assert int(policy.min_hold_us) == 22_000
    assert int(policy.min_hold_margin_us) == 0


def test_builtin_profiles_validate_with_margin() -> None:
    for name, profile in DEFAULT_TIMING_PROFILES.items():
        for fps in (60, 144):
            validate_timing_profile(dict(profile), fps=fps)
            # Mirror consistency: validation must materialise the same min_hold the
            # runtime policy uses (scheduler_types comment pins the two computations).
            mirrored = _min_hold_us(dict(profile), fps=fps)
            resolved = FrameTimingPolicy.from_profile_name(name, fps=fps)
            assert mirrored == int(resolved.min_hold_us), (name, fps)
