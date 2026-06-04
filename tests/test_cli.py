import sys
from pathlib import Path
import pytest
from sky_music.config import AppConfig, clear_config_cache

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

import main

@pytest.fixture(autouse=True)
def _reset_config_cache():
    clear_config_cache()
    yield
    clear_config_cache()

def test_cli_song_argument_parsing():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--song", "Diamonds"])
    assert args.song == "Diamonds"

def test_cli_list_argument():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--list"])
    assert args.list is True

def test_txt_song_extension_is_supported():
    from sky_music.ui.picker_helpers import SUPPORTED_EXTENSIONS

    assert ".txt" in SUPPORTED_EXTENSIONS

def test_cli_fps_argument_applies_timing_policy():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--fps", "60"])
    main.configure_from_args(args, AppConfig())
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    assert isinstance(main.TIMING_POLICY, FrameTimingPolicy)
    assert main.TIMING_POLICY.fps == 60

def test_cli_theme_argument():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--theme", "cyberpunk"])
    assert args.theme == "cyberpunk"

def test_cli_ui_argument_defaults_to_auto():
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    assert args.ui == "auto"
    args = parser.parse_args(["--ui", "textual"])
    assert args.ui == "textual"


class DummyStdout:
    def __init__(self, is_tty: bool) -> None:
        self.is_tty = is_tty

    def isatty(self) -> bool:
        return self.is_tty


def test_supports_textual_requires_tty(monkeypatch):
    monkeypatch.setattr(main.sys, "stdout", DummyStdout(False))
    assert main._supports_textual() is False


def test_supports_textual_on_windows_terminal(monkeypatch):
    monkeypatch.setattr(main.sys, "stdout", DummyStdout(True))
    monkeypatch.setattr(main.sys, "platform", "win32")
    monkeypatch.setenv("WT_SESSION", "test-session")
    monkeypatch.delenv("TERM_PROGRAM", raising=False)
    assert main._supports_textual() is True


def test_supports_textual_falls_back_on_weak_windows_terminal(monkeypatch):
    monkeypatch.setattr(main.sys, "stdout", DummyStdout(True))
    monkeypatch.setattr(main.sys, "platform", "win32")
    monkeypatch.delenv("WT_SESSION", raising=False)
    monkeypatch.delenv("TERM_PROGRAM", raising=False)
    assert main._supports_textual() is False


def test_textual_selftest_argument_is_hidden_and_parses():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--selftest-textual"])
    assert args.selftest_textual is True
    assert "--selftest-textual" not in parser.format_help()


def test_textual_selftest_runs_headlessly():
    assert main._run_textual_selftest() == 0

def test_prompt_song_selection_routes_to_textual(monkeypatch):
    from sky_music.ui import textual_app
    from sky_music.ui.picker import SongPickerResult

    expected = SongPickerResult(
        song_path=Path("songs/Alpha.json"),
        action="dry_run",
        profile_name="balanced",
        tempo_scale=1.0,
        fps=None,
    )

    def fake_textual_picker(**kwargs: object) -> SongPickerResult:
        assert kwargs["initial_profile"] == "balanced"
        assert kwargs["initial_dry_run"] is True
        return expected

    monkeypatch.setattr(main, "PICKER_UI_MODE", "textual")
    monkeypatch.setattr(textual_app, "choose_song_interactively_textual", fake_textual_picker)

    assert main.prompt_song_selection(dry_run=True) == expected

def test_cli_repeat_argument():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--repeat", "5"])
    assert args.repeat == 5

def test_cli_countdown_argument():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--countdown", "10"])
    assert args.countdown == 10

def test_cli_doctor_flags():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--doctor"])
    assert args.doctor is True
    args = parser.parse_args(["--doctor-timing"])
    assert args.doctor_timing is True
    args = parser.parse_args(["--doctor-input"])
    assert args.doctor_input is True

def test_cli_save_calibration_argument():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--save-calibration"])
    assert args.save_calibration is True

def test_cli_calibration_summary_argument():
    parser = main.build_arg_parser()
    args = parser.parse_args(["--calibration-summary", "logs/run.summary.json"])
    assert args.calibration_summary == Path("logs/run.summary.json")


def test_profile_comparison_derives_hold_from_min_hold(capsys):
    main._print_profile_comparison_table(AppConfig())

    output = capsys.readouterr().out
    balanced_row = next(line for line in output.splitlines() if "balanced" in line)
    assert balanced_row.count("17") == 2


def test_dynamic_fps_resolution(monkeypatch):
    # Case 1: --fps is NOT in sys.argv (standard launch)
    monkeypatch.setattr(sys, "argv", ["main.py"])
    user_cfg = AppConfig(game_fps=144)
    parser = main.build_arg_parser()
    args = parser.parse_args([])
    main.apply_config_defaults(args, user_cfg)
    
    cli_fps_explicit = any(arg.startswith("--fps") for arg in sys.argv)
    resolved_fps = args.fps if cli_fps_explicit else (user_cfg.game_fps if user_cfg.game_fps > 0 else None)
    assert resolved_fps == 144
    
    # Simulate user changing FPS to 120 (picker persists this to user_cfg in memory):
    user_cfg.game_fps = 120
    resolved_fps = args.fps if cli_fps_explicit else (user_cfg.game_fps if user_cfg.game_fps > 0 else None)
    assert resolved_fps == 120
    
    # Case 2: --fps is explicitly in sys.argv (CLI override)
    monkeypatch.setattr(sys, "argv", ["main.py", "--fps", "60"])
    parser = main.build_arg_parser()
    args = parser.parse_args(["--fps", "60"])
    main.apply_config_defaults(args, user_cfg)
    
    cli_fps_explicit = any(arg.startswith("--fps") for arg in sys.argv)
    resolved_fps = args.fps if cli_fps_explicit else (user_cfg.game_fps if user_cfg.game_fps > 0 else None)
    assert resolved_fps == 60
    
    # Even if user config changes, the explicit CLI override wins:
    user_cfg.game_fps = 120
    resolved_fps = args.fps if cli_fps_explicit else (user_cfg.game_fps if user_cfg.game_fps > 0 else None)
    assert resolved_fps == 60



@pytest.mark.parametrize(
    "args",
    [
        ["--chord-merge-window-ms", "5"],
        ["--frame-align", "down_only"],
    ],
)
def test_removed_phase2_timing_arguments_are_rejected(args):
    parser = main.build_arg_parser()
    with pytest.raises(SystemExit):
        parser.parse_args(args)



