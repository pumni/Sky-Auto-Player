from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

import sky_music.orchestration.catalog_service as catalog_module
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
    PlaybackConfigDto,
    PlaybackSnapshotDto,
    PreparedPlaybackDto,
    RiskSummaryDto,
    SongDetailDto,
    SongRowDto,
    UpdatePreferencesDto,
)
from sky_music.orchestration.settings_service import normalize_application_settings
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


def test_desktop_dtos_are_frozen_and_slotted() -> None:
    config_dto = PlaybackConfigDto(1.0, 1.0, 60, False)
    risk = RiskSummaryDto("low", "Low risk", (), ())
    detail = SongDetailDto("a" * 32, "Song", None, None, "json", "low", "Ready")
    prepared = PreparedPlaybackDto("prepared", detail, config_dto, "ready", risk, ())
    bootstrap = BootstrapDto(
        "3.5.0",
        1,
        "native",
        config_dto,
        (),
        "aurora",
        False,
        UpdatePreferencesDto(True, "stable", ""),
        1,
    )
    row = SongRowDto("a" * 32, "Song", None, None, "unknown", "pending")
    playback = PlaybackSnapshotDto(1, "playing", "a" * 32, "Song", 0, 1, 0, "focused", "healthy", False, None)
    diagnostics = DiagnosticsSnapshotDto(1, 0, 0.0, 0.0, 0.0, 0, 0, 0, (), (), 0, 0, "ready", None, None)

    for dto in (bootstrap, row, detail, risk, prepared, playback, diagnostics):
        assert hasattr(type(dto), "__dataclass_fields__")
        assert hasattr(type(dto), "__slots__")
        with pytest.raises(AttributeError):
            dto.extra = "not allowed"  # type: ignore[attr-defined]
