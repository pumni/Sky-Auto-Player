"""Capture deterministic Wave 2 migration evidence from the current Python oracle.

This is an explicit maintainer tool. CI consumes the committed JSON fixtures;
it never regenerates them implicitly. Run it only when deliberately updating
the migration evidence after reviewing a current-behavior change.
"""

from __future__ import annotations

import argparse
import json
import tempfile
from dataclasses import asdict
from pathlib import Path

import sky_music.config as config_module
from sky_music.config import AppConfig, UpdateSettings
from sky_music.domain.update_policy import get_policy
from sky_music.orchestration.catalog_service import CatalogService
from sky_music.orchestration.settings_service import (
    SettingsService,
    normalize_application_settings,
)
from sky_music.orchestration.update_service import retry_delay_for, should_auto_check


def settings_fixture() -> dict[str, object]:
    malformed = AppConfig(
        theme="unknown",
        ui_background_mode="unknown",
        default_hold_frames=99.0,
        default_tempo_scale=float("nan"),
        game_fps=61,
        songs_dir="songs-from-fixture",
        sky_process_names=[" Sky.exe ", "", 42],  # type: ignore[list-item]
    )
    service = SettingsService(AppConfig())
    valid = service.patch(
        {
            "theme": " SLATE ",
            "default_hold_frames": 1.25,
            "default_tempo_scale": 0.95,
            "game_fps": 120,
            "telemetry_enabled": True,
            "verbose_hud": True,
            "update_auto_check": False,
            "update_channel": " BETA ",
            "update_skip_version": " 3.6.0 ",
        }
    )
    before_invalid = asdict(service.snapshot())
    try:
        service.patch({"theme": "cyberpunk", "update_channel": "nightly"})
    except ValueError as error:
        invalid_error = str(error)
    else:  # pragma: no cover - a changed oracle must fail fixture generation
        raise RuntimeError("current settings oracle unexpectedly accepted invalid patch")
    return {
        "schema": 1,
        "config_layouts": config_layout_fixture(),
        "default_normalized": asdict(normalize_application_settings(malformed)),
        "valid_patch": asdict(valid),
        "invalid_patch": {"error": invalid_error, "state_after": asdict(service.snapshot())},
        "state_before_invalid": before_invalid,
    }


def config_layout_fixture() -> dict[str, object]:
    """Capture migration and preservation behavior from the current config oracle."""
    original_path = config_module.CONFIG_PATH
    try:
        with tempfile.TemporaryDirectory(prefix="sky-wave2-fixture-") as directory:
            path = Path(directory) / "config.json"
            legacy = {
                "schema_version": 2,
                "default_timing_profile": "audience-safe",
                "timing_profiles": {"audience_safe": {"min_hold_frames": 1.5}},
                "future_field": {"keep": True},
            }
            path.write_text(json.dumps(legacy), encoding="utf-8")
            config_module.CONFIG_PATH = path
            config_module.clear_config_cache()
            loaded_legacy = config_module.load_config(force_reload=True)
            migrated = json.loads(path.read_text(encoding="utf-8"))

            current = {
                "schema_version": 3,
                "theme": " SLATE ",
                "default_hold_frames": 1.25,
                "game_fps": 120,
                "future_field": ["preserve"],
                "update": {"channel": "BETA", "unknown": 7},
            }
            path.write_text(json.dumps(current), encoding="utf-8")
            config_module.clear_config_cache()
            loaded_current = config_module.load_config(force_reload=True)
            normalized_current = normalize_application_settings(loaded_current)
            return {
                "legacy_v2": {
                    "input": legacy,
                    "normalized_hold_frames": loaded_legacy.default_hold_frames,
                    "migrated_schema_version": migrated.get("schema_version"),
                    "migrated_has_legacy_profile": "default_timing_profile" in migrated,
                    "unknown_field_preserved": migrated.get("future_field"),
                },
                "current_v3": {
                    "input": current,
                    "normalized_theme": normalized_current.theme,
                    "normalized_channel": normalized_current.update_preferences.channel,
                    "normalized_fps": normalized_current.game_fps,
                },
            }
    finally:
        config_module.CONFIG_PATH = original_path
        config_module.clear_config_cache()


def catalog_fixture() -> dict[str, object]:
    paths = [
        Path("C:/Wave2/Songs/Dandelions.txt"),
        Path("C:/Wave2/Songs/Cà Phê.json"),
        Path("C:/Wave2/Songs/Đàn Bay.skysheet"),
        Path("C:/Wave2/Songs/ignored.csv"),
    ]
    service = CatalogService("C:/Wave2/Songs")
    snapshot = service.replace_paths(paths)
    queries = {}
    for query in ("", "ca phe", "dan", "d"):
        page = service.search_window(query, offset=0, limit=20)
        queries[query] = {
            "generation": page.generation,
            "offset": page.offset,
            "limit": page.limit,
            "total": page.total,
            "items": [asdict(item) for item in page.items],
        }
    return {
        "schema": 1,
        "paths": [str(path) for path in paths],
        "snapshot": {"generation": snapshot.generation, "total": snapshot.total, "items": [asdict(item) for item in snapshot.items]},
        "queries": queries,
        "normalized": {value: __import__("sky_music.orchestration.catalog_service", fromlist=["normalize_search_text"]).normalize_search_text(value) for value in ("Cà Phê", "Đàn", "Élan")},
        "stale_generation_error": "catalog generation is stale",
    }


def update_fixture() -> dict[str, object]:
    preferences = UpdateSettings(last_check_ts=1_000, last_error_ts=0)
    retry_preferences = UpdateSettings(last_error_ts=1_000)
    return {
        "schema": 1,
        "channels": {
            channel: {
                "include_prerelease": get_policy(channel).include_prerelease,
                "github_api_path": get_policy(channel).github_api_path,
            }
            for channel in ("stable", "beta", "unknown")
        },
        "throttle": {
            "within_success_interval": should_auto_check(AppConfig(update=preferences), now_ts=1_500),
            "at_success_boundary": should_auto_check(AppConfig(update=preferences), now_ts=1_000 + 86_400),
            "retry_delay": retry_delay_for(AppConfig(update=retry_preferences), now_ts=1_100),
            "retry_at_boundary": should_auto_check(AppConfig(update=retry_preferences), now_ts=1_300),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=Path("tests/fixtures/wave2"))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    fixtures = {"settings": settings_fixture(), "catalog": catalog_fixture(), "update": update_fixture()}
    for name, payload in fixtures.items():
        (args.output / f"{name}.json").write_text(
            json.dumps(payload, indent=2, ensure_ascii=False, default=str) + "\n",
            encoding="utf-8",
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
