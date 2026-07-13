"""Tests for the update-related persistence additions in ``sky_music.config``."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

import sky_music.config as config_mod
from sky_music.config import (
    AppConfig,
    UpdateSettings,
    clear_config_cache,
)


@pytest.fixture(autouse=True)
def _reset_config_cache():
    clear_config_cache()
    yield
    clear_config_cache()


@pytest.fixture
def isolated_config(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Redirect ``CONFIG_PATH`` to an isolated tmp file for save_config tests."""
    cfg_path = tmp_path / "config.json"
    cfg_path.write_text("{}", encoding="utf-8")
    monkeypatch.setattr(config_mod, "CONFIG_PATH", cfg_path)
    return cfg_path


# ── UpdateSettings.from_dict ──────────────────────────────────────────────────


def test_update_settings_defaults() -> None:
    s = UpdateSettings()
    assert s.auto_check is True
    assert s.auto_apply is False
    assert s.skip_version == ""
    assert s.check_interval_s == 86400
    assert s.last_check_ts == 0


def test_update_settings_from_dict_full() -> None:
    raw: dict[str, Any] = {
        "auto_check": False,
        "auto_apply": True,
        "skip_version": "2.4.0",
        "check_interval_s": 3600,
        "last_check_ts": 1718200000,
    }
    s = UpdateSettings.from_dict(raw)
    assert s.auto_check is False
    assert s.auto_apply is True
    assert s.skip_version == "2.4.0"
    assert s.check_interval_s == 3600
    assert s.last_check_ts == 1718200000


def test_update_settings_from_dict_optional_keys_use_defaults() -> None:
    s = UpdateSettings.from_dict({"auto_check": False})
    assert s.auto_check is False
    assert s.auto_apply is False
    assert s.skip_version == ""
    assert s.check_interval_s == 86400
    assert s.last_check_ts == 0


def test_update_settings_from_dict_non_dict_returns_defaults() -> None:
    s = UpdateSettings.from_dict("not a dict")  # type: ignore[arg-type]
    assert s.auto_check is True  # default
    assert s.auto_apply is False
    assert s.check_interval_s == 86400


def test_update_settings_from_dict_invalid_integers_clamp() -> None:
    raw = {"check_interval_s": "garbage", "last_check_ts": "n/a"}
    s = UpdateSettings.from_dict(raw)
    assert s.check_interval_s == 86400  # falls back to default
    assert s.last_check_ts == 0


def test_update_settings_from_dict_negative_interval_clamps_to_zero() -> None:
    s = UpdateSettings.from_dict({"check_interval_s": -10})
    assert s.check_interval_s == 0


def test_update_settings_from_dict_skip_version_normalizes_none_to_empty() -> None:
    s = UpdateSettings.from_dict({"skip_version": None})
    assert s.skip_version == ""


# ── AppConfig.update field ─────────────────────────────────────────────────────


def test_appconfig_default_includes_update_settings() -> None:
    cfg = AppConfig()
    assert isinstance(cfg.update, UpdateSettings)
    assert cfg.update.auto_check is True


def test_appconfig_factory_produces_independent_instances() -> None:
    a = AppConfig()
    b = AppConfig()
    a.update.skip_version = "x"
    assert b.update.skip_version == ""


# ── load_config / save_config round-trip ──────────────────────────────────────


def test_save_config_persists_update_block(isolated_config: Path) -> None:
    from sky_music.config import save_config

    cfg = AppConfig()
    cfg.update.auto_check = False
    cfg.update.skip_version = "2.5.0"
    cfg.update.check_interval_s = 7200
    cfg.update.last_check_ts = 1718200000

    save_config(cfg)
    text = isolated_config.read_text(encoding="utf-8")
    raw = json.loads(text)
    assert "update" in raw
    assert raw["update"]["auto_check"] is False
    assert raw["update"]["skip_version"] == "2.5.0"
    assert raw["update"]["check_interval_s"] == 7200
    assert raw["update"]["last_check_ts"] == 1718200000


def test_load_config_round_trips_update_block(isolated_config: Path) -> None:
    from sky_music.config import load_config, save_config

    cfg = AppConfig()
    cfg.update.auto_check = False
    cfg.update.skip_version = "2.5.0"
    save_config(cfg)

    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.auto_check is False
    assert reloaded.update.skip_version == "2.5.0"


def test_load_config_handles_missing_update_block(isolated_config: Path) -> None:
    isolated_config.write_text(
        json.dumps({"theme": "aurora", "schema_version": 2}),
        encoding="utf-8",
    )
    from sky_music.config import load_config

    cfg = load_config(force_reload=True)
    assert cfg.update == UpdateSettings()  # defaults


def test_load_config_handles_malformed_update_block(isolated_config: Path) -> None:
    isolated_config.write_text(
        json.dumps({"update": "not a dict"}),
        encoding="utf-8",
    )
    from sky_music.config import load_config

    cfg = load_config(force_reload=True)
    # Malformed update block falls back to defaults — never raises.
    assert cfg.update == UpdateSettings()


# ── persistence helpers ───────────────────────────────────────────────────────


def test_persist_update_skip_version_writes_string(isolated_config: Path) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_skip_version,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_skip_version(cfg, "2.4.0")

    # Reload and verify
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.skip_version == "2.4.0"


def test_persist_update_skip_version_clears_with_empty(
    isolated_config: Path,
) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_skip_version,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_skip_version(cfg, "2.4.0")
    persist_update_skip_version(cfg, "")
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.skip_version == ""


def test_persist_update_check_ts(isolated_config: Path) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_check_ts,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_check_ts(cfg, 12345)
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.last_check_ts == 12345


def test_persist_update_auto_check(isolated_config: Path) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_auto_check,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_auto_check(cfg, False)
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.auto_check is False
