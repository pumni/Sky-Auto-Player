"""Regression tests that pin the empirically-measured timing floors (docs Appendix A).

These lock the in-game-measured standard so a future change to the frame ratios/floors
fails loudly instead of silently regressing reliability:

  - visibility (hold/min_hold) floor      = 1.25 x frame   (pure frame-relative)
  - same-key release-gap floor            = max(1.5 x frame, 18000us)
  - frame-aware sizing is disabled at fps=0/None (expert/experiment escape hatch)
"""

import sys
from pathlib import Path

import pytest

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.config import AppConfig, FrameTimingDefaults, clear_config_cache, load_config
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy
from sky_music.domain.session_context import PlaybackSessionContext


def test_frame_timing_defaults_encode_empirical_standard():
    """If these change, Appendix A and the in-game measurements must be revisited."""
    d = FrameTimingDefaults()
    assert d.min_visible_hold_frames == 1.25          # hold target: 1 frame + 25% margin
    assert d.min_hold_min_frame_ratio == 1.25         # compression floor: also >= ~1 frame
    assert d.repeat_release_gap_min_frame_ratio == 1.5  # repeat gap: frame term
    assert d.repeat_release_gap_floor_us == 18000     # repeat gap: fixed ~17ms wall


@pytest.mark.parametrize(
    ("fps", "expected"),
    [(30, 50000), (60, 25001), (144, 18000)],
)
def test_repeat_gap_floor_is_max_of_frame_and_fixed(fps, expected):
    # max(1.5*frame, 18000): frame term dominates at <=60fps, fixed 18000 wall at 144fps.
    base = TimingPolicy.from_dict(
        {"min_hold_us": 1000, "repeat_release_gap_us": 1000, "hold_us": 60000}
    )
    p = FrameTimingPolicy.from_timing_policy(base, fps=fps)
    assert p.repeat_release_gap_us == expected


@pytest.mark.parametrize(
    ("fps", "expected"),
    [(30, 41667), (60, 20834), (144, 8680)],
)
def test_min_hold_visibility_floor_is_one_and_a_quarter_frames(fps, expected):
    base = TimingPolicy.from_dict(
        {"min_hold_us": 1000, "repeat_release_gap_us": 1000, "hold_us": 100000}
    )
    p = FrameTimingPolicy.from_timing_policy(base, fps=fps)
    assert p.min_hold_us == expected


def test_fps_none_keeps_raw_base_for_experiments():
    # Frame-aware disabled -> no floors applied, so timing experiments (e.g. probing the
    # in-game floor with tiny holds/gaps) remain possible.
    base = TimingPolicy.from_dict(
        {"min_hold_us": 1000, "repeat_release_gap_us": 1000, "hold_us": 2000}
    )
    p = FrameTimingPolicy.from_timing_policy(base, fps=None)
    assert p.repeat_release_gap_us == 1000
    assert p.min_hold_us == 1000
    assert p.hold_us == 2000


def test_builtin_unframed_fallbacks_remain_60fps_safe():
    # Local hold floors can be sharper with FPS set, but the no-FPS escape hatch keeps
    # conservative raw values for existing CLI/config behavior.
    from sky_music.config import DEFAULT_TIMING_PROFILES

    for name in ("balanced", "local_precise", "dense_safe", "audience_safe"):
        prof = DEFAULT_TIMING_PROFILES[name]
        assert prof.get("min_hold_unframed_us", prof["min_hold_floor_us"]) >= 16667, name
        assert prof["repeat_release_gap_floor_us"] >= 18000, name


def test_repeat_release_gap_floor_config_no_longer_overrides_frame_profiles(tmp_path, monkeypatch):
    cfg_path = tmp_path / "config.json"
    cfg_path.write_text(
        '{"frame_timing": {"repeat_release_gap_floor_us": 20000}}', encoding="utf-8"
    )
    monkeypatch.setattr("sky_music.config.CONFIG_PATH", cfg_path)
    clear_config_cache()
    try:
        cfg = load_config(force_reload=True)
        assert cfg.frame_timing.repeat_release_gap_floor_us == 20000
        p = PlaybackSessionContext(profile_name="balanced", fps=144).resolve_effective_policy(cfg)
        # Built-in profiles declare their own frame/floor repeat gap; global frame_timing is
        # retained only as fallback for legacy _us-only policies.
        assert p.repeat_release_gap_us == 18000
    finally:
        clear_config_cache()


@pytest.mark.parametrize(
    ("profile", "fps", "hold", "min_hold", "repeat_gap"),
    [
        ("local-precise", 30, 41667, 41667, 50000),
        ("local-precise", 60, 20834, 20834, 25001),
        ("local-precise", 144, 8680, 8680, 18000),
        ("dense-safe", 30, 41667, 41667, 50000),
        ("dense-safe", 60, 20834, 20834, 25001),
        ("dense-safe", 144, 11000, 11000, 18000),
        ("balanced", 30, 41667, 41667, 50000),
        ("balanced", 60, 20834, 20834, 25001),
        ("balanced", 144, 14000, 14000, 18000),
        ("audience-safe", 30, 41667, 41667, 50000),
        ("audience-safe", 60, 20834, 20834, 25001),
        ("audience-safe", 144, 20000, 18000, 24000),
    ],
)
def test_builtin_frame_profile_materialisation_matches_tuned_behavior(
    profile, fps, hold, min_hold, repeat_gap
):
    policy = PlaybackSessionContext(profile_name=profile, fps=fps).resolve_effective_policy(AppConfig())
    assert policy.hold_us == hold
    assert policy.min_hold_us == min_hold
    assert policy.repeat_release_gap_us == repeat_gap


def test_local_profile_unframed_fallback_keeps_conservative_raw_values():
    policy = PlaybackSessionContext(profile_name="balanced", fps=None).resolve_effective_policy(AppConfig())
    assert policy.frame_us == 0
    assert policy.hold_us == 26000
    assert policy.min_hold_us == 17000


def test_high_fps_local_profiles_have_distinct_hold_intents():
    cfg = AppConfig()
    local = PlaybackSessionContext(profile_name="local-precise", fps=144).resolve_effective_policy(cfg)
    balanced = PlaybackSessionContext(profile_name="balanced", fps=144).resolve_effective_policy(cfg)
    dense = PlaybackSessionContext(profile_name="dense-safe", fps=144).resolve_effective_policy(cfg)

    # local_precise is pure frame-relative (floor 0) -> sharpest at high FPS; balanced and
    # dense keep a small absolute body floor above it.
    assert local.hold_us == 8680
    assert dense.hold_us == 11000
    assert balanced.hold_us == 14000
