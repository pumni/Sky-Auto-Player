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
    """Capture persisted behavior by exercising the real Python disk loader."""
    def rust_settings_shape(settings: AppConfig) -> dict[str, object]:
        normalized = normalize_application_settings(settings)
        return {
            "theme": normalized.theme,
            "ui_background_mode": normalized.ui_background_mode,
            "playback_defaults": {
                "hold_frames": normalized.default_hold_frames,
                "tempo_scale": normalized.default_tempo_scale,
                "fps": normalized.game_fps,
            },
            "telemetry_enabled": normalized.telemetry_enabled,
            "verbose_hud": normalized.verbose_hud,
            "songs_dir": str(normalized.songs_dir),
            "sky_process_names": list(normalized.sky_process_names),
            "allow_title_fallback": normalized.allow_title_fallback,
            "hotkeys": asdict(normalized.hotkeys),
            "safety": asdict(normalized.safety),
            "update": {
                **asdict(normalized.update_preferences),
                "last_check_ts": settings.update.last_check_ts,
                "last_error_ts": settings.update.last_error_ts,
                "last_notified_version": settings.update.last_notified_version,
                "legacy_old_dir_sweep_pending": settings.update.legacy_old_dir_sweep_pending,
            },
        }

    cases: list[dict[str, object]] = []
    original_path = config_module.CONFIG_PATH
    try:
        with tempfile.TemporaryDirectory(prefix="sky-wave2-fixture-") as directory:
            path = Path(directory) / "config.json"
            config_module.CONFIG_PATH = path

            def capture(name: str, raw: object, *, raw_text: str | None = None) -> None:
                path.unlink(missing_ok=True)
                if raw_text is not None:
                    path.write_text(raw_text, encoding="utf-8")
                elif raw is not None:
                    path.write_text(json.dumps(raw), encoding="utf-8")
                config_module.clear_config_cache()
                loaded = config_module.load_config(force_reload=True)
                migrated = json.loads(path.read_text(encoding="utf-8"))
                cases.append(
                    {
                        "name": name,
                        "input": raw if raw_text is None else raw_text,
                        "normalized": rust_settings_shape(loaded),
                        "migrated": migrated,
                    }
                )

            capture("missing_file", None)
            capture("malformed_json", {}, raw_text="{not valid json")
            capture("non_object_root", [], raw_text="[1, 2, 3]")
            capture(
                "v2_profile_fallback",
                {"schema_version": 2, "default_timing_profile": "balanced"},
            )
            capture(
                "v2_min_hold_frames_overrides_profile",
                {
                    "schema_version": 2,
                    "default_timing_profile": "balanced",
                    "timing_profiles": {"balanced": {"min_hold_frames": 1.5}},
                },
            )
            capture(
                "v2_top_level_hold_wins",
                {
                    "schema_version": 2,
                    "default_hold_frames": 1.25,
                    "default_timing_profile": "audience-safe",
                    "timing_profiles": {"audience_safe": {"min_hold_frames": 1.5}},
                },
            )
            capture(
                "v2_hold_frames",
                {
                    "schema_version": 2,
                    "default_timing_profile": "balanced",
                    "timing_profiles": {"balanced": {"hold_frames": 1.25}},
                },
            )
            capture(
                "v2_min_hold_us_at_120_fps",
                {
                    "schema_version": 2,
                    "game_fps": 120,
                    "default_timing_profile": "balanced",
                    "timing_profiles": {"balanced": {"min_hold_us": 12501}},
                },
            )
            capture(
                "v2_hold_us_at_90_fps",
                {
                    "schema_version": 2,
                    "game_fps": 90,
                    "default_timing_profile": "balanced",
                    "timing_profiles": {"balanced": {"hold_us": 13890}},
                },
            )
            capture(
                "v2_legacy_keys_removed",
                {
                    "schema_version": 2,
                    "default_timing_profile": "balanced",
                    "timing_profiles": {},
                    "frame_timing": {"legacy": True},
                    "hold_us": 10_000,
                    "min_hold_us": 10_000,
                    "hold_frames": 1.5,
                    "min_hold_frames": 1.5,
                    "hold_unframed_us": 10_000,
                    "min_hold_unframed_us": 10_000,
                    "future_field": "preserve",
                },
            )
            capture(
                "v3_string_coercions_and_mixed_lists",
                {
                    "schema_version": 3,
                    "default_hold_frames": "1.25",
                    "default_tempo_scale": "0.95",
                    "game_fps": "120",
                    "songs_dir": {"legacy": True},
                    "sky_process_names": [42, None, True, {"name": "Sky.exe"}],
                    "hotkeys": {"pause": 7, "skip": None, "quit": ["f10", 1]},
                    "update": {
                        "auto_check": "false",
                        "channel": "BETA",
                        "skip_version": 42,
                        "check_interval_s": "300",
                        "last_check_ts": "1",
                    },
                },
            )
            capture(
                "v3_unknown_nested_fields",
                {
                    "schema_version": 3,
                    "default_hold_frames": 1.0,
                    "theme": " SLATE ",
                    "future_field": {"keep": [1, 2, 3]},
                    "update": {"channel": "BETA", "unknown": {"keep": True}},
                },
            )
            capture(
                "v3_null_and_malformed_values",
                {
                    "schema_version": 3,
                    "default_hold_frames": None,
                    "default_tempo_scale": None,
                    "game_fps": None,
                    "theme": None,
                    "songs_dir": None,
                    "sky_process_names": "not-a-list",
                    "hotkeys": None,
                    "update": None,
                },
            )
            legacy = next(case for case in cases if case["name"] == "v2_min_hold_frames_overrides_profile")
            current = next(case for case in cases if case["name"] == "v3_unknown_nested_fields")
            return {
                "legacy_v2": {
                    "input": legacy["input"],
                    "normalized_hold_frames": legacy["normalized"]["playback_defaults"]["hold_frames"],
                    "migrated_schema_version": legacy["migrated"].get("schema_version"),
                    "migrated_has_legacy_profile": "default_timing_profile" in legacy["migrated"],
                    "unknown_field_preserved": None,
                },
                "current_v3": {
                    "input": current["input"],
                    "normalized_theme": current["normalized"]["theme"],
                    "normalized_channel": current["normalized"]["update"]["channel"],
                    "normalized_fps": current["normalized"]["playback_defaults"]["fps"],
                },
                "persisted_cases": cases,
            }
    finally:
        config_module.CONFIG_PATH = original_path
        config_module.clear_config_cache()


def catalog_fixture() -> dict[str, object]:
    paths = [
        Path("C:/Wave2/Songs/Dandelions.txt"),
        Path("C:/Wave2/Songs/Cà Phê.json"),
        Path("C:/Wave2/Songs/Đàn Bay.skysheet"),
        Path("C:/Wave2/Songs/Straße.json"),
        Path("C:/Wave2/Songs/STRASSE.txt"),
        Path("C:/Wave2/Songs/ΟΣ.skysheet"),
        Path("C:/Wave2/Songs/ος.json"),
        Path("C:/Wave2/Songs/Élan.json"),
        Path("C:/Wave2/Songs/ALPHA.json"),
        Path("C:/Wave2/Songs/alpha.txt"),
        Path("C:/Wave2/Songs/Dandelions.txt"),  # exact duplicate is ignored
        Path("C:/Wave2/Songs/ignored.csv"),
    ]
    service = CatalogService("C:/Wave2/Songs")
    snapshot = service.replace_paths(paths)

    normalize = __import__(
        "sky_music.orchestration.catalog_service",
        fromlist=["normalize_search_text"],
    ).normalize_search_text
    substring_queries = {}
    for query in ("", "ca phe", "dan", "d", "strasse", "οσ", "ss"):
        normalized_query = normalize(query).strip()
        items = [
            item
            for item in snapshot.items
            if not normalized_query or normalized_query in normalize(item.title)
        ]
        substring_queries[query] = {
            "generation": snapshot.generation,
            "offset": 0,
            "limit": 20,
            "total": len(items),
            "items": [asdict(item) for item in items[:20]],
        }

    def window_case(name: str, query: str, *, offset: int = 0, limit: int = 20) -> dict[str, object]:
        try:
            page = service.search_window(query, offset=offset, limit=limit)
        except Exception as error:  # the exception type/message are oracle evidence
            return {
                "name": name,
                "query": query,
                "offset": offset,
                "limit": limit,
                "status": "error",
                "error_type": type(error).__name__,
                "error": str(error),
            }
        return {
            "name": name,
            "query": query,
            "offset": offset,
            "limit": limit,
            "status": "ok",
            "page": {
                "generation": page.generation,
                "offset": page.offset,
                "limit": page.limit,
                "total": page.total,
                "items": [asdict(item) for item in page.items],
            },
        }

    window_cases = [
        window_case("offset_zero", "", offset=0, limit=20),
        window_case("offset_positive", "", offset=2, limit=3),
        window_case("offset_beyond_total", "", offset=1_000_000_001, limit=1),
        window_case("limit_zero", "", offset=0, limit=0),
        window_case("limit_above_max", "", offset=0, limit=201),
        window_case("query_exactly_1024_ascii", "a" * 1024),
        window_case("query_too_long_1025_ascii", "a" * 1025),
        window_case("query_exactly_1024_unicode", "é" * 1024),
        window_case("query_too_long_1025_unicode", "é" * 1025),
        window_case("query_multibyte_under_codepoint_limit", "é" * 600),
    ]
    return {
        "schema": 1,
        "paths": [str(path) for path in paths],
        "snapshot": {"generation": snapshot.generation, "total": snapshot.total, "items": [asdict(item) for item in snapshot.items]},
        "substring_queries": substring_queries,
        "normalized": {
            value: normalize(value)
            for value in (
                "Cà Phê",
                "Đàn",
                "Élan",
                "Straße",
                "STRASSE",
                "ΟΣ",
                "ος",
            )
        },
        "window_cases": window_cases,
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
