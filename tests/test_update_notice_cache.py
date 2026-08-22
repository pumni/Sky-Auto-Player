from __future__ import annotations

import json
from pathlib import Path

from sky_music.domain.update_checker import UpdateInfo
from sky_music.infrastructure import update_notice_cache


def _release(notes: str = "## Changes\n- visible") -> UpdateInfo:
    return UpdateInfo(
        latest_version="3.4.5",
        download_url="https://signed.example.invalid/payload.zip",
        release_notes=notes,
        html_url="https://github.com/pumni/Sky-Auto-Player/releases/tag/v3.4.5",
        published_at="2026-08-22T00:00:00Z",
    )


def test_pending_release_round_trip_does_not_persist_urls(
    monkeypatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))

    update_notice_cache.save_pending_release(_release())
    loaded = update_notice_cache.load_pending_release()

    assert loaded is not None
    assert loaded.latest_version == "3.4.5"
    assert loaded.release_notes == "## Changes\n- visible"
    raw = (tmp_path / "Sky-Auto-Player" / "update-state" / "pending-release.json").read_text(
        encoding="utf-8"
    )
    assert "signed.example.invalid" not in raw
    assert "github.com" not in raw


def test_pending_release_notes_are_bounded_at_utf8_boundary(
    monkeypatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))

    update_notice_cache.save_pending_release(_release("ế" * 100_000))
    loaded = update_notice_cache.load_pending_release()

    assert loaded is not None
    assert len(loaded.release_notes.encode("utf-8")) <= update_notice_cache._MAX_NOTES_BYTES
    assert "[Release notes truncated;" in loaded.release_notes


def test_pending_release_invalid_json_and_version_scoped_clear(
    monkeypatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))
    update_notice_cache.save_pending_release(_release())

    update_notice_cache.clear_pending_release("3.4.4")
    assert update_notice_cache.load_pending_release() is not None
    update_notice_cache.clear_pending_release("3.4.5")
    assert update_notice_cache.load_pending_release() is None

    path = tmp_path / "Sky-Auto-Player" / "update-state" / "pending-release.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"schema_version": 1, "latest_version": "bad"}), encoding="utf-8")
    assert update_notice_cache.load_pending_release() is None
