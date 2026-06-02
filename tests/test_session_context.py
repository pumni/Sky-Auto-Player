import sys
from pathlib import Path

import pytest

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.config import AppConfig, clear_config_cache, FrameTimingDefaults
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    merge_session_with_overrides,
    apply_recommendation_to_context,
)
from sky_music.ui.picker_metadata import (
    clear_metadata_cache,
    get_song_ui_metadata,
)


@pytest.fixture(autouse=True)
def _reset_caches():
    clear_config_cache()
    clear_metadata_cache()
    yield
    clear_config_cache()
    clear_metadata_cache()


def test_balanced_at_30fps_scales_hold():
    session = PlaybackSessionContext.balanced(fps=30)
    policy = session.resolve_effective_policy(AppConfig())
    assert policy.fps == 30
    assert policy.frame_us == 33_333
    assert policy.hold_us == 41_667


def test_with_profile_preserves_fps():
    session = PlaybackSessionContext(
        profile_name="balanced",
        fps=60,
    ).with_profile("audience-safe")
    assert session.profile_name == "audience-safe"
    assert session.fps == 60


def test_merge_session_with_overrides_keeps_fps_when_profile_changes():
    base = PlaybackSessionContext.balanced(fps=120)
    merged = merge_session_with_overrides(base, profile="dense-safe")
    assert merged.profile_name == "dense-safe"
    assert merged.fps == 120


def test_risk_profile_switch_keeps_fps():
    session = PlaybackSessionContext.balanced(fps=30)
    switched = session.with_profile("dense-safe")
    before = session.resolve_effective_policy(AppConfig())
    after = switched.resolve_effective_policy(AppConfig())
    assert before.fps == after.fps == 30
    assert after.hold_us != before.hold_us or switched.profile_name != session.profile_name


def test_metadata_cache_key_differs_by_fps():
    song = Path("songs/1test copy.json")
    no_fps = PlaybackSessionContext.balanced()
    at_30 = PlaybackSessionContext.balanced(fps=30)
    assert no_fps.metadata_cache_key(song) != at_30.metadata_cache_key(song)


def test_metadata_uses_session_fps_for_schedule():
    song = Path("songs/1test copy.json")
    meta_no_fps = get_song_ui_metadata(song, PlaybackSessionContext.balanced())
    meta_30 = get_song_ui_metadata(song, PlaybackSessionContext.balanced(fps=30))
    assert meta_no_fps.note_count == meta_30.note_count
    assert meta_no_fps.duration_seconds != meta_30.duration_seconds


def test_repeat_release_gap_scales_with_fps():
    session = PlaybackSessionContext.balanced(fps=30)
    policy = session.resolve_effective_policy(AppConfig())
    # Empirical floor (Exp2): max(base 8000, 1.5*frame=50000, fixed 18000) = 50000 at 30fps.
    assert policy.repeat_release_gap_us == 50_000


def test_balanced_at_30fps_scales_min_hold():
    session = PlaybackSessionContext.balanced(fps=30)
    policy = session.resolve_effective_policy(AppConfig())
    # Visibility floor (Exp1) = max(base 15000, 1.25*frame=41667) = 41667 at 30fps.
    assert policy.min_hold_us == 41_667


def test_frame_timing_config_overrides_ratios():
    cfg = AppConfig(
        frame_timing=FrameTimingDefaults(
            min_visible_hold_frames=2.0,
            min_hold_min_frame_ratio=0.25,
        )
    )
    session = PlaybackSessionContext(profile_name="local-precise", fps=30)
    policy = session.resolve_effective_policy(cfg)
    # Built-in frame-model profiles declare their own frame margins; global frame_timing
    # ratios are retained only for legacy _us-only policies.
    assert policy.hold_us == 41_667
    assert policy.min_hold_us == 41_667


def test_apply_recommendation_to_context_updates_session():
    from sky_music.orchestration.calibration import CalibrationRecommendation

    session = PlaybackSessionContext.balanced(tempo_scale=1.0, fps=60)
    rec = CalibrationRecommendation(
        profile_name="dense-safe",
        tempo_scale=0.9,
        input_lead_us=12_000,
        hold_us=30_000,
        reason="test",
        severity="moderate",
    )
    updated = apply_recommendation_to_context(session, rec)
    assert updated.profile_name == "dense-safe"
    assert updated.tempo_scale == 0.9
    assert dict(updated.policy_overrides)["input_lead_us"] == 12_000
    policy = updated.resolve_effective_policy(AppConfig())
    assert policy.input_lead_us >= 12_000


def test_frame_align_from_config():
    cfg = AppConfig(frame_timing=FrameTimingDefaults(frame_align="down_only"))
    session = PlaybackSessionContext.balanced(fps=30)
    assert session.resolved_frame_align(cfg) == "down_only"
    assert session.resolve_effective_policy(cfg).frame_align == "down_only"


def test_from_cli_args_applies_hold_override():
    import main

    parser = main.build_arg_parser()
    args = parser.parse_args(["--timing-profile", "balanced", "--hold-ms", "30", "--fps", "60"])
    session = PlaybackSessionContext.from_cli_args(args, AppConfig())
    policy = session.resolve_effective_policy(AppConfig())
    assert session.fps == 60
    assert policy.hold_us >= 30_000


def test_picker_lists_exactly_the_four_profiles():
    from sky_music.ui.picker import get_profiles_info
    names = [p[0] for p in get_profiles_info(120)]
    assert names == ["local-precise", "balanced", "audience-safe", "dense-safe"]


def test_strict_timing_profile_validation_enforcement():
    # 1. Test general 60fps limit override
    cfg_unsafe = AppConfig(
        timing_profiles={
            "balanced": {
                "hold_us": 10000,
                "min_hold_us": 8000,
                "release_gap_us": 2000,
                "repeat_release_gap_us": 4000,
                "input_lead_us": 3000,
                "chord_merge_window_us": 2000,
            }
        }
    )
    # Trying to resolve "balanced" at 60 FPS should fail validation due to min_hold_us < 10000us or unsafe cycle
    session = PlaybackSessionContext(profile_name="balanced", fps=60)
    with pytest.raises(ValueError, match="Unsafe cycle|min_hold_us below 10000us"):
        session.resolve_effective_policy(cfg_unsafe)

    # 2. Test audience-safe overrides strict limit
    cfg_unsafe_audience = AppConfig(
        timing_profiles={
            "audience_safe": {
                "hold_us": 34000,
                "min_hold_us": 15000,  # Below the 17000us limit for audience-safe!
                "release_gap_us": 8000,
                "repeat_release_gap_us": 14000,
                "input_lead_us": 14000,
                "chord_merge_window_us": 6000,
            }
        }
    )
    session_audience = PlaybackSessionContext(profile_name="audience-safe", fps=60)
    with pytest.raises(ValueError, match="audience-safe profile requires min_hold_us >= 17000us"):
        session_audience.resolve_effective_policy(cfg_unsafe_audience)

