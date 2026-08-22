"""Bounded LocalAppData persistence for pending release metadata."""

from __future__ import annotations

import json
import os
import secrets
from dataclasses import dataclass
from pathlib import Path

from sky_music.domain.update_checker import UpdateInfo, parse_version

APP_NAME = "Sky-Auto-Player"
_MAX_CACHE_BYTES = 128 * 1024
_MAX_NOTES_BYTES = 64 * 1024
_VERSION_MAX = 64
_TRUNCATION_SUFFIX = "\n\n[Release notes truncated; see GitHub Releases for the complete notes.]"


@dataclass(frozen=True, slots=True)
class PendingReleaseNotice:
    latest_version: str
    release_notes: str
    published_at: str


def _cache_path() -> Path | None:
    local_app_data = os.environ.get("LOCALAPPDATA", "")
    if not local_app_data:
        return None
    root = Path(local_app_data).resolve()
    if not root.is_absolute():
        return None
    return root / APP_NAME / "update-state" / "pending-release.json"


def _bounded_notes(notes: str) -> str:
    if len(notes.encode("utf-8")) <= _MAX_NOTES_BYTES:
        return notes
    suffix_bytes = _TRUNCATION_SUFFIX.encode("utf-8")
    limit = max(0, _MAX_NOTES_BYTES - len(suffix_bytes))
    prefix = notes.encode("utf-8")[:limit].decode("utf-8", errors="ignore")
    return prefix + _TRUNCATION_SUFFIX


def save_pending_release(update: UpdateInfo) -> None:
    path = _cache_path()
    if path is None:
        return
    if (
        not isinstance(update.latest_version, str)
        or len(update.latest_version) > _VERSION_MAX
        or parse_version(update.latest_version) is None
        or not isinstance(update.release_notes, str)
        or not isinstance(update.published_at, str)
    ):
        return
    payload = {
        "schema_version": 1,
        "latest_version": update.latest_version,
        "release_notes": _bounded_notes(update.release_notes),
        "published_at": update.published_at[:512],
    }
    encoded = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(encoded) > _MAX_CACHE_BYTES:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}-{secrets.token_hex(8)}.tmp")
    try:
        with temporary.open("xb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
        except OSError:
            pass


def load_pending_release() -> PendingReleaseNotice | None:
    path = _cache_path()
    if path is None or not path.is_file():
        return None
    try:
        if path.stat().st_size > _MAX_CACHE_BYTES:
            return None
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        return None
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        return None
    version = data.get("latest_version")
    notes = data.get("release_notes")
    published = data.get("published_at")
    if (
        not isinstance(version, str)
        or len(version) > _VERSION_MAX
        or parse_version(version) is None
        or not isinstance(notes, str)
        or len(notes.encode("utf-8")) > _MAX_NOTES_BYTES
        or "\x00" in notes
        or not isinstance(published, str)
        or len(published) > 512
        or "\x00" in published
    ):
        return None
    return PendingReleaseNotice(version, notes, published)


def clear_pending_release(version: str | None = None) -> None:
    path = _cache_path()
    if path is None or not path.is_file():
        return
    if version is not None:
        cached = load_pending_release()
        if cached is None or cached.latest_version != version:
            return
    try:
        path.unlink()
    except FileNotFoundError:
        pass
    except OSError:
        pass
