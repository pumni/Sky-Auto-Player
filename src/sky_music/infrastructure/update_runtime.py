"""Read and consume the native updater's bounded startup result file."""

from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass
from pathlib import Path

from sky_music.domain.update_checker import parse_version

APP_NAME = "Sky-Auto-Player"
_MAX_RESULT_BYTES = 32 * 1024
_VERSION_FIELD = re.compile(r"^[0-9A-Za-z][0-9A-Za-z.!+_-]{0,63}$")
_ALLOWED_STATUSES = {"success", "rolled_back", "failure", "dry_run"}


@dataclass(frozen=True, slots=True)
class UpdateRuntimeResult:
    status: str
    from_version: str
    target_version: str
    timestamp_utc: str
    error_code: str = ""
    message: str = ""


def _result_path() -> Path | None:
    local_app_data = os.environ.get("LOCALAPPDATA", "")
    if not local_app_data:
        return None
    root = Path(local_app_data).resolve()
    if not root.is_absolute():
        return None
    return root / APP_NAME / "update-state" / "last-result.json"


def _text(data: dict[str, object], name: str, *, required: bool = True) -> str | None:
    value = data.get(name)
    if value is None and not required:
        return ""
    if not isinstance(value, str) or len(value) > 512 or "\x00" in value:
        return None
    return value


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
    return UpdateRuntimeResult(
        status=status,
        from_version=from_version,
        target_version=target_version,
        timestamp_utc=timestamp,
        error_code=error_code,
        message=message,
    )


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
