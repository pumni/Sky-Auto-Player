"""Strict acceptance checks for unified profile + FPS timing flow (Phases 0–4)."""

import sys
from pathlib import Path

import pytest

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.config import AppConfig, clear_config_cache, apply_config_defaults, canonical_profile_name
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    merge_session_with_overrides,
)
from sky_music.domain.scheduler import build_key_actions, ScheduleBuildError
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy
from sky_music.domain.domain import Song, Note, NoteKey, Millis
from sky_music.orchestration.calibration import calibrate_profile, CalibrationInput

import main


@pytest.fixture(autouse=True)
def _reset():
    clear_config_cache()
    yield
    clear_config_cache()


def test_configure_and_session_resolve_same_policy():
    cfg = AppConfig(default_timing_profile="audience-safe", game_fps=60)
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    apply_config_defaults(args, cfg)
    main.configure_from_args(args, cfg)
    assert main.TIMING_POLICY == main.PLAYBACK_SESSION.resolve_effective_policy(cfg)


def test_play_fallback_balanced_accepts_scan_code_mode():
    session = PlaybackSessionContext.balanced(
        tempo_scale=1.0,
        fps=30,
        scan_code_mode="mapped",
    )
    assert session.scan_code_mode == "mapped"
    assert session.fps == 30


def test_play_fallback_uses_game_fps_when_no_session():
    cfg = AppConfig(game_fps=120)
    fallback = PlaybackSessionContext.balanced(
        tempo_scale=1.0,
        fps=cfg.game_fps if cfg.game_fps > 0 else None,
        scan_code_mode="physical",
    )
    assert fallback.fps == 120


def test_profile_switch_preserves_fps():
    cfg = AppConfig()
    balanced = PlaybackSessionContext.balanced(fps=120)
    dense = balanced.with_profile("dense-safe")
    assert dense.fps == 120
    p_bal = balanced.resolve_effective_policy(cfg)
    p_dense = dense.resolve_effective_policy(cfg)
    # At a fixed FPS the same-key release gap is governed by the universal empirical floor
    # (Exp2), so different profiles converge to the same value rather than differing.
    assert p_bal.repeat_release_gap_us == p_dense.repeat_release_gap_us
    assert p_bal.repeat_release_gap_us >= 18000


def test_no_orphan_timing_policy_wrap_in_src():
    """build_key_actions must not accept raw TimingPolicy (silent fps loss)."""
    song = Song("t", notes=(Note(Millis(0), NoteKey("Key0")),))
    with pytest.raises(TypeError):
        build_key_actions(song, policy=TimingPolicy.balanced())  # type: ignore[arg-type]


def test_strict_policy_aborts_build():
    song = Song(
        "t",
        notes=(
            Note(Millis(1000), NoteKey("Key0")),
            Note(Millis(1001), NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_dict({"input_lead_us": 0}),
        same_key_conflict_policy="strict",
    )
    with pytest.raises(ScheduleBuildError) as exc:
        build_key_actions(song, policy=policy)
    assert exc.value.recommended_profile
    assert exc.value.recommended_tempo_scale is not None


def test_saved_profile_name_has_no_fps_suffix():
    cfg = AppConfig(default_timing_profile="audience-safe@60fps", game_fps=60)
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    apply_config_defaults(args, cfg)
    main.configure_from_args(args, cfg)
    assert canonical_profile_name(main.PLAYBACK_SESSION.profile_name) == "audience-safe"
    assert main.TIMING_PROFILE_NAME == "audience-safe@60fps"


def test_calibration_hold_matches_resolve_path():
    inp = CalibrationInput(
        profile_name="balanced",
        tempo_scale=1.0,
        fps=30,
        p95_lateness_us=1000,
        p99_lateness_us=2000,
        p95_send_duration_us=500,
        late_over_10ms=0,
        impossible_same_key_repeats=0,
        risky_same_key_repeats=0,
        failed_release_count=0,
    )
    rec = calibrate_profile(inp)
    eff = FrameTimingPolicy.from_timing_policy(
        TimingPolicy.from_profile_name("local-precise"),
        fps=30,
    )
    assert rec.hold_us == eff.hold_us
