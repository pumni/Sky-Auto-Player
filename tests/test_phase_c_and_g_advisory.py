"""Phase C & G advisory tests.

C: sub_60fps_frame_notes advisory fires when notes shorter than one 60 fps frame exist.
G: UIPI advisory text matches spec; unfocused+require_focus never sends down.
"""
from __future__ import annotations

from unittest.mock import MagicMock, patch

# ---------------------------------------------------------------------------
# Phase C: FPS advisory in doctor.print_fps_advisory()
# ---------------------------------------------------------------------------

def test_doctor_fps_advisory_prints_when_fps_gt_60(capsys) -> None:
    """print_fps_advisory() must emit advisory text when configured fps > 60."""
    import sky_music.infrastructure.doctor as doctor

    mock_cfg = MagicMock()
    mock_cfg.game_fps = 120

    with patch("sky_music.infrastructure.doctor.load_config", return_value=mock_cfg):
        doctor.print_fps_advisory()

    captured = capsys.readouterr()
    assert "shorter than one 60 fps frame" in captured.out


def test_doctor_fps_advisory_silent_at_60(capsys) -> None:
    """print_fps_advisory() must stay silent at exactly 60 fps."""
    import sky_music.infrastructure.doctor as doctor

    mock_cfg = MagicMock()
    mock_cfg.game_fps = 60

    with patch("sky_music.infrastructure.doctor.load_config", return_value=mock_cfg):
        doctor.print_fps_advisory()

    captured = capsys.readouterr()
    assert "shorter than one 60 fps frame" not in captured.out


def test_schedule_metadata_sub60fps_field_and_warning() -> None:
    """ScheduleMetadata.sub_60fps_frame_notes is set and warning string is present."""
    from sky_music.domain.scheduler_types import Microseconds, ScheduleMetadata

    meta = ScheduleMetadata(
        actions=(),
        source_duration_us=Microseconds(100_000),
        playback_duration_us=Microseconds(100_000),
        sub_60fps_frame_notes=3,
        warnings=("3 short note(s) are shorter than one 60 fps frame",),
    )
    assert meta.sub_60fps_frame_notes == 3
    assert any("shorter than one 60 fps frame" in w for w in meta.warnings)


# ---------------------------------------------------------------------------
# Phase G.3: UIPI text in check_sky_window()
# ---------------------------------------------------------------------------

def test_check_sky_window_uipi_text_non_admin() -> None:
    """check_sky_window() non-admin path must contain spec-exact UIPI wording."""
    import sky_music.infrastructure.doctor as doctor

    fake_hwnd = 12345

    with (
        patch.object(doctor.inputs, "get_sky_window", return_value=fake_hwnd),
        patch.object(doctor.inputs.user32, "GetWindowThreadProcessId", return_value=0),
        patch.object(doctor.inputs, "get_process_name_by_pid", return_value="Sky.exe"),
        patch("sky_music.infrastructure.doctor.is_admin", return_value=False),
    ):
        result = doctor.check_sky_window()

    assert result["ok"] is True
    assert "SendInput may return 0 (UIPI)" in result["msg"]
    assert "Run both elevated or both not elevated" in result["msg"]


# ---------------------------------------------------------------------------
# Phase G.2: unfocused + require_focus => no backend down calls
# ---------------------------------------------------------------------------

def test_unfocused_require_focus_no_backend_down() -> None:
    """Engine with require_focus=True must not dispatch key_down when unfocused."""
    from sky_music.domain import Song
    from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
    from sky_music.infrastructure.backend import DryRunBackend
    from sky_music.orchestration.engine import PlaybackEngine

    action = KeyAction(
        kind="down",  # type: ignore[arg-type]
        scan_codes=(ScanCode(0x21),),
        at_us=Microseconds(0),
        reason="test",
    )
    backend = DryRunBackend()

    engine = PlaybackEngine(
        song=Song(name="focus-test", notes=()),
        actions=(action,),
        backend=backend,
        require_focus=True,
        telemetry_enabled=False,
        use_dispatch_thread=False,
    )

    class _UnfocusedGuard:
        def is_active(self) -> bool:
            return False
        def focus(self) -> None:
            pass

    class _QuitAfterOne:
        def __init__(self):
            self._count = 0
        def poll(self):
            self._count += 1
            if self._count >= 1:
                return "quit"
            return None

    engine.focus_guard = _UnfocusedGuard()  # type: ignore[assignment]
    engine.controls = _QuitAfterOne()  # type: ignore[assignment]

    engine.play()
    down_calls = [r for r in backend.history if r[0] == "down"]
    assert len(down_calls) == 0, (
        f"Expected 0 down calls while unfocused, got {len(down_calls)}: {down_calls}"
    )
