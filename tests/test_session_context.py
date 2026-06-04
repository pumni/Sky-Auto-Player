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
    assert policy.frame_us == 33_334
    assert policy.hold_us == 33_668


def test_with_profile_preserves_fps():
    session = PlaybackSessionContext(
        profile_name="balanced",
        fps=60,
    ).with_profile("audience-safe")
    assert session.profile_name == "audience-safe"
    assert session.fps == 60


def test_merge_session_with_overrides_keeps_fps_when_profile_changes():
    base = PlaybackSessionContext.balanced(fps=120)
    merged = merge_session_with_overrides(base, profile="local-precise")
    assert merged.profile_name == "local-precise"
    assert merged.fps == 120


def test_risk_profile_switch_keeps_fps():
    session = PlaybackSessionContext.balanced(fps=30)
    switched = session.with_profile("local-precise")
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


def test_balanced_at_30fps_scales_min_hold():
    session = PlaybackSessionContext.balanced(fps=30)
    policy = session.resolve_effective_policy(AppConfig())
    # 30fps frame = ceil(1e6/30) = 33334. Visibility floor (Exp1) = ceil(1.01*frame)=33668 at 30fps.
    assert policy.min_hold_us == 33_668


def test_frame_timing_config_overrides_ratios():
    cfg = AppConfig(
        frame_timing=FrameTimingDefaults(
            min_visible_hold_frames=2.0,
            min_hold_min_frame_ratio=0.25,
        )
    )
    session = PlaybackSessionContext(profile_name="local-precise", fps=30)
    policy = session.resolve_effective_policy(cfg)
    # Built-in frame-model profiles declare their own frame margins (local_precise = 1.0 frame);
    # global frame_timing ratios are retained only for legacy _us-only policies. 30fps frame =
    # ceil(1e6/30) = 33334.
    assert policy.hold_us == 33_334
    assert policy.min_hold_us == 33_334


def test_apply_recommendation_to_context_updates_session():
    from sky_music.orchestration.calibration import CalibrationRecommendation

    session = PlaybackSessionContext.balanced(tempo_scale=1.0, fps=60)
    rec = CalibrationRecommendation(
        profile_name="local-precise",
        tempo_scale=0.9,
        hold_us=30_000,
        reason="test",
        severity="moderate",
    )
    updated = apply_recommendation_to_context(session, rec)
    assert updated.profile_name == "local-precise"
    assert updated.tempo_scale == 0.9
    policy = updated.resolve_effective_policy(AppConfig())
    assert policy.hold_us > 0


def test_from_cli_args_applies_hold_override():
    import main

    parser = main.build_arg_parser()
    args = parser.parse_args(["--timing-profile", "balanced", "--hold-ms", "30", "--fps", "60"])
    session = PlaybackSessionContext.from_cli_args(args, AppConfig())
    policy = session.resolve_effective_policy(AppConfig())
    assert session.fps == 60
    assert policy.hold_us >= 30_000


def test_cli_hold_and_min_hold_overrides_keep_compression_band():
    import main
    from sky_music.domain.scheduler import plan_same_key_hold

    parser = main.build_arg_parser()
    args = parser.parse_args([
        "--timing-profile", "balanced",
        "--hold-ms", "24",
        "--min-hold-ms", "10",
    ])
    policy = PlaybackSessionContext.from_cli_args(args, AppConfig()).resolve_effective_policy(
        AppConfig()
    )

    assert policy.hold_us == 24_000
    assert policy.min_hold_us == 10_000
    planned = plan_same_key_hold(
        target_hold_us=policy.hold_us,
        min_hold_us=policy.min_hold_us,
        effective_delta_us=21_000,
    )
    assert planned.risk == "moderate"
    assert planned.hold_us == 21_000
    assert planned.compressed is True


def test_picker_lists_exactly_the_three_profiles():
    from sky_music.ui.picker import get_profiles_info
    names = [p[0] for p in get_profiles_info(120)]
    assert names == ["local-precise", "balanced", "audience-safe"]


def test_strict_timing_profile_validation_enforcement():
    # 1. Test general 60fps limit override
    cfg_unsafe = AppConfig(
        timing_profiles={
            "balanced": {
                "hold_us": 10000,
                "min_hold_us": 8000,
            }
        }
    )
    # Trying to resolve "balanced" at 60 FPS should fail validation due to min_hold_us < one frame.
    session = PlaybackSessionContext(profile_name="balanced", fps=60)
    with pytest.raises(ValueError, match="Unsafe min_hold_us|min_hold_us below 10000us"):
        session.resolve_effective_policy(cfg_unsafe)

    # 2. Audience-safe now uses the same frame visibility validation as other profiles.
    cfg_unsafe_audience = AppConfig(
        timing_profiles={
            "audience_safe": {
                "hold_us": 34000,
                "min_hold_us": 15000,
            }
        }
    )
    session_audience = PlaybackSessionContext(profile_name="audience-safe", fps=60)
    with pytest.raises(ValueError, match="Unsafe min_hold_us"):
        session_audience.resolve_effective_policy(cfg_unsafe_audience)

