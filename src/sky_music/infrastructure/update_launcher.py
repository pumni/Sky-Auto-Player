"""Safely stage and launch the verified native updater."""

from __future__ import annotations

import hashlib
import json
import os
import re
import secrets
import shutil
import subprocess
import time
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from sky_music.domain.update_checker import is_newer, is_prerelease, parse_version

APP_NAME = "Sky-Auto-Player"
PRIMARY_EXE = "Sky-Auto-Player.exe"
UPDATER_NAME = "Sky-Auto-Player-Updater.exe"
_RUN_NAME = re.compile(r"^run-[0-9a-f]{32}$")
_SHA256 = re.compile(r"^[0-9a-fA-F]{64}$")
_MAX_MANIFEST_BYTES = 4 * 1024 * 1024
_MAX_HANDOFF_BYTES = 8 * 1024
HANDOFF_POLL_INTERVAL_S = 0.05
HANDOFF_TIMEOUT_S = 5.0
HANDOFF_TERMINATE_WAIT_S = 2.0
_HANDOFF_RUN_NAME = re.compile(r"^run-[0-9a-f]{32}$")


class UpdateLaunchError(RuntimeError):
    """The updater could not be staged or launched safely."""


@dataclass(frozen=True, slots=True)
class UpdateLaunchRequest:
    install_root: Path
    current_version: str
    target_version: str
    channel: str
    restart: bool = True


@dataclass(frozen=True, slots=True)
class UpdateLaunchResult:
    status: Literal["ready", "already_running"]
    staged_updater: Path
    run_root: Path
    updater_pid: int

    # Compatibility conveniences for callers that only used the old Path
    # return value. New code should use ``staged_updater`` explicitly.
    @property
    def name(self) -> str:
        return self.staged_updater.name

    def read_bytes(self) -> bytes:
        return self.staged_updater.read_bytes()


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


def _validate_manifest_path(path: object) -> str:
    if not isinstance(path, str) or not path or "\\" in path or ":" in path:
        raise UpdateLaunchError("installed manifest contains an unsafe path")
    parts = path.split("/")
    if (
        path.startswith("/")
        or any(part in {"", ".", ".."} for part in parts)
        or path == "MANIFEST.json"
    ):
        raise UpdateLaunchError("installed manifest contains an unsafe path")
    return path


def _read_installed_manifest(install_root: Path) -> dict[str, object]:
    manifest_path = install_root / "MANIFEST.json"
    try:
        if manifest_path.is_symlink() or not manifest_path.is_file():
            raise UpdateLaunchError("installed manifest is missing")
        if manifest_path.stat().st_size > _MAX_MANIFEST_BYTES:
            raise UpdateLaunchError("installed manifest exceeds the size limit")
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
    except UpdateLaunchError:
        raise
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise UpdateLaunchError("installed manifest is not valid UTF-8 JSON") from exc
    if not isinstance(data, dict):
        raise UpdateLaunchError("installed manifest must be a JSON object")
    if data.get("schema_version") != 2 or data.get("app") != APP_NAME:
        raise UpdateLaunchError("installed manifest schema or app is invalid")
    if not isinstance(data.get("version"), str) or parse_version(data["version"]) is None:
        raise UpdateLaunchError("installed manifest version is invalid")
    if data.get("executable") != PRIMARY_EXE:
        raise UpdateLaunchError("installed manifest executable is invalid")
    files = data.get("files")
    if not isinstance(files, list):
        raise UpdateLaunchError("installed manifest files must be a list")
    seen: set[str] = set()
    updater_entries = 0
    for entry in files:
        if not isinstance(entry, dict):
            raise UpdateLaunchError("installed manifest contains a non-object file entry")
        path = _validate_manifest_path(entry.get("path"))
        folded = path.casefold()
        if folded in seen:
            raise UpdateLaunchError("installed manifest contains duplicate paths")
        seen.add(folded)
        size = entry.get("size")
        digest = entry.get("sha256")
        if type(size) is not int or size < 0 or not isinstance(digest, str) or not _SHA256.fullmatch(digest):
            raise UpdateLaunchError(f"installed manifest entry is invalid: {path}")
        if path == UPDATER_NAME:
            updater_entries += 1
    if updater_entries != 1:
        raise UpdateLaunchError("installed manifest must contain exactly one native updater entry")
    return data


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _verified_installed_updater(
    install_root: Path, manifest: dict[str, object]
) -> Path:
    files = manifest.get("files")
    if not isinstance(files, list):
        raise UpdateLaunchError("installed manifest files must be a list")
    entry = next(
        (
            item
            for item in files
            if isinstance(item, dict) and item.get("path") == UPDATER_NAME
        ),
        None,
    )
    if not isinstance(entry, dict):
        raise UpdateLaunchError("installed manifest is missing the native updater entry")
    updater = install_root / UPDATER_NAME
    if updater.is_symlink() or not updater.is_file():
        raise UpdateLaunchError(f"bundled updater is missing: {UPDATER_NAME}")
    try:
        if updater.resolve().parent != install_root.resolve():
            raise UpdateLaunchError("bundled updater escaped the install root")
        size = updater.stat().st_size
        digest = _sha256(updater)
    except OSError as exc:
        raise UpdateLaunchError("bundled updater could not be read") from exc
    if size != entry.get("size") or digest.lower() != str(entry.get("sha256", "")).lower():
        raise UpdateLaunchError("installed native updater does not match MANIFEST.json")
    return updater


def _preflight_install_root_writable(install_root: Path) -> None:
    """Prove the portable install root accepts a small temporary write."""

    sentinel = install_root / f".sky-auto-player-update-write-{secrets.token_hex(16)}"
    created = False
    try:
        with sentinel.open("xb") as stream:
            created = True
            stream.write(b"Sky Auto Player update preflight\n")
            stream.flush()
            os.fsync(stream.fileno())
    except OSError as exc:
        raise UpdateLaunchError(f"install root is not writable: {exc}") from exc
    finally:
        if created:
            try:
                sentinel.unlink()
            except OSError as exc:
                raise UpdateLaunchError(
                    f"install root write preflight cleanup failed: {exc}"
                ) from exc


def _new_update_run() -> Path:
    update_root = _local_update_root()
    if update_root.exists() and update_root.is_symlink():
        raise UpdateLaunchError("update state root must not be a symlink")
    runs = update_root / "update-runs"
    if runs.exists() and runs.is_symlink():
        raise UpdateLaunchError("update run root must not be a symlink")
    try:
        update_root.mkdir(parents=True, exist_ok=True)
        runs.mkdir(parents=True, exist_ok=True)
        runs_root = runs.resolve()
        for _ in range(10):
            candidate = runs / f"run-{secrets.token_hex(16)}"
            candidate.mkdir()
            if candidate.resolve().parent != runs_root:
                shutil.rmtree(candidate, ignore_errors=True)
                raise UpdateLaunchError("update run escaped its allow-listed directory")
            return candidate
    except UpdateLaunchError:
        raise
    except OSError as exc:
        raise UpdateLaunchError("could not create native updater run directory") from exc
    raise UpdateLaunchError("could not allocate a unique native updater run directory")


def _parse_handoff(
    path: Path,
    *,
    run_root: Path,
    process_pid: int,
    target_version: str,
) -> tuple[str, str] | None:
    try:
        if path.stat().st_size > _MAX_HANDOFF_BYTES:
            return None
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        return None
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        return None
    state = data.get("state")
    run_id = data.get("run_id")
    updater_pid = data.get("updater_pid")
    handoff_target = data.get("target_version")
    error_code = data.get("error_code", "")
    message = data.get("message", "")
    if (
        state not in {"ready", "rejected"}
        or not isinstance(run_id, str)
        or _HANDOFF_RUN_NAME.fullmatch(run_id) is None
        or run_id != run_root.name
        or type(updater_pid) is not int
        or updater_pid != process_pid
        or not isinstance(handoff_target, str)
        or handoff_target != target_version
        or not isinstance(error_code, str)
        or not isinstance(message, str)
        or len(error_code) > 128
        or len(message) > 512
        or "\x00" in error_code
        or "\x00" in message
    ):
        return None
    return state, error_code


def _terminate_after_handshake_timeout(process: subprocess.Popen[bytes]) -> None:
    with suppress(OSError):
        process.terminate()
    try:
        process.wait(timeout=HANDOFF_TERMINATE_WAIT_S)
        return
    except (OSError, subprocess.TimeoutExpired):
        pass
    with suppress(OSError):
        process.kill()
    with suppress(OSError, subprocess.TimeoutExpired):
        process.wait(timeout=HANDOFF_TERMINATE_WAIT_S)


def _wait_for_handoff(
    process: subprocess.Popen[bytes],
    *,
    run_root: Path,
    target_version: str,
) -> tuple[str, str]:
    handoff = run_root / "handoff.json"
    deadline = time.monotonic() + HANDOFF_TIMEOUT_S
    while time.monotonic() < deadline:
        parsed = _parse_handoff(
            handoff,
            run_root=run_root,
            process_pid=process.pid,
            target_version=target_version,
        )
        if parsed is not None:
            state, error_code = parsed
            if state == "ready":
                return parsed
            if error_code == "UPDATE_ALREADY_RUNNING":
                return parsed
            raise UpdateLaunchError(
                f"native updater rejected startup [{error_code or 'UNKNOWN'}]"
            )
        if process.poll() is not None:
            raise UpdateLaunchError("native updater exited before the ready handshake")
        time.sleep(HANDOFF_POLL_INTERVAL_S)
    _terminate_after_handshake_timeout(process)
    raise UpdateLaunchError("native updater did not complete the ready handshake")


def cleanup_stale_update_runs(*, max_age_s: int = 7 * 24 * 60 * 60) -> int:
    """Remove only old directories created by this launcher."""

    if type(max_age_s) is not int or max_age_s < 60:
        raise ValueError("max_age_s must be an integer of at least 60 seconds")
    runs = _local_update_root() / "update-runs"
    if not runs.is_dir():
        return 0
    now = time.time()
    removed = 0
    for candidate in runs.iterdir():
        if (
            candidate.is_symlink()
            or not candidate.is_dir()
            or _RUN_NAME.fullmatch(candidate.name) is None
        ):
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


def launch_update(request: UpdateLaunchRequest) -> UpdateLaunchResult:
    """Copy the verified updater, spawn it, and wait for a durable ready handoff."""

    _validate_request(request)
    install_root = request.install_root.resolve()
    manifest = _read_installed_manifest(install_root)
    if manifest.get("version") != request.current_version:
        raise UpdateLaunchError("installed manifest version does not match the running app")
    updater = _verified_installed_updater(install_root, manifest)
    _preflight_install_root_writable(install_root)
    run_root = _new_update_run()
    staged = run_root / UPDATER_NAME
    try:
        shutil.copy2(updater, staged)
        files = manifest.get("files")
        if not isinstance(files, list):
            raise UpdateLaunchError("installed manifest files must be a list")
        entry = next(
            item
            for item in files
            if isinstance(item, dict) and item.get("path") == UPDATER_NAME
        )
        if staged.stat().st_size != entry["size"] or _sha256(staged).lower() != str(entry["sha256"]).lower():
            raise UpdateLaunchError("staged native updater does not match MANIFEST.json")
        arguments = [
            str(staged),
            "--install-root",
            str(install_root),
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
        process = subprocess.Popen(
            arguments,
            cwd=str(install_root),
            shell=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        state, _error_code = _wait_for_handoff(
            process,
            run_root=run_root,
            target_version=request.target_version,
        )
        if state == "rejected":
            return UpdateLaunchResult(
                status="already_running",
                staged_updater=staged,
                run_root=run_root,
                updater_pid=process.pid,
            )
    except UpdateLaunchError:
        shutil.rmtree(run_root, ignore_errors=True)
        raise
    except (OSError, StopIteration, KeyError, TypeError) as exc:
        shutil.rmtree(run_root, ignore_errors=True)
        raise UpdateLaunchError(f"could not launch native updater: {exc}") from exc
    return UpdateLaunchResult(
        status="ready",
        staged_updater=staged,
        run_root=run_root,
        updater_pid=process.pid,
    )
