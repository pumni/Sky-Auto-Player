"""Stage and launch the bundled native updater.

The Python process never downloads, replaces, or deletes the installed
application. It only copies its bundled updater to a per-run directory and
starts that copy with a validated, explicit argument set.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path

from sky_music.domain.update_checker import is_newer, is_prerelease, parse_version

APP_NAME = "Sky-Auto-Player"
UPDATER_NAME = "Sky-Auto-Player-Updater.exe"
_RUN_NAME = re.compile(r"^run-[0-9a-f]{32}$")


class UpdateLaunchError(RuntimeError):
    """The updater could not be staged or launched safely."""


@dataclass(frozen=True, slots=True)
class UpdateLaunchRequest:
    install_root: Path
    current_version: str
    target_version: str
    channel: str
    restart: bool = True


def _local_update_root() -> Path:
    local_app_data = os.environ.get("LOCALAPPDATA", "")
    if not local_app_data:
        raise UpdateLaunchError("LOCALAPPDATA is unavailable")
    root = Path(local_app_data).resolve() / APP_NAME
    if not root.is_absolute():
        raise UpdateLaunchError("update root must be absolute")
    return root


def _validate_request(request: UpdateLaunchRequest) -> None:
    if not request.install_root.is_absolute() or not request.install_root.is_dir():
        raise UpdateLaunchError("install root must be an existing absolute directory")
    if request.channel not in {"stable", "beta"}:
        raise UpdateLaunchError("channel must be stable or beta")
    if parse_version(request.current_version) is None or parse_version(request.target_version) is None:
        raise UpdateLaunchError("current and target versions must be valid PEP 440 versions")
    if not is_newer(request.target_version, request.current_version):
        raise UpdateLaunchError("target version must be newer than the running version")
    if request.channel == "stable" and is_prerelease(request.target_version):
        raise UpdateLaunchError("stable channel cannot install a prerelease")


def _bundled_updater(install_root: Path) -> Path:
    updater = install_root / UPDATER_NAME
    try:
        if updater.resolve().parent != install_root.resolve():
            raise UpdateLaunchError("bundled updater escaped the install root")
    except OSError as exc:
        raise UpdateLaunchError("bundled updater path could not be resolved") from exc
    if updater.name != UPDATER_NAME or not updater.is_file():
        raise UpdateLaunchError(f"bundled updater is missing: {UPDATER_NAME}")
    return updater


def _sha256(path: Path) -> str:
    import hashlib

    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def cleanup_stale_update_runs(*, max_age_s: int = 7 * 24 * 60 * 60) -> int:
    """Remove only old directories created by this launcher.

    Unknown entries and recent runs are left untouched. This is intentionally
    conservative because the updater may still be finishing after an app
    restart.
    """

    if type(max_age_s) is not int or max_age_s < 60:
        raise ValueError("max_age_s must be an integer of at least 60 seconds")
    runs = _local_update_root() / "update-runs"
    if not runs.is_dir():
        return 0
    now = time.time()
    removed = 0
    for candidate in runs.iterdir():
        if not candidate.is_dir() or _RUN_NAME.fullmatch(candidate.name) is None:
            continue
        try:
            age = now - candidate.stat().st_mtime
            if age < max_age_s:
                continue
            shutil.rmtree(candidate)
            removed += 1
        except OSError:
            continue
    return removed


def launch_update(request: UpdateLaunchRequest) -> Path:
    """Copy the bundled updater and launch it without a shell."""

    if sys.platform != "win32":
        raise UpdateLaunchError("native self-update is supported only on Windows")
    if not getattr(sys, "frozen", False):
        raise UpdateLaunchError("self-update requires a frozen release build")
    _validate_request(request)
    bundled = _bundled_updater(request.install_root)
    root = _local_update_root()
    runs = root / "update-runs"
    runs.mkdir(parents=True, exist_ok=True)
    run_dir = runs / f"run-{uuid.uuid4().hex}"
    run_dir.mkdir()
    staged_updater = run_dir / UPDATER_NAME
    temporary = run_dir / f"{UPDATER_NAME}.tmp"
    try:
        shutil.copy2(bundled, temporary)
        if _sha256(temporary) != _sha256(bundled):
            raise UpdateLaunchError("staged updater hash does not match bundled updater")
        os.replace(temporary, staged_updater)
        arguments = [
            str(staged_updater),
            "--install-root",
            str(request.install_root),
            "--parent-pid",
            str(os.getpid()),
            "--current-version",
            request.current_version,
            "--target-version",
            request.target_version,
            "--channel",
            request.channel,
        ]
        if request.restart:
            arguments.append("--restart")
        subprocess.Popen(
            arguments,
            cwd=str(run_dir),
            shell=False,
            close_fds=True,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
    except UpdateLaunchError:
        raise
    except (OSError, subprocess.SubprocessError) as exc:
        raise UpdateLaunchError(f"could not launch native updater: {exc}") from exc
    finally:
        temporary.unlink(missing_ok=True)
    return run_dir
