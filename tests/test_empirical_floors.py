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


def test_builtin_bases_are_60fps_safe_when_frame_aware_disabled():
    # docs Non-Negotiable Rule 6: a 60fps-general profile must stay safe with scaling off.
    from sky_music.config import DEFAULT_TIMING_PROFILES

    for name in ("balanced", "local_precise", "dense_safe", "audience_safe"):
        prof = DEFAULT_TIMING_PROFILES[name]
        assert prof["min_hold_us"] >= 16667, name      # >= one 60fps frame
        assert prof["repeat_release_gap_us"] >= 18000, name


def test_repeat_release_gap_floor_config_roundtrips(tmp_path, monkeypatch):
    cfg_path = tmp_path / "config.json"
    cfg_path.write_text(
        '{"frame_timing": {"repeat_release_gap_floor_us": 20000}}', encoding="utf-8"
    )
    monkeypatch.setattr("sky_music.config.CONFIG_PATH", cfg_path)
    clear_config_cache()
    try:
        cfg = load_config(force_reload=True)
        assert cfg.frame_timing.repeat_release_gap_floor_us == 20000
        # The configured floor must flow all the way through resolve_effective_policy.
        p = PlaybackSessionContext(profile_name="balanced", fps=144).resolve_effective_policy(cfg)
        # 144fps: 1.5*frame = 10416 < configured 20000 -> the configured fixed floor wins.
        assert p.repeat_release_gap_us == 20000
    finally:
        clear_config_cache()
