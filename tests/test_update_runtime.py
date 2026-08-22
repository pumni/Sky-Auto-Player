from __future__ import annotations

import json
from pathlib import Path

from sky_music.infrastructure import update_runtime


def _write_result(root: Path, payload: dict[str, object]) -> Path:
    path = root / update_runtime.APP_NAME / "update-state" / "last-result.json"
    path.parent.mkdir(parents=True)
    path.write_text(json.dumps(payload), encoding="utf-8")
    return path


def test_consume_last_result_moves_valid_result_aside(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))
    path = _write_result(
        tmp_path,
        {
            "schema_version": 1,
            "status": "success",
            "from_version": "2.4.0",
            "target_version": "2.5.0",
            "timestamp_utc": "2026-08-09T00:00:00Z",
            "error_code": None,
            "message": None,
        },
    )

    result = update_runtime.consume_last_result()

    assert result is not None
    assert result.status == "success"
    assert not path.exists()
    assert path.with_name("last-result.json.consumed").exists()
    assert update_runtime.consume_last_result() is None


def test_invalid_result_is_left_for_diagnostics(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))
    path = _write_result(
        tmp_path,
        {
            "schema_version": 1,
            "status": "success",
            "from_version": "2.4.0",
            "target_version": "2.5.0",
            "timestamp_utc": "not-a-timestamp",
        },
    )

    assert update_runtime.consume_last_result() is None
    assert path.exists()


def test_schema_one_result_preserves_provenance_and_cleanup_warnings(
    monkeypatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))
    _write_result(
        tmp_path,
        {
            "schema_version": 1,
            "status": "success",
            "from_version": "2.4.0",
            "target_version": "2.5.0",
            "timestamp_utc": "2026-08-09T00:00:00Z",
            "error_code": None,
            "message": None,
            "phase": "CleaningUp",
            "operation": "remove updater-owned artifact",
            "path": ".sky-update-1-2.bak",
            "os_error": 5,
            "warnings": [
                {
                    "code": "ARTIFACT_CLEANUP_FAILED",
                    "message": "Access is denied.",
                    "phase": "cleanup",
                    "operation": "remove updater-owned artifact",
                    "path": ".sky-update-1-2.bak",
                    "os_error": 5,
                }
            ],
            "cleanup_pending": True,
        },
    )

    result = update_runtime.consume_last_result()

    assert result is not None
    assert result.phase == "CleaningUp"
    assert result.operation == "remove updater-owned artifact"
    assert result.path == ".sky-update-1-2.bak"
    assert result.os_error == 5
    assert result.cleanup_pending is True
    assert result.warnings[0].code == "ARTIFACT_CLEANUP_FAILED"
