from __future__ import annotations

import hashlib
import json
import sqlite3
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from threading import RLock
from typing import Any, Literal

from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.domain.song_repository import get_shared_song_repository
from sky_music.config import AppConfig


@dataclass(frozen=True, slots=True)
class SongUiMetadata:
    path: Path
    name: str
    duration_seconds: float
    note_count: int
    max_polyphony: int
    min_note_gap_ms: float
    min_same_key_gap_ms: float
    risk: Literal["low", "medium", "high", "error"]
    recommended_profile: str
    recommended_tempo_scale: float
    warnings: tuple[str, ...]
    average_notes_per_second: float = 0.0
    peak_notes_per_second_1s: float = 0.0
    impossible_repeats: int = 0
    max_chord_size: int = 0
    chords_count: int = 0
    timing_stress_rate: float = 0.0


_metadata_cache: dict[tuple[Any, ...], SongUiMetadata] = {}
_persistent_cache: dict[str, SongUiMetadata] = {}
_persistent_loaded = False
_cache_lock = RLock()
_song_repository = get_shared_song_repository()

PERSISTENT_CACHE_SCHEMA_VERSION = 2
PERSISTENT_CACHE_PATH = Path(".cache") / "sky_music" / "picker_metadata.sqlite3"
_PERSISTENT_POLICY_ATTRS: tuple[str, ...] = (
    "hold_us",
    "min_hold_us",
    "release_gap_us",
    "repeat_release_gap_us",
    "input_lead_us",
    "chord_merge_window_us",
    "spin_threshold_us",
    "focus_restore_grace_us",
    "same_key_conflict_policy",
    "frame_us",
    "fps",
    "frame_align",
)


def _metadata_to_payload(meta: SongUiMetadata) -> dict[str, Any]:
    payload = asdict(meta)
    payload["path"] = str(meta.path)
    payload["warnings"] = list(meta.warnings)
    return payload


def _metadata_from_payload(payload: dict[str, Any]) -> SongUiMetadata | None:
    try:
        risk = str(payload.get("risk", "error")).lower()
        if risk not in {"low", "medium", "high", "error"}:
            risk = "error"
        return SongUiMetadata(
            path=Path(str(payload.get("path", ""))),
            name=str(payload.get("name", "")),
            duration_seconds=float(payload.get("duration_seconds", 0.0)),
            note_count=int(payload.get("note_count", 0)),
            max_polyphony=int(payload.get("max_polyphony", 0)),
            min_note_gap_ms=float(payload.get("min_note_gap_ms", 0.0)),
            min_same_key_gap_ms=float(payload.get("min_same_key_gap_ms", 0.0)),
            risk=risk,  # type: ignore[arg-type]
            recommended_profile=str(payload.get("recommended_profile", "balanced")),
            recommended_tempo_scale=float(payload.get("recommended_tempo_scale", 1.0)),
            warnings=tuple(str(item) for item in payload.get("warnings", ())),
            average_notes_per_second=float(payload.get("average_notes_per_second", 0.0)),
            peak_notes_per_second_1s=float(payload.get("peak_notes_per_second_1s", 0.0)),
            impossible_repeats=int(payload.get("impossible_repeats", 0)),
            max_chord_size=int(payload.get("max_chord_size", 0)),
            chords_count=int(payload.get("chords_count", 0)),
            timing_stress_rate=float(payload.get("timing_stress_rate", 0.0)),
        )
    except Exception:
        return None


def _stable_file_identity(song_path: Path) -> dict[str, Any]:
    stat = song_path.stat()
    try:
        path_key = str(song_path.resolve())
    except Exception:
        path_key = str(song_path)
    return {
        "path": path_key,
        "mtime_ns": int(stat.st_mtime_ns),
        "size": int(stat.st_size),
    }


def _effective_policy_signature(session: PlaybackSessionContext, cfg: AppConfig | None) -> dict[str, Any]:
    policy = session.resolve_effective_policy(cfg)
    return {
        attr: getattr(policy, attr)
        for attr in _PERSISTENT_POLICY_ATTRS
        if hasattr(policy, attr)
    }


def _persistent_cache_key(
    song_path: Path,
    session: PlaybackSessionContext,
    cfg: AppConfig | None,
) -> str | None:
    try:
        payload = {
            "schema": PERSISTENT_CACHE_SCHEMA_VERSION,
            "file": _stable_file_identity(song_path),
            "session": {
                "profile_name": session.profile_name,
                "tempo_scale": float(session.tempo_scale),
                "fps": session.fps,
                "scan_code_mode": session.scan_code_mode,
                "same_key_conflict_policy": session.same_key_conflict_policy,
                "frame_align": session.resolved_frame_align(cfg),
                "policy_overrides": list(session.policy_overrides),
            },
            "effective_policy": _effective_policy_signature(session, cfg),
            "platform": sys.platform,
        }
        encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"), default=str).encode("utf-8")
        return hashlib.sha256(encoded).hexdigest()
    except Exception:
        return None


def _connect_persistent_cache() -> sqlite3.Connection:
    PERSISTENT_CACHE_PATH.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(PERSISTENT_CACHE_PATH, timeout=0.2)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS picker_metadata (
            cache_key TEXT PRIMARY KEY,
            payload TEXT NOT NULL,
            updated_at REAL NOT NULL
        )
        """
    )
    return conn


def warm_persistent_metadata_cache(limit: int = 6000) -> int:
    """Load persistent picker metadata into memory.

    This is safe to run from the picker metadata worker. `peek_cached_song_ui_metadata`
    intentionally does not do disk I/O, so the UI thread stays responsive.
    """
    global _persistent_loaded
    with _cache_lock:
        if _persistent_loaded:
            return len(_persistent_cache)

    loaded: dict[str, SongUiMetadata] = {}
    try:
        with _connect_persistent_cache() as conn:
            rows = conn.execute(
                """
                SELECT cache_key, payload
                FROM picker_metadata
                ORDER BY updated_at DESC
                LIMIT ?
                """,
                (int(limit),),
            ).fetchall()
    except Exception:
        rows = []

    for key, payload_text in rows:
        try:
            payload = json.loads(str(payload_text))
            meta = _metadata_from_payload(payload)
            if meta is not None:
                loaded[str(key)] = meta
        except Exception:
            continue

    with _cache_lock:
        _persistent_cache.update(loaded)
        _persistent_loaded = True
        return len(_persistent_cache)


def hydrate_persistent_metadata_for_paths(
    paths: list[Path] | tuple[Path, ...],
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> int:
    """Load disk-cached metadata for specific songs into memory.

    This is meant for a tiny visible-window batch. It avoids waiting for the
    full persistent cache warmup, and it should be called only from a background
    cache worker, not from prompt_toolkit render paths.
    """
    if not paths:
        return 0

    session = session or PlaybackSessionContext.balanced()
    key_to_path: dict[str, Path] = {}
    for path in paths:
        key = _persistent_cache_key(path, session, cfg)
        if key is not None:
            key_to_path[key] = path

    if not key_to_path:
        return 0

    with _cache_lock:
        missing_keys = [key for key in key_to_path if key not in _persistent_cache]
    if not missing_keys:
        return 0

    placeholders = ",".join("?" for _ in missing_keys)
    try:
        with _connect_persistent_cache() as conn:
            rows = conn.execute(
                f"SELECT cache_key, payload FROM picker_metadata WHERE cache_key IN ({placeholders})",
                tuple(missing_keys),
            ).fetchall()
    except Exception:
        return 0

    loaded: dict[str, SongUiMetadata] = {}
    for key, payload_text in rows:
        try:
            payload = json.loads(str(payload_text))
            meta = _metadata_from_payload(payload)
            if meta is not None:
                loaded[str(key)] = meta
        except Exception:
            continue

    if not loaded:
        return 0

    # Also seed the normal RAM cache, so future peeks do not need to compute the
    # persistent key again once the file identity is available.
    with _cache_lock:
        _persistent_cache.update(loaded)
        for key, meta in loaded.items():
            try:
                song_file_key = _song_repository.cache_key(key_to_path[key])
                ram_key = session.metadata_cache_key(song_file_key, cfg)
                _metadata_cache[ram_key] = meta
            except Exception:
                continue

    return len(loaded)


def persistent_metadata_cache_stats() -> dict[str, Any]:
    with _cache_lock:
        return {
            "loaded": _persistent_loaded,
            "entries": len(_persistent_cache),
            "path": str(PERSISTENT_CACHE_PATH),
        }


def session_to_worker_payload(session: PlaybackSessionContext) -> dict[str, Any]:
    """Return a small, picklable session payload for process workers."""
    return {
        "profile_name": session.profile_name,
        "tempo_scale": float(session.tempo_scale),
        "fps": session.fps,
        "scan_code_mode": session.scan_code_mode,
        "same_key_conflict_policy": session.same_key_conflict_policy,
        "frame_align": session.frame_align,
        "policy_overrides": list(session.policy_overrides),
    }


def _session_from_worker_payload(payload: dict[str, Any]) -> PlaybackSessionContext:
    raw_overrides = payload.get("policy_overrides", ())
    overrides: list[tuple[str, Any]] = []
    if isinstance(raw_overrides, list | tuple):
        for item in raw_overrides:
            if isinstance(item, list | tuple) and len(item) == 2:
                overrides.append((str(item[0]), item[1]))

    conflict_policy = str(payload.get("same_key_conflict_policy", "degraded"))
    if conflict_policy not in {"degraded", "strict"}:
        conflict_policy = "degraded"

    frame_align_raw = payload.get("frame_align")
    frame_align = str(frame_align_raw) if frame_align_raw in {"none", "down_only"} else None

    return PlaybackSessionContext(
        profile_name=str(payload.get("profile_name", "balanced")),
        tempo_scale=float(payload.get("tempo_scale", 1.0)),
        fps=payload.get("fps"),
        scan_code_mode=str(payload.get("scan_code_mode", "physical")),
        same_key_conflict_policy=conflict_policy,  # type: ignore[arg-type]
        frame_align=frame_align,  # type: ignore[arg-type]
        policy_overrides=tuple(overrides),
    )


def compute_song_ui_metadata_payloads(
    path_values: list[str] | tuple[str, ...],
    session_payload: dict[str, Any],
    cfg: AppConfig | None = None,
) -> list[dict[str, Any]]:
    """Compute song UI metadata in a worker process or worker thread.

    This function is intentionally module-level so it is picklable by
    ProcessPoolExecutor on Windows. It returns plain JSON-like payloads instead
    of touching the parent process' RAM cache.
    """
    session = _session_from_worker_payload(session_payload)
    payloads: list[dict[str, Any]] = []
    for value in path_values:
        meta = get_song_ui_metadata(Path(value), session, cfg)
        payloads.append(_metadata_to_payload(meta))
    return payloads


def store_computed_song_ui_metadata_payloads(
    payloads: list[dict[str, Any]] | tuple[dict[str, Any], ...],
    session: PlaybackSessionContext,
    cfg: AppConfig | None = None,
) -> int:
    """Store metadata returned by a worker into parent RAM + SQLite caches."""
    stored = 0
    for payload in payloads:
        meta = _metadata_from_payload(payload)
        if meta is None:
            continue
        try:
            song_file_key = _song_repository.cache_key(meta.path)
            ram_key = session.metadata_cache_key(song_file_key, cfg)
            with _cache_lock:
                _metadata_cache[ram_key] = meta
            _store_persistent_metadata(meta.path, session, cfg, meta)
            stored += 1
        except Exception:
            continue
    return stored


def _store_persistent_metadata(
    song_path: Path,
    session: PlaybackSessionContext,
    cfg: AppConfig | None,
    meta: SongUiMetadata,
) -> None:
    key = _persistent_cache_key(song_path, session, cfg)
    if key is None:
        return

    payload = json.dumps(_metadata_to_payload(meta), ensure_ascii=False, separators=(",", ":"))
    with _cache_lock:
        _persistent_cache[key] = meta

    try:
        with _connect_persistent_cache() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO picker_metadata(cache_key, payload, updated_at)
                VALUES (?, ?, ?)
                """,
                (key, payload, time.time()),
            )
            # Opportunistic pruning keeps the cache bounded without blocking normal reads.
            conn.execute(
                """
                DELETE FROM picker_metadata
                WHERE cache_key NOT IN (
                    SELECT cache_key FROM picker_metadata
                    ORDER BY updated_at DESC
                    LIMIT 6000
                )
                """
            )
    except Exception:
        return


def _peek_persistent_metadata(
    song_path: Path,
    session: PlaybackSessionContext,
    cfg: AppConfig | None,
) -> SongUiMetadata | None:
    # Never load from disk here; this function is used in render paths.
    # It may still return entries hydrated by a targeted background cache read
    # before the full persistent cache warmup has completed.
    key = _persistent_cache_key(song_path, session, cfg)
    if key is None:
        return None
    with _cache_lock:
        return _persistent_cache.get(key)


def get_song_ui_metadata(
    song_path: Path,
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> SongUiMetadata:
    session = session or PlaybackSessionContext.balanced()
    try:
        from sky_music.domain.scheduler import build_key_actions
        from sky_music.domain.analyzer import analyze_schedule

        # build_key_actions builds the (now unified) DefaultNoteResolver when resolver is None.
        resolver = None

        song = _song_repository.load(song_path)
        policy = session.resolve_effective_policy(cfg)
        sched = build_key_actions(
            song,
            policy=policy,
            scan_code_mode=session.scan_code_mode,
            resolver=resolver,
            tempo_scale=session.tempo_scale,
        )
        report = analyze_schedule(sched, raw_notes=song.notes)

        min_note_gap = (report.min_any_note_gap_us / 1000.0) if report.min_any_note_gap_us is not None else 0.0
        min_repeat_gap = (report.min_same_key_gap_us / 1000.0) if report.min_same_key_gap_us is not None else 0.0

        return SongUiMetadata(
            path=song_path,
            name=song.name or song_path.stem,
            duration_seconds=sched.source_duration_us / 1_000_000,
            note_count=sched.note_count,
            max_polyphony=report.max_polyphony,
            min_note_gap_ms=min_note_gap,
            min_same_key_gap_ms=min_repeat_gap,
            risk=report.severity,
            recommended_profile=report.suggested_profile,
            recommended_tempo_scale=report.suggested_tempo_scale,
            warnings=report.recommendations,
            average_notes_per_second=report.average_notes_per_second,
            peak_notes_per_second_1s=report.peak_notes_per_second_1s,
            impossible_repeats=report.impossible_repeats,
            max_chord_size=report.max_chord_size,
            chords_count=report.chords_count,
            timing_stress_rate=report.timing_stress_rate,
        )
    except Exception as e:
        return SongUiMetadata(
            path=song_path,
            name=song_path.stem,
            duration_seconds=0.0,
            note_count=0,
            max_polyphony=0,
            min_note_gap_ms=0.0,
            min_same_key_gap_ms=0.0,
            risk="error",
            recommended_profile="unplayable",
            recommended_tempo_scale=1.0,
            warnings=(f"Failed to analyze song: {e}",),
            average_notes_per_second=0.0,
            peak_notes_per_second_1s=0.0,
            impossible_repeats=0,
            max_chord_size=0,
            chords_count=0,
            timing_stress_rate=0.0,
        )


def get_cached_song_ui_metadata(
    song_path: Path,
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> SongUiMetadata:
    session = session or PlaybackSessionContext.balanced()
    try:
        song_file_key = _song_repository.cache_key(song_path)
    except Exception:
        return get_song_ui_metadata(song_path, session, cfg)

    ram_key = session.metadata_cache_key(song_file_key, cfg)
    with _cache_lock:
        cached = _metadata_cache.get(ram_key)
    if cached is not None:
        return cached

    persistent = _peek_persistent_metadata(song_path, session, cfg)
    if persistent is not None:
        with _cache_lock:
            _metadata_cache[ram_key] = persistent
        return persistent

    meta = get_song_ui_metadata(song_path, session, cfg)
    with _cache_lock:
        _metadata_cache[ram_key] = meta
    _store_persistent_metadata(song_path, session, cfg, meta)
    return meta


def peek_cached_song_ui_metadata(
    song_path: Path,
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> SongUiMetadata | None:
    """Return cached metadata without parsing/analyzing the song.

    This function is safe in render paths: it checks RAM and already-warmed
    persistent cache only, never disk-loads synchronously.
    """
    session = session or PlaybackSessionContext.balanced()
    try:
        song_file_key = _song_repository.cache_key(song_path)
    except Exception:
        return None

    ram_key = session.metadata_cache_key(song_file_key, cfg)
    with _cache_lock:
        cached = _metadata_cache.get(ram_key)
    if cached is not None:
        return cached

    persistent = _peek_persistent_metadata(song_path, session, cfg)
    if persistent is None:
        return None

    with _cache_lock:
        _metadata_cache[ram_key] = persistent
    return persistent


def clear_metadata_cache(*, clear_persistent: bool = False) -> None:
    global _persistent_loaded
    with _cache_lock:
        _metadata_cache.clear()
        _song_repository.clear()
        if clear_persistent:
            _persistent_cache.clear()
            _persistent_loaded = False
    if clear_persistent:
        try:
            PERSISTENT_CACHE_PATH.unlink(missing_ok=True)
        except Exception:
            pass


def _get_song_recommendation(
    song_path: Path,
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> tuple[str, float]:
    meta = get_cached_song_ui_metadata(song_path, session, cfg)
    return meta.recommended_profile, meta.recommended_tempo_scale
