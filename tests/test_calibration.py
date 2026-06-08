import sys
from pathlib import Path
import json
import pytest
from sky_music.config import AppConfig, clear_config_cache, HotkeyDefaults, resolve_game_fps

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

import main

@pytest.fixture(autouse=True)
def _reset_config_cache():
    clear_config_cache()
    yield
    clear_config_cache()

def test_timing_profile_parsing():
    parser = main.build_arg_parser()
    
    # 1. Test balanced profile (default)
    args = parser.parse_args(["--timing-profile", "balanced"])
    main.configure_from_args(args, AppConfig())
    assert main.TIMING_POLICY.hold_us == 17_000
    assert main.TIMING_POLICY.min_hold_us == 17_000
    assert not hasattr(main.TIMING_POLICY, "release_gap_us")


def test_local_precise_profile_from_builtin_defaults():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--timing-profile", "local-precise"])
    main.configure_from_args(args, AppConfig())
    assert main.TIMING_POLICY.fps == 60
    assert main.TIMING_POLICY.hold_us == 16_667
    assert main.SLEEP_POLICY.spin_threshold_us == 800


def test_runtime_ablation_flags_configure_debug_switches():
    parser = main.build_arg_parser()
    args = parser.parse_args([
        "--no-dispatch-thread",
        "--no-timer-guard",
        "--no-waitable-timer",
        "--no-gc-pause",
    ])

    main.configure_from_args(args, AppConfig())

    assert main.USE_DISPATCH_THREAD is False
    assert main.ENABLE_TIMER_GUARD is False
    assert main.ENABLE_WAITABLE_TIMER is False
    assert main.ENABLE_GC_PAUSE is False


def test_removed_dense_safe_profile_is_rejected_by_cli():
    parser = main.build_arg_parser()
    with pytest.raises(SystemExit):
        parser.parse_args(["--timing-profile", "dense-safe"])


def test_saved_profile_restored_with_fps_config():
    """Profile must survive restart when game_fps is set (no @fps suffix in storage)."""
    from sky_music.config import canonical_profile_name

    cfg = AppConfig(default_timing_profile="audience-safe", default_tempo_scale=0.95, game_fps=60)
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    main.apply_config_defaults(args, cfg)
    assert args.timing_profile == "audience-safe"
    assert args.fps == 60

    main.configure_from_args(args, cfg)
    assert main.TIMING_PROFILE_NAME == "audience-safe@60fps"
    assert canonical_profile_name(main.TIMING_PROFILE_NAME) == "audience-safe"

    # Simulate picker init on next launch
    assert canonical_profile_name(cfg.default_timing_profile) == "audience-safe"


def test_picker_metadata_loads_song_stats():
    from pathlib import Path
    from sky_music.ui.picker_metadata import get_song_ui_metadata, clear_metadata_cache

    clear_metadata_cache()
    meta = get_song_ui_metadata(Path("songs/1test copy.json"))
    assert meta.note_count > 0
    assert meta.duration_seconds > 0


def test_picker_metadata_marks_invalid_song_as_error(tmp_path):
    from sky_music.ui.picker_metadata import get_song_ui_metadata, clear_metadata_cache

    song_file = tmp_path / "bad.json"
    song_file.write_text(
        json.dumps({
            "name": "Bad",
            "songNotes": [{"time": 0, "key": "InvalidKey"}],
        }),
        encoding="utf-8",
    )

    clear_metadata_cache()
    meta = get_song_ui_metadata(song_file)
    assert meta.risk == "error"
    assert meta.recommended_profile == "unplayable"
    assert meta.note_count == 0


def test_display_profile_name_no_double_fps_suffix():
    from sky_music.config import display_profile_name

    assert display_profile_name("audience-safe@120fps", 120) == "audience-safe@120fps"


def test_canonical_profile_name_strips_fps_suffix():
    from sky_music.config import AppConfig, canonical_profile_name, merged_timing_profiles

    assert canonical_profile_name("remote-safe@60fps") == "audience-safe"
    assert canonical_profile_name("local_precise") == "local-precise"
    assert canonical_profile_name("dense-safe@120fps") == "balanced"
    assert set(merged_timing_profiles(AppConfig(timing_profiles={"dense_safe": {}}))) == {
        "local_precise",
        "balanced",
        "audience_safe",
    }
    # Removed profiles canonicalise to the balanced default.
    assert canonical_profile_name("high_fps_precise") == "balanced"


def test_load_config_sanitizes_corrupted_profile_with_fps_suffix(tmp_path, monkeypatch):
    from sky_music.config import load_config, clear_config_cache

    config_file = tmp_path / "config.json"
    config_file.write_text(
        json.dumps({
            "default_timing_profile": "audience-safe@60fps",
            "default_tempo_scale": 0.95,
            "game_fps": 60,
        }),
        encoding="utf-8",
    )
    monkeypatch.setattr("sky_music.config.CONFIG_PATH", config_file)
    clear_config_cache()
    cfg = load_config(force_reload=True)
    assert cfg.default_timing_profile == "audience-safe"
    assert cfg.game_fps == 60


def test_timing_overrides_parsing():
    parser = main.build_arg_parser()
    args = parser.parse_args([
        "--timing-profile", "balanced",
        "--hold-ms", "30",
        "--min-hold-ms", "15",
    ])
    main.configure_from_args(args, AppConfig())
    assert main.TIMING_POLICY.hold_us == 30_000
    assert main.TIMING_POLICY.min_hold_us == 15_000


def test_fractional_timing_overrides_parse_as_milliseconds():
    parser = main.build_arg_parser()
    args = parser.parse_args([
        "--timing-profile", "local-precise",
        "--hold-ms", "10.5",
        "--min-hold-ms", "7.25",
        "--focus-restore-grace-ms", "50.5",
    ])

    main.configure_from_args(args, AppConfig())

    assert main.TIMING_POLICY.hold_us == 10_500
    assert main.TIMING_POLICY.min_hold_us == 7_250
    assert main.TIMING_POLICY.focus_restore_grace_us == 50_500


def test_repeat_release_gap_override_is_removed_from_cli():
    parser = main.build_arg_parser()
    with pytest.raises(SystemExit):
        parser.parse_args(["--repeat-release-gap-ms", "4"])


def test_release_gap_override_is_removed_from_cli():
    parser = main.build_arg_parser()
    with pytest.raises(SystemExit):
        parser.parse_args(["--release-gap-ms", "2.5"])


def test_dry_run_simulation_flag():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--dry-run"])
    main.configure_from_args(args, AppConfig())
    assert main.DRY_RUN_MODE is True

def test_tempo_scale_parsing():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--tempo-scale", "0.85"])
    main.configure_from_args(args, AppConfig())
    assert main.TEMPO_SCALE == 0.85

def test_tempo_scale_invalid_fails():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--tempo-scale", "-1.0"])
    with pytest.raises(ValueError, match="tempo_scale must be > 0"):
        main.configure_from_args(args, AppConfig())

def test_hotkey_overlap_detection():
    # Test that overlapping hotkeys raise ValueError (Default behavior)
    parser = main.build_arg_parser()
    # 'y' is a note key (Key0)
    args = parser.parse_args(["--pause-key", "y"])
    with pytest.raises(ValueError, match="Hotkey overlaps with note keys"):
        main.build_playback_controls(args)

def test_hotkey_overlap_allowed_with_flag():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--pause-key", "y", "--allow-note-hotkeys"])
    controls = main.build_playback_controls(args)
    assert controls.pause.name == "y"

def test_telemetry_csv_default_from_config():
    # Mock config with telemetry enabled
    cfg = AppConfig(telemetry_enabled_by_default=True)
    parser = main.build_arg_parser()
    args = parser.parse_args([]) # No CLI flags
    main.apply_config_defaults(args, cfg)
    main.configure_from_args(args, cfg)
    assert main.TELEMETRY_CSV_ENABLED is True

def test_verbose_hud_default_from_config():
    cfg = AppConfig(verbose_hud=True)
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    main.apply_config_defaults(args, cfg)
    main.configure_from_args(args, cfg)
    assert main.VERBOSE_HUD is True

def test_game_fps_default_from_config():
    cfg = AppConfig(game_fps=120)
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    main.apply_config_defaults(args, cfg)
    main.configure_from_args(args, cfg)
    # Check that TIMING_POLICY is now a FrameTimingPolicy with 120 FPS
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    assert isinstance(main.TIMING_POLICY, FrameTimingPolicy)
    assert main.TIMING_POLICY.fps == 120

def test_config_fps_override_by_cli():
    cfg = AppConfig(game_fps=60)
    parser = main.build_arg_parser()
    args = parser.parse_args(["--fps", "30"])
    main.apply_config_defaults(args, cfg)
    main.configure_from_args(args, cfg)
    assert main.TIMING_POLICY.fps == 30

def test_no_fps_config_or_cli_defaults_to_60fps():
    cfg = AppConfig(game_fps=0)
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    main.apply_config_defaults(args, cfg)
    main.configure_from_args(args, cfg)
    assert main.TIMING_POLICY.fps == 60


def test_resolve_game_fps_never_zero():
    assert resolve_game_fps(0) == 60
    assert resolve_game_fps(None) == 60
    assert resolve_game_fps(-5) == 60
    assert resolve_game_fps(144) == 144


def test_config_load_migrates_zero_fps(tmp_path, monkeypatch):
    import sky_music.config as config_module

    config_file = tmp_path / "config.json"
    config_file.write_text(json.dumps({"game_fps": 0}), encoding="utf-8")
    monkeypatch.setattr(config_module, "CONFIG_PATH", config_file)
    clear_config_cache()
    assert config_module.load_config(force_reload=True).game_fps == 60

def test_hotkeys_default_from_config():
    cfg = AppConfig(hotkeys=HotkeyDefaults(pause="f7", skip="f11"))
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    main.apply_config_defaults(args, cfg)
    assert args.pause_key == "f7"
    assert args.skip_key == "f11"


def test_sky_process_names_default_from_config():
    cfg = AppConfig(sky_process_names=["CustomSky.exe"])
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    main.apply_config_defaults(args, cfg)
    assert args.sky_process_names == "CustomSky.exe"


def test_calibrate_advisory_moderate_jitter():
    from sky_music.orchestration.calibration import CalibrationInput, calibrate_profile
    inp = CalibrationInput(
        profile_name="balanced", tempo_scale=1.0, fps=60,
        p95_lateness_us=2000, p99_lateness_us=9000,
        p95_send_duration_us=1000, late_over_10ms=0,
        impossible_same_key_repeats=0, risky_same_key_repeats=0,
        failed_release_count=0
    )
    rec = calibrate_profile(inp)
    assert rec.severity == "moderate"
    assert rec.profile_name == "balanced"
    assert rec.tempo_scale == 0.95

def test_calibrate_advisory_severe_jitter():
    from sky_music.orchestration.calibration import CalibrationInput, calibrate_profile
    inp = CalibrationInput(
        profile_name="balanced", tempo_scale=1.0, fps=60,
        p95_lateness_us=5000, p99_lateness_us=16000,
        p95_send_duration_us=2000, late_over_10ms=10,
        impossible_same_key_repeats=0, risky_same_key_repeats=0,
        failed_release_count=0
    )
    rec = calibrate_profile(inp)
    assert rec.severity == "severe"
    assert rec.profile_name == "local-precise"
    assert rec.tempo_scale == 0.90


def test_calibrate_advisory_dense_schedule_without_lateness():
    from sky_music.orchestration.calibration import CalibrationInput, calibrate_profile

    inp = CalibrationInput(
        profile_name="balanced",
        tempo_scale=1.0,
        fps=60,
        p95_lateness_us=1000,
        p99_lateness_us=2500,
        p95_send_duration_us=500,
        late_over_10ms=0,
        impossible_same_key_repeats=0,
        risky_same_key_repeats=8,
        failed_release_count=0,
        compressed_holds=12,
        max_polyphony=6,
        note_count=100,
    )
    rec = calibrate_profile(inp)
    assert rec.severity == "moderate"
    assert rec.profile_name == "local-precise"
    assert rec.tempo_scale == 0.95


def test_calibration_summary_prefers_infeasible_same_key_alias():
    from sky_music.orchestration.calibration import calibration_input_from_summary

    inp = calibration_input_from_summary({
        "profile": "balanced",
        "fps": 60,
        "lateness_us": {},
        "send_duration_us": {},
        "backend": {},
        "schedule": {
            "impossible_same_key_repeats": 0,
            "infeasible_same_key_repeats": 2,
            "risky_same_key_repeats": 0,
            "note_count": 10,
        },
    })

    assert inp.impossible_same_key_repeats == 0
    assert inp.infeasible_same_key_repeats == 2


def test_calibrate_hold_uses_frame_timing_policy():
    from sky_music.orchestration.calibration import CalibrationInput, calibrate_profile

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
    assert rec.profile_name == "local-precise"
    assert rec.hold_us == 33_334


def test_frame_timing_defaults_from_config(tmp_path, monkeypatch):
    from sky_music.config import FrameTimingDefaults, load_config

    cfg_path = tmp_path / "config.json"
    cfg_path.write_text(
        '{"frame_timing": {"min_hold_min_frame_ratio": 0.75, "repeat_release_gap_min_frame_ratio": 0.2}}',
        encoding="utf-8",
    )
    monkeypatch.setattr("sky_music.config.CONFIG_PATH", cfg_path)
    clear_config_cache()
    cfg = load_config()
    assert cfg.frame_timing.min_hold_min_frame_ratio == 0.75
    assert not hasattr(cfg.frame_timing, "repeat_release_gap_min_frame_ratio")
    assert cfg.frame_timing.min_visible_hold_frames == FrameTimingDefaults.min_visible_hold_frames


def _write_summary(tmp_path: Path, data: dict, name: str = "playback_telemetry_test.summary.json") -> Path:
    logs_dir = tmp_path / "logs"
    logs_dir.mkdir(exist_ok=True)
    summary_path = logs_dir / name
    summary_path.write_text(
        json.dumps(data),
        encoding="utf-8",
    )
    return summary_path


def test_apply_calibration_loads_latest_summary(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    _write_summary(tmp_path, {
        "profile": "balanced",
        "tempo_scale": 1.0,
        "fps": 60,
        "lateness_us": {"p95_us": 1000, "p99_us": 2000, "over_10ms": 0},
        "send_duration_us": {"p95_us": 500},
        "backend": {"panic_release_failures": 0},
        "schedule": {"impossible_same_key_repeats": 0, "risky_same_key_repeats": 0},
    })

    from sky_music.cli.calibration_command import apply_calibration_from_telemetry
    from sky_music.orchestration.runtime_session import RuntimeSessionState

    state = RuntimeSessionState()
    res = apply_calibration_from_telemetry(AppConfig(), state)
    assert res.applied is True
    assert state.session is not None
    assert state.timing_profile_name == "local-precise@60fps"


def test_save_calibration_persists_profile_tempo_and_fps(tmp_path, monkeypatch):
    from sky_music.config import load_config

    config_file = tmp_path / "config.json"
    monkeypatch.setattr("sky_music.config.CONFIG_PATH", config_file)
    monkeypatch.chdir(tmp_path)
    clear_config_cache()
    _write_summary(tmp_path, {
        "profile": "balanced",
        "tempo_scale": 1.0,
        "fps": 60,
        "lateness_us": {"p95_us": 2000, "p99_us": 9000, "over_10ms": 0},
        "send_duration_us": {"p95_us": 1000},
        "backend": {"panic_release_failures": 0},
        "schedule": {"impossible_same_key_repeats": 0, "risky_same_key_repeats": 0},
    })

    from sky_music.cli.calibration_command import apply_calibration_from_telemetry
    from sky_music.orchestration.runtime_session import RuntimeSessionState

    state = RuntimeSessionState()
    res = apply_calibration_from_telemetry(AppConfig(), state, persist=True)
    assert res.applied is True

    saved = load_config(force_reload=True)
    assert saved.default_timing_profile == "balanced"
    assert saved.default_tempo_scale == 0.95
    assert saved.game_fps == 60
    assert "balanced" in saved.timing_profiles
    assert "@" not in saved.default_timing_profile


def test_apply_calibration_uses_explicit_summary_path(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    _write_summary(tmp_path, {
        "profile": "balanced",
        "tempo_scale": 1.0,
        "fps": 60,
        "lateness_us": {"p95_us": 1000, "p99_us": 2000, "over_10ms": 0},
        "send_duration_us": {"p95_us": 500},
        "backend": {"panic_release_failures": 0},
        "schedule": {"impossible_same_key_repeats": 0, "risky_same_key_repeats": 0},
    })
    explicit = tmp_path / "chosen.summary.json"
    explicit.write_text(
        json.dumps({
            "profile": "balanced",
            "tempo_scale": 1.0,
            "fps": 30,
            "lateness_us": {"p95_us": 5000, "p99_us": 16000, "over_10ms": 10},
            "send_duration_us": {"p95_us": 2000},
            "backend": {"panic_release_failures": 0},
            "schedule": {"impossible_same_key_repeats": 0, "risky_same_key_repeats": 0},
        }),
        encoding="utf-8",
    )

    from sky_music.cli.calibration_command import apply_calibration_from_telemetry
    from sky_music.orchestration.runtime_session import RuntimeSessionState

    state = RuntimeSessionState()
    res = apply_calibration_from_telemetry(AppConfig(), state, summary_path=explicit)
    assert res.applied is True
    assert state.session is not None
    assert state.session.fps == 30
    assert state.session.profile_name == "audience-safe"

