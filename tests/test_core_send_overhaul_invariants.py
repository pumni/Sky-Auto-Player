from __future__ import annotations

import json
from unittest.mock import patch

from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import get_calibrated_margin_recommendation
from sky_music.infrastructure.timing import Clock
from sky_music.infrastructure.wait_strategy import HybridWaitStrategy
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.telemetry import TelemetryLogger
from sky_music.platform.win32 import inputs


def test_musical_path_retry_no_sleep():
    """Invariant I3: First SendInput fails to send all keys, retry once immediately sleeperless."""
    with patch("sky_music.platform.win32.inputs.user32.SendInput") as mock_send_input, \
         patch("sky_music.platform.win32.inputs._retry_wait_seconds") as mock_retry_wait:
        # 3 keys: first sends 2, second sends remainder (1)
        mock_send_input.side_effect = [2, 1]
        
        inputs._DIAG.keys_retried = 0
        inputs._DIAG.keys_dropped = 0
        
        landed = inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=False)
        
        assert landed == 3
        assert inputs._DIAG.keys_retried == 1
        assert inputs._DIAG.keys_dropped == 0
        mock_retry_wait.assert_not_called()


def test_musical_path_persistent_block():
    """Invariant I3: Persistent block drops remaining keys instead of waiting or sleeping."""
    with patch("sky_music.platform.win32.inputs.user32.SendInput") as mock_send_input, \
         patch("sky_music.platform.win32.inputs._retry_wait_seconds") as mock_retry_wait:
        # Both calls return 0
        mock_send_input.side_effect = [0, 0]
        
        inputs._DIAG.keys_retried = 0
        inputs._DIAG.keys_dropped = 0
        
        landed = inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=False)
        
        assert landed == 0
        assert inputs._DIAG.keys_retried == 0
        assert inputs._DIAG.keys_dropped == 3
        mock_retry_wait.assert_not_called()


def test_release_path_completes_remainder():
    """Invariant I4: Note-off/release_all completes remainder via retry helper."""
    with patch("sky_music.platform.win32.inputs.user32.SendInput") as mock_send_input, \
         patch("sky_music.platform.win32.inputs.send_input_batch") as mock_send_batch:
        # Release path: completes remainder using send_input_batch
        mock_send_input.return_value = 1
        
        landed = inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=True)
        
        assert landed == 3
        mock_send_batch.assert_called_once()


def test_hybrid_wait_strategy_spin_does_not_sleep():
    """Invariant I12: Pure spin phase does not call sleep/yield functions."""
    class FakeClock(Clock):
        def __init__(self):
            self.ticks = [100, 105, 110]
            self._ns_based = False
        def now_us(self) -> int:
            return self.ticks.pop(0) if self.ticks else 200

    strategy = HybridWaitStrategy()
    with patch("time.sleep") as mock_sleep:
        strategy.spin_until_us(108, FakeClock())
        mock_sleep.assert_not_called()


def test_calibrated_margin_recommendation_poison_cases(tmp_path, monkeypatch):
    """Invariant: get_calibrated_margin_recommendation handles corrupted or missing cache gracefully."""
    monkeypatch.chdir(tmp_path)
    
    # Missing file -> None
    assert get_calibrated_margin_recommendation() is None
    
    # Corrupted file -> None
    cache_dir = tmp_path / ".cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    cache_file = cache_dir / "input_latency.json"
    cache_file.write_text("{invalid_json", encoding="utf-8")
    assert get_calibrated_margin_recommendation() is None
    
    # Absurd values -> None
    cache_data_absurd = {
        "version": 1,
        "down_us": {"p50": 200000, "p90": 300000, "p99": 400000},
        "up_us": {"p50": 100},
    }
    cache_file.write_text(json.dumps(cache_data_absurd), encoding="utf-8")
    assert get_calibrated_margin_recommendation() is None
    
    # Wrong version -> None
    cache_data_v2 = {
        "version": 2,
        "down_us": {"p50": 200, "p90": 300, "p99": 400},
        "up_us": {"p50": 100},
    }
    cache_file.write_text(json.dumps(cache_data_v2), encoding="utf-8")
    assert get_calibrated_margin_recommendation() is None


def test_telemetry_summary_game_observed_default():
    """Invariant: Telemetry defaults to game_observed.available = False."""
    logger = TelemetryLogger(song_name="test_song", fps=60, min_hold_us=1000, enabled=True)
    logger.record(
        event_index=0,
        kind="down",
        scheduled_us=1000,
        actual_us=1010,
        lateness_us=10,
        send_duration_us=5,
        scan_codes=(0x15,),
        reason="note",
        sent_scan_codes=(0x15,),
    )
    summary = logger.get_summary()
    assert summary is not None
    assert summary["evidence_boundaries"]["game_observed"]["available"] is False
    assert summary["game_acceptance_unknown"] is True
    assert summary["timing_semantics"]["onset_definition"] == "sendinput_return"
    assert summary["timing_semantics"]["game_observed_available"] is False
    assert "visible_lateness_us" in summary




def test_min_hold_assumes_fps_present_in_options():
    """Invariant: engine records min_hold_assumes_fps in runtime_options."""
    from sky_music.infrastructure.backend import DryRunBackend
    song = Song(name="test_song", notes=())
    engine = PlaybackEngine(
        song=song,
        actions=(),
        backend=DryRunBackend(),
        telemetry_enabled=True,
        fps=144
    )
    opts = engine.telemetry.runtime_options
    assert opts["min_hold_assumes_fps"] == 144
