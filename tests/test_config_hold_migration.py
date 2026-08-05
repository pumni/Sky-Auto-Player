import json

import pytest

import sky_music.config as config


@pytest.fixture
def config_file(tmp_path, monkeypatch):
    path = tmp_path / "config.json"
    monkeypatch.setattr(config, "CONFIG_PATH", path)
    config.clear_config_cache()
    yield path
    config.clear_config_cache()


def write(path, payload):
    path.write_text(json.dumps(payload), encoding="utf-8")


@pytest.mark.parametrize(
    ("name", "expected"),
    [("balanced", 1.0), ("local-precise", 1.0), ("audience-safe", 1.5), ("remote_safe", 1.5), ("online_audible", 1.5), ("unknown", 1.0)],
)
def test_v2_profile_migration(config_file, name, expected):
    write(config_file, {"schema_version": 2, "default_timing_profile": name, "game_fps": 60, "unknown_key": "keep"})
    cfg = config.load_config(force_reload=True)
    assert cfg.default_hold_frames == expected
    payload = json.loads(config_file.read_text(encoding="utf-8"))
    assert payload["schema_version"] == 3
    assert payload["default_hold_frames"] == expected
    assert "default_timing_profile" not in payload
    assert payload["unknown_key"] == "keep"


def test_override_precedence_and_unframed_is_ignored(config_file):
    write(config_file, {
        "schema_version": 2,
        "default_timing_profile": "balanced",
        "game_fps": 60,
        "timing_profiles": {"balanced": {"min_hold_frames": 1.375, "min_hold_unframed_us": 1}},
    })
    cfg = config.load_config(force_reload=True)
    assert cfg.default_hold_frames == 1.5


def test_absolute_override_uses_persisted_fps(config_file):
    write(config_file, {
        "schema_version": 2,
        "default_timing_profile": "balanced",
        "game_fps": 30,
        "timing_profiles": {"balanced": {"min_hold_us": 41_667}},
    })
    cfg = config.load_config(force_reload=True)
    assert cfg.default_hold_frames == 1.25


def test_v3_invalid_hold_falls_back_and_second_load_is_idempotent(config_file):
    write(config_file, {"schema_version": 3, "default_hold_frames": 2.0, "game_fps": 60})
    cfg = config.load_config(force_reload=True)
    first = config_file.read_bytes()
    assert cfg.default_hold_frames == 1.0
    config.load_config(force_reload=True)
    assert config_file.read_bytes() == first
