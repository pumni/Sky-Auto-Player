from __future__ import annotations

import hashlib
import json
import os
import time
from pathlib import Path

import pytest

from sky_music.infrastructure import update_launcher


def _request(root: Path, *, target: str = "2.5.0") -> update_launcher.UpdateLaunchRequest:
    return update_launcher.UpdateLaunchRequest(
        install_root=root,
        current_version="2.4.0",
        target_version=target,
        channel="stable",
    )


def _write_install_manifest(root: Path, *, updater: bytes = b"updater") -> None:
    files = []
    for name, data in {
        "Sky-Auto-Player.exe": b"app",
        "native_calibration.exe": b"calibration",
        update_launcher.UPDATER_NAME: updater,
    }.items():
        (root / name).write_bytes(data)
        files.append(
            {
                "path": name,
                "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    (root / "MANIFEST.json").write_text(
        json.dumps(
            {
                "schema_version": 2,
                "app": update_launcher.APP_NAME,
                "version": "2.4.0",
                "executable": "Sky-Auto-Player.exe",
                "files": files,
            }
        ),
        encoding="utf-8",
    )


def test_launch_stages_verified_native_updater(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    calls: list[tuple[list[str], dict[str, object]]] = []

    def fake_popen(arguments: list[str], **kwargs: object) -> object:
        calls.append((arguments, kwargs))
        return object()

    monkeypatch.setattr(update_launcher.subprocess, "Popen", fake_popen)
    copied = update_launcher.launch_update(_request(root))

    assert copied.name == update_launcher.UPDATER_NAME
    assert copied.read_bytes() == b"updater"
    assert len(calls) == 1
    arguments, kwargs = calls[0]
    assert arguments[0] == str(copied)
    assert arguments[1:] == [
        "--install-root",
        str(root.resolve()),
        "--parent-pid",
        str(os.getpid()),
        "--current-version",
        "2.4.0",
        "--target-version",
        "2.5.0",
        "--channel",
        "stable",
        "--restart",
    ]
    assert kwargs["shell"] is False


def test_launch_rejects_tampered_installed_updater(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    (root / update_launcher.UPDATER_NAME).write_bytes(b"tampered")
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "local"))
    with pytest.raises(update_launcher.UpdateLaunchError, match="does not match"):
        update_launcher.launch_update(_request(root))


def test_launch_failure_cleans_created_run(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    _write_install_manifest(root)
    local = tmp_path / "local"
    monkeypatch.setenv("LOCALAPPDATA", str(local))

    def fail_popen(*_args: object, **_kwargs: object) -> object:
        raise OSError("spawn denied")

    monkeypatch.setattr(update_launcher.subprocess, "Popen", fail_popen)
    with pytest.raises(update_launcher.UpdateLaunchError, match="could not launch"):
        update_launcher.launch_update(_request(root))
    assert list((local / update_launcher.APP_NAME / "update-runs").iterdir()) == []


def test_launcher_request_validation_remains_strict(tmp_path: Path) -> None:
    root = tmp_path / "install"
    root.mkdir()
    (root / "Sky-Auto-Player.exe").write_bytes(b"app")

    with pytest.raises(update_launcher.UpdateLaunchError, match="stable channel"):
        update_launcher._validate_request(_request(root, target="2.5.0rc1"))


def test_cleanup_only_removes_old_owned_run_directories(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))
    runs = tmp_path / update_launcher.APP_NAME / "update-runs"
    old_run = runs / ("run-" + "a" * 32)
    recent_run = runs / ("run-" + "b" * 32)
    unknown = runs / "run-not-owned"
    old_run.mkdir(parents=True)
    recent_run.mkdir()
    unknown.mkdir()
    (old_run / "payload").write_text("old", encoding="utf-8")
    old_timestamp = time.time() - 120
    os.utime(old_run, (old_timestamp, old_timestamp))

    assert update_launcher.cleanup_stale_update_runs(max_age_s=60) == 1
    assert not old_run.exists()
    assert recent_run.exists()
    assert unknown.exists()
