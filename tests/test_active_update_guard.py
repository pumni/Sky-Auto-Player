from __future__ import annotations

import hashlib
import json
from pathlib import Path

from sky_music.infrastructure import update_runtime
from sky_music.platform.win32 import process_state


def _install_id(root: Path) -> str:
    normalized = str(root.resolve()).replace("/", "\\").lower()
    return hashlib.sha256(normalized.encode("utf-8")).hexdigest()


def _write_active(local: Path, install: Path, run_id: str, pid: int = 1234) -> Path:
    state = local / "Sky-Auto-Player" / "update-state" / "active-update.json"
    state.parent.mkdir(parents=True)
    state.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "install_id": _install_id(install),
                "run_id": run_id,
                "updater_pid": pid,
                "target_version": "3.4.5",
                "phase": "Installing",
                "started_at_utc": "2026-08-22T00:00:00Z",
                "updated_at_utc": "2026-08-22T00:00:10Z",
            }
        ),
        encoding="utf-8",
    )
    return state


def test_live_canonical_updater_is_active(monkeypatch, tmp_path: Path) -> None:
    install = tmp_path / "install"
    install.mkdir()
    local = tmp_path / "local"
    run_id = "run-" + "a" * 32
    updater = local / "Sky-Auto-Player" / "update-runs" / run_id / "Sky-Auto-Player-Updater.exe"
    updater.parent.mkdir(parents=True)
    updater.write_bytes(b"updater")
    state_path = _write_active(local, install, run_id)
    monkeypatch.setenv("LOCALAPPDATA", str(local))
    monkeypatch.setattr(
        process_state,
        "query_process_image",
        lambda _pid: process_state.ProcessImageState(True, updater),
    )

    active = update_runtime.active_update_for_install(install)

    assert active is not None
    assert active.target_version == "3.4.5"
    assert state_path.exists()


def test_dead_or_noncanonical_updater_state_is_removed(monkeypatch, tmp_path: Path) -> None:
    install = tmp_path / "install"
    install.mkdir()
    local = tmp_path / "local"
    state_path = _write_active(local, install, "run-" + "b" * 32)
    monkeypatch.setenv("LOCALAPPDATA", str(local))
    monkeypatch.setattr(
        process_state,
        "query_process_image",
        lambda _pid: process_state.ProcessImageState(False, None),
    )

    assert update_runtime.active_update_for_install(install) is None
    assert not state_path.exists()
