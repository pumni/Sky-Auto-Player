from __future__ import annotations

import hashlib
import math
from dataclasses import FrozenInstanceError
from pathlib import Path

import pytest

import sky_music.orchestration.catalog_service as catalog_module
import sky_music.orchestration.settings_service as settings_module
import sky_music.orchestration.song_metadata_service as metadata_module
from sky_music.config import AppConfig
from sky_music.orchestration.catalog_service import (
    CatalogCollisionError,
    CatalogGenerationError,
    CatalogLookupError,
    CatalogService,
    song_id_for_path,
)
from sky_music.orchestration.desktop_models import (
    BootstrapDto,
    DiagnosticsSnapshotDto,
    NativeBuildDto,
    PlaybackConfigDto,
    PlaybackOptionSetsDto,
    PlaybackRecommendationDto,
    PlaybackSnapshotDto,
    PreparedPlaybackDto,
    RiskDecisionDto,
    RiskSummaryDto,
    SongDetailDto,
    SongRowDto,
    UpdatePreferencesDto,
)
from sky_music.orchestration.playback_controller import (
    PlaybackPlan as SharedPlaybackPlan,
)
from sky_music.orchestration.settings_service import (
    SettingsService,
    normalize_application_settings,
)
from sky_music.orchestration.song_metadata_service import MetadataPrioritySnapshot


def test_catalog_scan_ids_search_pages_and_backend_lookup(tmp_path: Path) -> None:
    (tmp_path / "Cà Phê.json").write_text("{}", encoding="utf-8")
    (tmp_path / "Dandelions.txt").write_text("", encoding="utf-8")
    (tmp_path / "ignore.csv").write_text("", encoding="utf-8")
    (tmp_path / "not-a-file.json").mkdir()

    service = CatalogService(tmp_path)
    snapshot = service.scan()

    assert snapshot.total == 2
    assert snapshot.generation == 1
    assert [row.title for row in snapshot.items] == ["Cà Phê", "Dandelions"]
    first_id = snapshot.items[0].song_id
    expected = hashlib.sha256(
        catalog_module.normalized_canonical_path(tmp_path / "Cà Phê.json").encode("utf-8")
    ).hexdigest()[:32]
    assert first_id == expected == song_id_for_path(tmp_path / "Cà Phê.json")
    assert service.path_for_song_id(first_id) == tmp_path / "Cà Phê.json"

    result = service.search("ca phe", page_size=1)
    assert result.total == 1
    assert result.items[0].song_id == first_id
    assert result.has_next is False
    with pytest.raises(CatalogLookupError):
        service.path_for_song_id("not-an-id")

    service.replace_paths([tmp_path / "Cà Phê.json"])
    with pytest.raises(CatalogGenerationError):
        service.search("cafe", generation=1)


def test_catalog_rejects_id_collision(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    first = tmp_path / "first.json"
    second = tmp_path / "second.json"
    first.write_text("{}", encoding="utf-8")
    second.write_text("{}", encoding="utf-8")
    monkeypatch.setattr(catalog_module, "song_id_for_path", lambda _path: "0" * 32)

    with pytest.raises(CatalogCollisionError):
        CatalogService(tmp_path).replace_paths([first, second])


def test_catalog_search_keeps_rows_and_generation_from_one_atomic_snapshot(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    first = tmp_path / "Alpha.json"
    second = tmp_path / "Beta.json"
    first.write_text("{}", encoding="utf-8")
    second.write_text("{}", encoding="utf-8")
    service = CatalogService(tmp_path)
    service.replace_paths([first])
    original_rank = CatalogService.rank_search_keys

    def reload_during_ranking(
        search_keys: list[str],
        query: str = "",
        *,
        score_cutoff: float = catalog_module.FUZZY_SCORE_CUTOFF,
    ) -> tuple[int, ...]:
        service.replace_paths([second])
        return original_rank(search_keys, query, score_cutoff=score_cutoff)

    monkeypatch.setattr(
        CatalogService,
        "rank_search_keys",
        staticmethod(reload_during_ranking),
    )

    result = service.search("alpha")

    assert result.generation == 1
    assert [row.title for row in result.items] == ["Alpha"]
    assert service.generation == 2


def test_catalog_path_lookup_rejects_stale_generation_inside_lookup_lock(tmp_path: Path) -> None:
    song = tmp_path / "Alpha.json"
    song.write_text("{}", encoding="utf-8")
    service = CatalogService(tmp_path)
    snapshot = service.scan()
    song_id = snapshot.items[0].song_id
    service.replace_paths([])

    with pytest.raises(CatalogGenerationError):
        service.path_for_song_id(song_id, generation=snapshot.generation)


def test_catalog_tied_titles_preserve_input_order(tmp_path: Path) -> None:
    json_path = tmp_path / "Same.json"
    txt_path = tmp_path / "Same.txt"
    json_path.write_text("{}", encoding="utf-8")
    txt_path.write_text("", encoding="utf-8")

    service = CatalogService(tmp_path)
    service.replace_paths([txt_path, json_path])
    assert [entry.path for entry in service.entries()] == [txt_path, json_path]


def test_settings_service_normalizes_config_without_exposing_raw_layout() -> None:
    cfg = AppConfig(
        theme="not-a-theme",
        ui_background_mode="not-a-mode",
        default_hold_frames=99.0,
        default_tempo_scale=float("nan"),
        game_fps=999,
        songs_dir="custom-songs",
        sky_process_names=[" Sky.exe ", 42],  # type: ignore[list-item]
    )

    settings = normalize_application_settings(cfg)

    assert settings.theme == "aurora"
    assert settings.ui_background_mode == "transparent"
    assert settings.default_hold_frames == 1.0
    assert settings.default_tempo_scale == 1.0
    assert settings.game_fps == 60
    assert settings.songs_dir == Path("custom-songs")
    assert settings.sky_process_names == ("Sky.exe",)
    assert settings.playback_defaults.hold_frames == 1.0
    assert not hasattr(settings, "config_path")


def test_settings_service_persists_all_writes_through_one_boundary(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cfg = AppConfig()
    saved: list[AppConfig] = []
    monkeypatch.setattr(settings_module, "save_config", saved.append)
    service = SettingsService(cfg)

    service.set_hold_frames(1.25)
    service.set_tempo_scale(0.9)
    service.set_fps(120)
    service.set_theme(" CYBERPUNK ")
    service.set_telemetry_enabled(True)
    service.set_verbose_hud(True)

    assert len(saved) == 6
    assert cfg.default_hold_frames == 1.25
    assert cfg.default_tempo_scale == 0.9
    assert cfg.game_fps == 120
    assert cfg.theme == "cyberpunk"
    assert cfg.telemetry_enabled_by_default is True
    assert cfg.verbose_hud is True


@pytest.mark.parametrize("tempo", [math.nan, math.inf, -math.inf, 0.0, -1.0])
def test_settings_service_rejects_invalid_tempo_without_persisting(
    monkeypatch: pytest.MonkeyPatch, tempo: float
) -> None:
    cfg = AppConfig(default_tempo_scale=0.95)
    saved: list[AppConfig] = []
    monkeypatch.setattr(settings_module, "save_config", saved.append)
    service = SettingsService(cfg)

    with pytest.raises(ValueError):
        service.set_tempo_scale(tempo)

    assert cfg.default_tempo_scale == 0.95
    assert saved == []


def test_settings_service_rejects_invalid_write_enums_and_booleans(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cfg = AppConfig()
    saved: list[AppConfig] = []
    monkeypatch.setattr(settings_module, "save_config", saved.append)
    service = SettingsService(cfg)

    with pytest.raises(ValueError):
        service.set_theme("unknown-theme")
    with pytest.raises(ValueError):
        service.set_fps(61)
    with pytest.raises(ValueError):
        service.set_telemetry_enabled(1)  # type: ignore[arg-type]
    with pytest.raises(ValueError):
        service.set_verbose_hud("false")  # type: ignore[arg-type]

    assert saved == []
    assert cfg.theme == "aurora"
    assert cfg.game_fps == 60


def test_settings_service_persists_normalized_update_preferences_atomically(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cfg = AppConfig()
    saved: list[AppConfig] = []
    monkeypatch.setattr(settings_module, "save_config", saved.append)
    service = SettingsService(cfg)

    settings = service.patch(
        {
            "update_auto_check": False,
            "update_channel": " BETA ",
            "update_skip_version": " 3.6.0 ",
        }
    )

    assert settings.update_preferences.auto_check is False
    assert settings.update_preferences.channel == "beta"
    assert settings.update_preferences.skip_version == "3.6.0"
    assert len(saved) == 1

    before = service.snapshot()
    with pytest.raises(ValueError):
        service.patch({"update_auto_check": True, "update_channel": "nightly"})
    assert service.snapshot() == before
    assert len(saved) == 1


def test_metadata_priority_preserves_selected_visible_overscan_filtered_order(tmp_path: Path) -> None:
    selected = tmp_path / "selected.json"
    visible = tmp_path / "visible.json"
    overscan = tmp_path / "overscan.json"
    filtered = tmp_path / "filtered.json"
    snapshot = MetadataPrioritySnapshot(
        selected=[selected],
        visible=[selected, visible],
        overscan=[visible, overscan],
        filtered=[overscan, filtered],
    )

    assert snapshot.ordered_paths() == [selected, visible, overscan, filtered]


def test_phase1_compatibility_aliases_share_orchestration_implementations() -> None:
    import sky_music.ui.picker_metadata as legacy_metadata
    import sky_music.ui.textual_app.playback_controller as legacy_playback

    assert legacy_metadata is metadata_module
    assert legacy_metadata.SongUiMetadata is metadata_module.SongUiMetadata
    assert legacy_playback.PlaybackPlan is SharedPlaybackPlan


def test_desktop_dtos_are_frozen_and_slotted() -> None:
    config_dto = PlaybackConfigDto(1.0, 1.0, 60, False)
    native_build = NativeBuildDto("a" * 40, "3.5.0", 4, "cp314t-win_amd64", "1.98.0", True)
    option_sets = PlaybackOptionSetsDto((1.0, 1.25, 1.5), (0.9, 1.0, 1.1), (30, 60, 120))
    risk = RiskSummaryDto("low", "Low risk", (), ())
    recommendation = PlaybackRecommendationDto(1.0, 1.0, "Keep the selected settings.")
    detail = SongDetailDto("a" * 32, "Song", None, None, "json", risk, recommendation)
    decision = RiskDecisionDto("use_recommended", "Use recommended settings")
    prepared = PreparedPlaybackDto("prepared", detail, config_dto, "ready", risk, (decision,))
    bootstrap = BootstrapDto(
        "3.5.0",
        1,
        native_build,
        config_dto,
        option_sets,
        "aurora",
        False,
        UpdatePreferencesDto(True, "stable", ""),
        1,
    )
    row = SongRowDto("a" * 32, "Song", None, None, "unknown", "pending")
    playback = PlaybackSnapshotDto(1, "playing", "a" * 32, "Song", 0, 1, 0, "focused", "healthy", False, None)
    diagnostics = DiagnosticsSnapshotDto(1, 0, 0.0, 0.0, 0.0, 0, 0, 0, 0, 0, 0, 0, "ready", None, None)

    with pytest.raises(FrozenInstanceError):
        config_dto.fps = 120  # type: ignore[misc]

    for dto in (bootstrap, native_build, option_sets, row, detail, recommendation, risk, decision, prepared, playback, diagnostics):
        assert hasattr(type(dto), "__dataclass_fields__")
        assert hasattr(type(dto), "__slots__")
        with pytest.raises(AttributeError):
            dto.extra = "not allowed"  # type: ignore[attr-defined]
