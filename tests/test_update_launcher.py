from __future__ import annotations

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


def test_launch_stages_hash_verified_updater_and_uses_explicit_args(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    root = tmp_path / "install"
    root.mkdir()
    (root / "Sky-Auto-Player.exe").write_bytes(b"app")
    bundled = root / update_launcher.UPDATER_NAME
    bundled.write_bytes(b"signed-updater")
    local = tmp_path / "local-app-data"
    monkeypatch.setenv("LOCALAPPDATA", str(local))
    monkeypatch.setattr(update_launcher.sys, "platform", "win32")
    monkeypatch.setattr(update_launcher.sys, "frozen", True, raising=False)
    calls: list[tuple[list[str], dict[str, object]]] = []

    def fake_popen(arguments: list[str], **kwargs: object) -> object:
        calls.append((arguments, kwargs))
        return object()

    monkeypatch.setattr(update_launcher.subprocess, "Popen", fake_popen)

    run_dir = update_launcher.launch_update(_request(root))

    staged = run_dir / update_launcher.UPDATER_NAME
    assert staged.read_bytes() == bundled.read_bytes()
    assert calls and calls[0][0][0] == str(staged)
    assert calls[0][0][1:] == [
        "--install-root",
        str(root),
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
    assert calls[0][1]["shell"] is False


def test_launcher_rejects_unsafe_request(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    root = tmp_path / "install"
    root.mkdir()
    (root / "Sky-Auto-Player.exe").write_bytes(b"app")
    monkeypatch.setattr(update_launcher.sys, "platform", "win32")
    monkeypatch.setattr(update_launcher.sys, "frozen", True, raising=False)

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
