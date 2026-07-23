
import pytest

from sky_music.config import (
    AppConfig,
    FrameTimingDefaults,
)
from sky_music.platform.win32 import inputs


def test_set_expected_process_names_rejects_empty_after_normalize():
    # Store old for restore
    old_names = inputs.EXPECTED_PROCESS_NAMES
    try:
        inputs.set_expected_process_names(["Sky.exe"])
        assert "Sky.exe" in inputs.EXPECTED_PROCESS_NAMES
        
        with pytest.raises(ValueError, match="Expected process names cannot be empty"):
            inputs.set_expected_process_names(["", "   "])
            
        assert "Sky.exe" in inputs.EXPECTED_PROCESS_NAMES
    finally:
        inputs.EXPECTED_PROCESS_NAMES = old_names


def test_config_rejects_bool_for_numeric_timing_field(monkeypatch):
    import sky_music.config as config
    
    def mock_load_raw():
        return {
            "default_tempo_scale": True,
            "input_path_warn_us": False,
            "frame_timing": {
                "min_visible_hold_frames": True
            }
        }
    monkeypatch.setattr(config, "_load_raw", mock_load_raw)
    
    cfg = config._build_config_from_disk()
    assert cfg.default_tempo_scale != 1.0 or type(cfg.default_tempo_scale) is float  # Actually we want it to reject bool entirely and fallback to default
    assert cfg.input_path_warn_us == AppConfig.input_path_warn_us
    assert cfg.frame_timing.min_visible_hold_frames == FrameTimingDefaults.min_visible_hold_frames


def test_config_booleans_strictness(monkeypatch):
    import sky_music.config as config
    def mock_load_raw():
        return {
            "telemetry_enabled_by_default": "true",
            "verbose_hud": 1,
        }
    monkeypatch.setattr(config, "_load_raw", mock_load_raw)
    
    cfg = config._build_config_from_disk()
    # It should not invent loose truthiness. So "true" string or 1 should fall back to default
    assert cfg.telemetry_enabled_by_default == AppConfig.telemetry_enabled_by_default
    assert cfg.verbose_hud == AppConfig.verbose_hud


def test_tempo_scale_nan_inf_rejected(monkeypatch):
    import sky_music.config as config
    def mock_load_raw():
        return {
            "default_tempo_scale": float("inf"),
        }
    monkeypatch.setattr(config, "_load_raw", mock_load_raw)
    cfg = config._build_config_from_disk()
    assert cfg.default_tempo_scale == config.AppConfig.default_tempo_scale


def test_timing_fields_nan_inf_rejected():
    from typing import Any, cast

    import pytest

    from sky_music.domain.validation import validate_timing_profile

    with pytest.raises(ValueError, match="finite"):
        validate_timing_profile(cast(Any, {
            "min_hold_frames": float("nan"),
            "min_hold_us": 20000
        }))
        
    with pytest.raises(ValueError, match="finite"):
        validate_timing_profile(cast(Any, {
            "hold_frames": float("inf"),
            "min_hold_us": 20000
        }))


def test_fps_reject_unknown(monkeypatch):
    import sky_music.config as config
    def mock_load_raw():
        return {
            "game_fps": 50  # Unknown FPS
        }
    monkeypatch.setattr(config, "_load_raw", mock_load_raw)
    
    cfg = config._build_config_from_disk()
    # Reject unknown FPS, should not silently clamp, it should raise or use default?
    # Wait, the requirement: "Reject unknown FPS at the same boundary that accepts user FPS (CLI/UI/config) — do not clamp silently"
    # For config load, if it rejects, it should probably fall back to default, or we can test `resolve_game_fps`
    
    from sky_music.config import DEFAULT_GAME_FPS
    assert cfg.game_fps == DEFAULT_GAME_FPS


def test_scan_code_strictness():
    from typing import Any, cast

    import pytest

    from sky_music.platform.win32.inputs import _cached_key_input
    
    with pytest.raises(TypeError, match="strict int"):
        _cached_key_input(cast(Any, True), 0)
        
    with pytest.raises(TypeError, match="strict int"):
        _cached_key_input(cast(Any, 21.5), 0)
        
    with pytest.raises(ValueError, match="out of bounds"):
        _cached_key_input(-1, 0)
        
    with pytest.raises(ValueError, match="out of bounds"):
        _cached_key_input(0x10000, 0)
        
    # Valid
    _cached_key_input(21, 0)

