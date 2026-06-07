"""Regression tests that pin the frame-relative timing model.

These lock the in-game-measured standard so a future change to the frame ratios
fails loudly instead of silently regressing reliability:

  - visibility (hold/min_hold) target     = profile frames (pure frame-relative)
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


@pytest.mark.parametrize(
    ("fps", "expected"),
    [(30, 41668), (60, 20834), (144, 8682)],
)
def test_min_hold_visibility_floor_is_one_and_a_quarter_frames(fps, expected):
    base = TimingPolicy.from_dict(
        {"min_hold_us": 1000, "hold_us": 100000}
    )
    p = FrameTimingPolicy.from_timing_policy(base, fps=fps)
    assert p.min_hold_us == expected


def test_fps_none_keeps_raw_base_for_experiments():
    # Frame-aware disabled -> no floors applied, so timing experiments (e.g. probing the
    # in-game floor with tiny holds) remain possible.
    base = TimingPolicy.from_dict(
        {"min_hold_us": 1000, "hold_us": 2000}
    )
    p = FrameTimingPolicy.from_timing_policy(base, fps=None)
    assert p.min_hold_us == 1000
    assert p.hold_us == 2000


def test_builtin_unframed_fallbacks_remain_60fps_safe():
    # Local holds can be sharper with FPS set, but the no-FPS escape hatch keeps
    # conservative raw values for existing CLI/config behavior.
    from sky_music.config import DEFAULT_TIMING_PROFILES

    for name in ("balanced", "local_precise", "audience_safe"):
        prof = DEFAULT_TIMING_PROFILES[name]
        assert prof["min_hold_unframed_us"] >= 16667, name


def test_removed_repeat_release_gap_config_is_ignored(tmp_path, monkeypatch):
    cfg_path = tmp_path / "config.json"
    cfg_path.write_text(
        '{"frame_timing": {"repeat_release_gap_floor_us": 20000}}', encoding="utf-8"
    )
    monkeypatch.setattr("sky_music.config.CONFIG_PATH", cfg_path)
    clear_config_cache()
    try:
        cfg = load_config(force_reload=True)
        p = PlaybackSessionContext(profile_name="balanced", fps=144).resolve_effective_policy(cfg)
        assert not hasattr(cfg.frame_timing, "repeat_release_gap_floor_us")
        assert not hasattr(p, "repeat_release_gap_us")
    finally:
        clear_config_cache()

def test_local_profile_unframed_fallback_keeps_conservative_raw_values():
    policy = PlaybackSessionContext(profile_name="balanced", fps=None).resolve_effective_policy(AppConfig())
    assert policy.frame_us == 0
    assert policy.hold_us == policy.min_hold_us == 17000
