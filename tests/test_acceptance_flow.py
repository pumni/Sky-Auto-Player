import pytest

import main
from sky_music.config import AppConfig, apply_config_defaults
from sky_music.domain.domain import Millis, Note, NoteKey, Song
from sky_music.domain.scheduler import ScheduleBuildError, build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.orchestration.calibration import CalibrationInput, calibrate_timing


def test_configured_hold_and_fps_reach_the_session() -> None:
    cfg = AppConfig(default_hold_frames=1.5, game_fps=120)
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    apply_config_defaults(args, cfg)

    session = PlaybackSessionContext.from_cli_args(args, cfg)
    assert session.hold_frames == 1.5
    assert session.fps == 120


def test_strict_policy_recommends_short_hold_for_impossible_repeat() -> None:
    song = Song(
        "t",
        notes=(
            Note(Millis(1000), NoteKey("Key0")),
            Note(Millis(1001), NoteKey("Key0")),
        ),
    )
    policy = FrameTimingPolicy.from_hold_frames(
        1.5, 60, same_key_conflict_policy="strict"
    )

    with pytest.raises(ScheduleBuildError) as exc:
        build_key_actions(song, policy=policy)

    assert exc.value.recommended_hold_frames == 1.0
    assert exc.value.recommended_tempo_scale is not None


def test_calibration_recommendation_uses_hold_frames() -> None:
    rec = calibrate_timing(
        CalibrationInput(
            hold_frames=1.25,
            tempo_scale=1.0,
            fps=60,
            p95_lateness_us=0,
            p99_lateness_us=0,
            p95_send_duration_us=0,
            late_over_10ms=0,
            impossible_same_key_repeats=0,
            risky_same_key_repeats=0,
            failed_release_count=0,
        )
    )

    assert rec.hold_frames == 1.25
    assert rec.recommended_hold_us == 21_334


def test_calibration_repeat_stress_wins_over_polyphony() -> None:
    rec = calibrate_timing(
        CalibrationInput(
            hold_frames=1.5,
            tempo_scale=1.0,
            fps=60,
            p95_lateness_us=0,
            p99_lateness_us=0,
            p95_send_duration_us=0,
            late_over_10ms=0,
            impossible_same_key_repeats=0,
            risky_same_key_repeats=1,
            failed_release_count=0,
            max_polyphony=5,
        )
    )

    assert rec.hold_frames == 1.0


def test_calibration_uses_summary_device_margin() -> None:
    from sky_music.orchestration.calibration import calibration_input_from_summary

    inp = calibration_input_from_summary(
        {
            "hold_frames": 1.0,
            "fps": 60,
            "min_hold_margin_us": 800,
            "min_hold_margin_source": "device_cache",
        }
    )
    rec = calibrate_timing(inp)

    assert rec.recommended_hold_us == 17_467
