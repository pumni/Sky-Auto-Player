from __future__ import annotations

from unittest.mock import patch

from sky_music.infrastructure.backend import DryRunBackend
from sky_music.orchestration.core import loop
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.telemetry import TelemetryLogger
from sky_music.platform.win32 import inputs


def test_core_warmup_spin_us_exists():
    assert hasattr(loop, "CORE_WARMUP_SPIN_US")
    assert isinstance(loop.CORE_WARMUP_SPIN_US, int)
    assert loop.CORE_WARMUP_SPIN_US >= 200


def test_send_cold_threshold_us_exists():
    assert hasattr(loop, "SEND_COLD_THRESHOLD_US")
    assert isinstance(loop.SEND_COLD_THRESHOLD_US, int)
    assert loop.SEND_COLD_THRESHOLD_US > 0


def test_spin_floor_us_default_at_least_700():
    from sky_music.domain.domain import Song
    engine = PlaybackEngine(
        song=Song(name="test", notes=()),
        actions=(),
        backend=DryRunBackend(),
        telemetry_enabled=True,
    )
    assert engine.spin_floor_us >= 700


def test_musical_path_no_sleep_retry():
    with patch("sky_music.platform.win32.inputs.user32.SendInput") as mock_send_input, \
         patch("sky_music.platform.win32.inputs._retry_wait_seconds") as mock_retry_wait:
        mock_send_input.side_effect = [2, 1]
        inputs._DIAG.keys_retried = 0
        inputs._DIAG.keys_dropped = 0
        landed = inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=False)

        assert landed == 3
        assert inputs._DIAG.keys_retried == 1
        assert inputs._DIAG.keys_dropped == 0
        mock_retry_wait.assert_not_called()


def test_option_modal_supports_info_text():
    from sky_music.ui.textual_app.modals import OptionModal
    modal = OptionModal("Test Title", [], info_text="hello")
    assert modal.info_text == "hello"


def test_telemetry_game_observed_default_false():
    logger = TelemetryLogger(
        song_name="test", fps=60, min_hold_us=1000, enabled=True,
    )
    logger.record(
        event_index=0, kind="down", scheduled_us=1000, actual_us=1010,
        lateness_us=10, send_duration_us=5, scan_codes=(0x15,), reason="note",
        sent_scan_codes=(0x15,),
    )
    summary = logger.get_summary()
    assert summary is not None
    assert summary["evidence_boundaries"]["game_observed"]["available"] is False


def test_mid_song_reprobe_exists_on_loop():
    assert loop.DispatchLoop._run_mid_song_reprobe is not None


def test_telemetry_summary_has_partial_note_on_count():
    logger = TelemetryLogger(song_name="test", fps=60, min_hold_us=1000, enabled=True)
    logger.record(
        event_index=0, kind="down", scheduled_us=1000, actual_us=1005,
        lateness_us=5, send_duration_us=3, scan_codes=(0x15,), reason="note",
        sent_scan_codes=(0x15,), runtime_outcome="partial_note_on",
    )
    summary = logger.get_summary()
    assert summary is not None
    assert summary["partial_note_on_count"] >= 1


def test_telemetry_summary_includes_event_wait_flag():
    logger = TelemetryLogger(song_name="test", fps=60, min_hold_us=1000, enabled=True)
    logger.record(
        event_index=0, kind="down", scheduled_us=1000, actual_us=1010,
        lateness_us=10, send_duration_us=5, scan_codes=(0x15,), reason="note",
        sent_scan_codes=(0x15,),
    )
    logger.record_runtime_options({"event_wait_degraded_to_polled": True})
    summary = logger.get_summary()
    assert summary is not None
    assert summary["runtime_options"]["event_wait_degraded_to_polled"] is True