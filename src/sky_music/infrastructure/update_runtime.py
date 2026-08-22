"""Read native updater state and consume its bounded terminal result file."""

from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass
from pathlib import Path

from sky_music.domain.update_checker import parse_version

APP_NAME = "Sky-Auto-Player"
_MAX_RESULT_BYTES = 32 * 1024
_MAX_ACTIVE_BYTES = 8 * 1024
_VERSION_FIELD = re.compile(r"^[0-9A-Za-z][0-9A-Za-z.!+_-]{0,63}$")
_INSTALL_ID = re.compile(r"^[0-9a-f]{64}$")
_RUN_NAME = re.compile(r"^run-[0-9a-f]{32}$")
_ALLOWED_STATUSES = {"success", "rolled_back", "failure", "dry_run"}
_INVALID = object()
_ALLOWED_PHASES = {
    "Starting",
    "WaitingForParent",
    "FetchingRelease",
    "VerifyingRelease",
    "Extracting",
    "VerifyingStaging",
    "Preflight",
    "BackingUp",
    "Installing",
    "VerifyingInstall",
    "Committing",
    "CleaningUp",
    "Restarting",
    "Completed",
    "Failed",
    "RolledBack",
}


@dataclass(frozen=True, slots=True)
class UpdateRuntimeWarning:
    code: str
    message: str
    phase: str = ""
    operation: str = ""
    path: str = ""
    os_error: int | None = None


@dataclass(frozen=True, slots=True)
class UpdateRuntimeResult:
    status: str
    from_version: str
    target_version: str
    timestamp_utc: str
    error_code: str = ""
    message: str = ""
    phase: str = ""
    operation: str = ""
    path: str = ""
    os_error: int | None = None
    warnings: tuple[UpdateRuntimeWarning, ...] = ()
    cleanup_pending: bool = False


@dataclass(frozen=True, slots=True)
class ActiveUpdateState:
    install_id: str
    run_id: str
    updater_pid: int
    target_version: str
    phase: str
    started_at_utc: str
    updated_at_utc: str


def _result_path() -> Path | None:
    local_app_data = os.environ.get("LOCALAPPDATA", "")
    if not local_app_data:
        return None
    root = Path(local_app_data).resolve()
    if not root.is_absolute():
        return None
    return root / APP_NAME / "update-state" / "last-result.json"


def _active_path() -> Path | None:
    local_app_data = os.environ.get("LOCALAPPDATA", "")
    if not local_app_data:
        return None
    root = Path(local_app_data).resolve()
    if not root.is_absolute():
        return None
    return root / APP_NAME / "update-state" / "active-update.json"


def _text(data: dict[str, object], name: str, *, required: bool = True) -> str | None:
    value = data.get(name)
    if value is None and not required:
        return ""
    if not isinstance(value, str) or len(value) > 512 or "\x00" in value:
        return None
    return value


def _optional_text(data: dict[str, object], name: str) -> str | None:
    value = data.get(name, "")
    if not isinstance(value, str) or len(value) > 512 or "\x00" in value:
        return None
    return value


def _optional_os_error(data: dict[str, object], name: str) -> int | None | object:
    if name not in data or data[name] is None:
        return None
    value = data[name]
    if type(value) is not int or not 0 <= value <= 0xFFFFFFFF:
        return _INVALID
    return value


def _parse_warning(value: object) -> UpdateRuntimeWarning | None:
    if not isinstance(value, dict):
        return None
    code = _text(value, "code")
    message = _text(value, "message")
    phase = _optional_text(value, "phase")
    operation = _optional_text(value, "operation")
    path = _optional_text(value, "path")
    os_error = _optional_os_error(value, "os_error")
    if (
        code is None
        or message is None
        or phase is None
        or operation is None
        or path is None
        or (os_error is not None and type(os_error) is not int)
    ):
        return None
    return UpdateRuntimeWarning(
        code=code,
        message=message,
        phase=phase,
        operation=operation,
        path=path,
        os_error=os_error,
    )


def _parse_result(data: object) -> UpdateRuntimeResult | None:
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        return None
    status = data.get("status")
    if not isinstance(status, str) or status not in _ALLOWED_STATUSES:
        return None
    from_version = _text(data, "from_version")
    target_version = _text(data, "target_version")
    timestamp = _text(data, "timestamp_utc")
    error_code = _text(data, "error_code", required=False)
    message = _text(data, "message", required=False)
    if (
        from_version is None
        or target_version is None
        or timestamp is None
        or error_code is None
        or message is None
    ):
        return None
    if not _VERSION_FIELD.fullmatch(from_version) or not _VERSION_FIELD.fullmatch(target_version):
        return None
    if parse_version(from_version) is None or parse_version(target_version) is None:
        return None
    if not timestamp.endswith("Z") or len(timestamp) > 64:
        return None
    phase = _optional_text(data, "phase")
    operation = _optional_text(data, "operation")
    path = _optional_text(data, "path")
    os_error = _optional_os_error(data, "os_error")
    warnings_raw = data.get("warnings", [])
    if not isinstance(warnings_raw, list) or len(warnings_raw) > 8:
        return None
    warnings: list[UpdateRuntimeWarning] = []
    for warning_raw in warnings_raw:
        warning = _parse_warning(warning_raw)
        if warning is None:
            return None
        warnings.append(warning)
    cleanup_pending = data.get("cleanup_pending", False)
    if (
        phase is None
        or operation is None
        or path is None
        or (os_error is not None and type(os_error) is not int)
        or type(cleanup_pending) is not bool
    ):
        return None
    return UpdateRuntimeResult(
        status=status,
        from_version=from_version,
        target_version=target_version,
        timestamp_utc=timestamp,
        error_code=error_code,
        message=message,
        phase=phase,
        operation=operation,
        path=path,
        os_error=os_error,
        warnings=tuple(warnings),
        cleanup_pending=cleanup_pending,
    )


def _canonical_install_identity(install_root: Path) -> str | None:
    try:
        canonical = str(install_root.resolve(strict=True)).replace("/", "\\")
    except OSError:
        return None
    if os.name == "nt" and not canonical.startswith("\\\\?\\"):
        if canonical.startswith("\\\\"):
            canonical = "\\\\?\\UNC\\" + canonical[2:]
        else:
            canonical = "\\\\?\\" + canonical
    return canonical.lower()


def _install_id(install_root: Path) -> str | None:
    canonical = _canonical_install_identity(install_root)
    if canonical is None:
        return None
    import hashlib

    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def _parse_active_state(data: object) -> ActiveUpdateState | None:
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        return None
    install_id = data.get("install_id")
    run_id = data.get("run_id")
    updater_pid = data.get("updater_pid")
    target_version = data.get("target_version")
    phase = data.get("phase")
    started = data.get("started_at_utc")
    updated = data.get("updated_at_utc")
    if (
        not isinstance(install_id, str)
        or _INSTALL_ID.fullmatch(install_id) is None
        or not isinstance(run_id, str)
        or _RUN_NAME.fullmatch(run_id) is None
        or type(updater_pid) is not int
        or updater_pid <= 0
        or not isinstance(target_version, str)
        or _VERSION_FIELD.fullmatch(target_version) is None
        or parse_version(target_version) is None
        or not isinstance(phase, str)
        or phase not in _ALLOWED_PHASES
        or not isinstance(started, str)
        or not isinstance(updated, str)
        or len(started) > 64
        or len(updated) > 64
        or "\x00" in started
        or "\x00" in updated
        or not started.endswith("Z")
        or not updated.endswith("Z")
    ):
        return None
    return ActiveUpdateState(
        install_id=install_id,
        run_id=run_id,
        updater_pid=updater_pid,
        target_version=target_version,
        phase=phase,
        started_at_utc=started,
        updated_at_utc=updated,
    )


def _remove_active_state(path: Path) -> None:
    try:
        path.unlink()
    except FileNotFoundError:
        pass
    except OSError:
        pass


def active_update_for_install(install_root: Path) -> ActiveUpdateState | None:
    """Return a live, canonical updater state owned by ``install_root``.

    Invalid or stale state is removed only from the fixed active-state path.
    Process identity validation is delegated to the Win32 platform seam.
    """

    path = _active_path()
    install_id = _install_id(install_root)
    if path is None or install_id is None or not path.is_file():
        return None
    try:
        if path.stat().st_size > _MAX_ACTIVE_BYTES:
            _remove_active_state(path)
            return None
        state = _parse_active_state(json.loads(path.read_text(encoding="utf-8")))
    except (OSError, UnicodeError, json.JSONDecodeError):
        _remove_active_state(path)
        return None
    if state is None:
        _remove_active_state(path)
        return None

    from sky_music.platform.win32.process_state import query_process_image

    process = query_process_image(state.updater_pid)
    if not process.alive or process.image_path is None:
        _remove_active_state(path)
        return None
    if state.install_id != install_id:
        return None
    try:
        image = process.image_path.resolve(strict=True)
        runs = (path.parent.parent / "update-runs").resolve(strict=True)
        run_dir = image.parent.resolve(strict=True)
        if (
            image.name.casefold() != "sky-auto-player-updater.exe"
            or _RUN_NAME.fullmatch(run_dir.name) is None
            or run_dir.parent != runs
            or run_dir.name != state.run_id
        ):
            _remove_active_state(path)
            return None
    except OSError:
        _remove_active_state(path)
        return None
    return state


def consume_last_result() -> UpdateRuntimeResult | None:
    """Read one valid result and move it aside so it is not shown repeatedly."""

    path = _result_path()
    if path is None or not path.is_file():
        return None
    try:
        if path.stat().st_size > _MAX_RESULT_BYTES:
            return None
        result = _parse_result(json.loads(path.read_text(encoding="utf-8")))
        if result is None:
            return None
        consumed = path.with_name("last-result.json.consumed")
        os.replace(path, consumed)
        return result
    except (OSError, UnicodeError, json.JSONDecodeError):
        return None
