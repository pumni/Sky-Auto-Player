"""Capture deterministic Wave 2 migration evidence from the current Python oracle.

This is an explicit maintainer tool. CI consumes the committed JSON fixtures;
it never regenerates them implicitly. Run it only when deliberately updating
the migration evidence after reviewing a current-behavior change.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import asdict
from pathlib import Path

from sky_music.config import AppConfig, UpdateSettings
from sky_music.domain.update_policy import get_policy
from sky_music.orchestration.catalog_service import CatalogService
from sky_music.orchestration.settings_service import SettingsService, normalize_application_settings
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
        "default_normalized": asdict(normalize_application_settings(malformed)),
        "valid_patch": asdict(valid),
        "invalid_patch": {"error": invalid_error, "state_after": asdict(service.snapshot())},
        "state_before_invalid": before_invalid,
    }


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
