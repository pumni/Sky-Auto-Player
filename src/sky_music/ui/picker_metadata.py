from __future__ import annotations

import contextlib
import hashlib
import json
import sqlite3
import sys
import threading
import time
from collections import OrderedDict
from dataclasses import asdict, dataclass
from pathlib import Path
from threading import RLock
from typing import Any, Literal, TypeVar

from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.domain.song_repository import get_shared_song_repository

_K = TypeVar("_K")
_V = TypeVar("_V")

# LRU caps (see docs/2026-07_ram-memory-hygiene-plan.md §0 Q3 / §7).
_METADATA_CACHE_MAX = 2048
_RAW_CACHE_MAX = 5000
_PERSISTENT_CACHE_MAX = 3000
_PERSISTENT_CACHE_MAX_SQLITE = 6000
_PKEY_RAM_CACHE_MAX = 2000
_PATH_SESSION_RAM_CACHE_MAX = 5000


def _lru_set[K, V](cache: OrderedDict[K, V], key: K, value: V, *, maxsize: int) -> None:
    """Insert or update key with LRU eviction. Caller holds the cache lock."""
    if key in cache:
        cache.move_to_end(key)
    cache[key] = value
    while len(cache) > maxsize:
        cache.popitem(last=False)


def _lru_get[K, V](cache: OrderedDict[K, V], key: K) -> V | None:
    """Return value for key and promote to MRU; None if absent. Caller holds lock."""
    if key not in cache:
        return None
    cache.move_to_end(key)
    return cache[key]


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
    # False = only the cheap, policy-independent "raw" stats are filled
    # (Time/Notes/density/gaps); risk + recommended profile are still being
    # computed in the background. True = full schedule risk analysis is present.
    analyzed: bool = True


_metadata_cache: OrderedDict[tuple[Any, ...], SongUiMetadata] = OrderedDict()
_raw_cache: OrderedDict[tuple[Any, ...], SongUiMetadata] = OrderedDict()
_persistent_cache: OrderedDict[str, SongUiMetadata] = OrderedDict()
_persistent_loaded = False
_cache_lock = RLock()
_song_repository = get_shared_song_repository()

PERSISTENT_CACHE_SCHEMA_VERSION = 3
PERSISTENT_CACHE_PATH = Path(".cache") / "sky_music" / "picker_metadata.sqlite3"
_PERSISTENT_POLICY_ATTRS: tuple[str, ...] = (
    "hold_us",
    "min_hold_us",
    "min_hold_margin_us",
    "spin_threshold_us",
    "focus_restore_grace_us",
    "same_key_conflict_policy",
    "frame_us",
    "fps",
)

# ---------------------------------------------------------------------------
# Phase 1A – SHA-256 persistent-key RAM cache
# ---------------------------------------------------------------------------
# _persistent_cache_key() is called in the render path (peek_cached_song_ui_metadata)
# for every visible row on every frame repaint.  Computing stat() + json.dumps() +
# sha256() per call is expensive.  We cache the result keyed by a cheap tuple
# (resolved_path_str, mtime_ns, size, profile_name, tempo_scale, fps, …).
# The cache is automatically invalidated when the file's mtime/size changes.
_pkey_ram_cache: OrderedDict[tuple[Any, ...], str] = OrderedDict()
_pkey_ram_lock = RLock()

# ---------------------------------------------------------------------------
# Phase 1B – Persistent SQLite connection + deferred pruning
# ---------------------------------------------------------------------------
# Opening a new sqlite3.Connection on every write adds ~1-3 ms of overhead.
# We keep one connection per thread (thread-local) to eliminate that cost.
# _PRUNE_EVERY_N_WRITES writes instead of after each UPSERT.
_PRUNE_EVERY_N_WRITES: int = 50
_write_counter: int = 0
_write_counter_lock = RLock()
_tls = threading.local()  # thread-local storage for per-thread SQLite connections

# ---------------------------------------------------------------------------
# Phase 3B – Path+session → RAM-key short-circuit cache
# ---------------------------------------------------------------------------
# peek_cached_song_ui_metadata() is the hottest render-path function.  Its
# first step is _song_repository.cache_key(song_path) which, even with the
# _identity_cache, requires a dict lookup keyed by (Path, int(profile)).  For
# 111 songs × 10+ repaints/s that adds up.  We cache the final ram_key tuple
# keyed by (song_path_str, session_signature) so that after the first render
# we bypass cache_key() completely on every subsequent frame.
_path_session_ram_cache: OrderedDict[tuple[str, tuple[Any, ...]], tuple[tuple[Any, ...], tuple[Any, ...]]] = OrderedDict()
_path_session_ram_lock = RLock()


def _session_signature(session: PlaybackSessionContext) -> tuple[Any, ...]:
    """Cheap, hashable identity for a session (no config file access)."""
    return (
        session.profile_name,
        session.fps,
        session.tempo_scale,
        session.scan_code_mode,
        session.same_key_conflict_policy,
        session.policy_overrides,
    )


def _update_path_session_ram_cache(
    song_path: Path,
    session: PlaybackSessionContext,
    ram_key: tuple[Any, ...],
    song_file_key: tuple[Any, ...],
) -> bool:
    path_str = str(song_path)
    sig = _session_signature(session)
    ps_key = (path_str, sig)
    with _path_session_ram_lock:
        if ps_key in _path_session_ram_cache and _path_session_ram_cache[ps_key] == (ram_key, song_file_key):
            # Put it at the front of the LRU
            _lru_get(_path_session_ram_cache, ps_key)
            return False
        _lru_set(
            _path_session_ram_cache,
            ps_key,
            (ram_key, song_file_key),
            maxsize=_PATH_SESSION_RAM_CACHE_MAX,
        )
        return True


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

def _metadata_to_payload(meta: SongUiMetadata) -> dict[str, Any]:
    payload: dict[str, Any] = asdict(meta)
    payload["path"] = str(meta.path)
    payload["warnings"] = list(meta.warnings)
    return payload


def _raw_metadata_payload(meta: SongUiMetadata) -> dict[str, Any]:
    """Serialise only policy-independent fields for the persistent raw table.

    A fully analyzed ``SongUiMetadata`` (risk / recommended profile / tempo /
    warnings derived from a specific session) must NEVER be written verbatim to
    the ``picker_raw_metadata`` table, otherwise a later session could rehydrate
    it, treat the song as analyzed, and reuse stale session-specific results.

    The raw payload keeps only fields that do not depend on the timing policy
    and forces ``analyzed=False`` so the coordinator always runs a fresh full
    analysis for the current session.
    """
    return {
        "path": str(meta.path),
        "name": meta.name,
        "duration_seconds": meta.duration_seconds,
        "note_count": meta.note_count,
        "max_polyphony": meta.max_polyphony,
        "min_note_gap_ms": meta.min_note_gap_ms,
        "min_same_key_gap_ms": meta.min_same_key_gap_ms,
        "average_notes_per_second": meta.average_notes_per_second,
        "peak_notes_per_second_1s": meta.peak_notes_per_second_1s,
        "max_chord_size": meta.max_chord_size,
        "chords_count": meta.chords_count,
        "analyzed": False,
        "risk": "low",
        "recommended_profile": "",
        "recommended_tempo_scale": 1.0,
        "warnings": (),
        "impossible_repeats": 0,
        "timing_stress_rate": 0.0,
    }


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
            analyzed=bool(payload.get("analyzed", True)),
        )
    except Exception:
        return None


def _stable_file_identity(song_path: Path) -> dict[str, Any]:
    """Return {path, mtime_ns, size} for cache-key construction.

    Note: _persistent_cache_key() now calls song_path.stat() itself and reuses
    the result, so this helper is only kept for external callers that still need
    a standalone identity dict.
    """
    stat = song_path.stat()
    try:
        path_key = str(song_path.resolve())
    except Exception:
        path_key = str(song_path)
    return {
        "path": path_key,
        "mtime_ns": stat.st_mtime_ns,
        "size": stat.st_size,
    }


def _effective_policy_signature(session: PlaybackSessionContext, cfg: AppConfig | None) -> dict[str, Any]:
    policy = session.resolve_effective_policy(cfg)
    return {
        attr: getattr(policy, attr)
        for attr in _PERSISTENT_POLICY_ATTRS
        if hasattr(policy, attr)
    }


def _canonical_path_str(path: Path) -> str:
    """Single canonical string for a song path used for persistent cache keys.

    Both the write path (given a ``song_file_key``) and the read path (given a
    bare ``song_path``) must hash the *same* string or the SHA-256 cache key
    diverges silently (e.g. ``songs/x.json`` vs ``C:\\...\\songs\\x.json``).
    """
    try:
        return str(path.resolve(strict=False))
    except Exception:
        return str(path)


def _persistent_cache_key(
    song_path: Path,
    session: PlaybackSessionContext,
    cfg: AppConfig | None,
    *,
    song_file_key: tuple[Any, ...] | None = None,
) -> str | None:
    """Return the SHA-256 cache key for a (song_path, session) pair.

    Phase 1A: The SHA-256 computation (stat + json.dumps + sha256) is expensive
    and was being called on every frame repaint in the render path.  We now cache
    the result in a module-level dict keyed by a cheap tuple that encodes all
    inputs that can change the key.  The cache entry is automatically stale-safe
    because mtime_ns and size are part of the RAM key – if the file changes the
    tuple changes and we recompute.
    """
    try:
        if song_file_key is not None:
            path_str = _canonical_path_str(song_file_key[0])
            mtime_ns = int(song_file_key[1])
            size = int(song_file_key[2])
        else:
            stat = song_path.stat()
            path_str = _canonical_path_str(song_path)
            mtime_ns = stat.st_mtime_ns
            size = stat.st_size

        # Build a lightweight, hashable RAM key without any SHA-256 work.
        ram_key = (
            path_str,
            mtime_ns,
            size,
            session.profile_name,
            session.tempo_scale,
            session.fps,
            session.scan_code_mode,
            session.same_key_conflict_policy,
            session.policy_overrides,
            PERSISTENT_CACHE_SCHEMA_VERSION,
            sys.platform,
        )
        with _pkey_ram_lock:
            cached_key = _lru_get(_pkey_ram_cache, ram_key)
        if cached_key is not None:
            return cached_key

        # Cache miss – pay the sha256 cost once and store the result.
        file_identity = {
            "path": path_str,
            "mtime_ns": mtime_ns,
            "size": size,
        }
        payload = {
            "schema": PERSISTENT_CACHE_SCHEMA_VERSION,
            "file": file_identity,
            "session": {
                "profile_name": session.profile_name,
                "tempo_scale": session.tempo_scale,
                "fps": session.fps,
                "scan_code_mode": session.scan_code_mode,
                "same_key_conflict_policy": session.same_key_conflict_policy,
                "policy_overrides": list(session.policy_overrides),
            },
            "effective_policy": _effective_policy_signature(session, cfg),
            "platform": sys.platform,
        }
        encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"), default=str).encode("utf-8")
        result = hashlib.sha256(encoded).hexdigest()

        with _pkey_ram_lock:
            _lru_set(_pkey_ram_cache, ram_key, result, maxsize=_PKEY_RAM_CACHE_MAX)
        return result
    except Exception:
        return None


def _persistent_raw_cache_key(
    song_path: Path,
    *,
    song_file_key: tuple[Any, ...] | None = None,
) -> str | None:
    try:
        if song_file_key is not None:
            path_str = _canonical_path_str(song_file_key[0])
            mtime_ns = int(song_file_key[1])
            size = int(song_file_key[2])
        else:
            stat = song_path.stat()
            path_str = _canonical_path_str(song_path)
            mtime_ns = stat.st_mtime_ns
            size = stat.st_size

        ram_key = (
            path_str,
            mtime_ns,
            size,
            "RAW",
            PERSISTENT_CACHE_SCHEMA_VERSION,
        )
        with _pkey_ram_lock:
            cached_key = _lru_get(_pkey_ram_cache, ram_key)
        if cached_key is not None:
            return cached_key

        file_identity = {
            "path": path_str,
            "mtime_ns": mtime_ns,
            "size": size,
        }
        payload = {
            "schema": PERSISTENT_CACHE_SCHEMA_VERSION,
            "file": file_identity,
            "raw_only": True,
        }
        encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"), default=str).encode("utf-8")
        result = hashlib.sha256(encoded).hexdigest()

        with _pkey_ram_lock:
            _lru_set(_pkey_ram_cache, ram_key, result, maxsize=_PKEY_RAM_CACHE_MAX)
        return result
    except Exception:
        return None


# ---------------------------------------------------------------------------
# Phase 1B – SQLite connection management
# ---------------------------------------------------------------------------

def _connect_persistent_cache() -> sqlite3.Connection:
    """Open (or reuse) the thread-local SQLite connection.

    Phase 1B: each thread keeps one connection alive for the lifetime of the
    process instead of opening/closing on every write.  The WAL journal means
    concurrent readers never block the writer.
    """
    conn: sqlite3.Connection | None = getattr(_tls, "db_conn", None)
    if conn is not None:
        # Validate the connection is still usable (handles rare OS-level errors).
        try:
            conn.execute("SELECT 1")
            return conn
        except sqlite3.Error:
            _tls.db_conn = None

    PERSISTENT_CACHE_PATH.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(PERSISTENT_CACHE_PATH, timeout=0.5, check_same_thread=False)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute("PRAGMA cache_size=-4096")  # 4 MB page cache
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS picker_metadata (
            cache_key TEXT PRIMARY KEY,
            payload TEXT NOT NULL,
            updated_at REAL NOT NULL
        )
        """
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_picker_metadata_updated_at ON picker_metadata(updated_at)"
    )
    
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS picker_raw_metadata (
            cache_key TEXT PRIMARY KEY,
            payload TEXT NOT NULL,
            updated_at REAL NOT NULL
        )
        """
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_picker_raw_metadata_updated_at ON picker_raw_metadata(updated_at)"
    )
    conn.commit()
    _tls.db_conn = conn
    return conn


def _close_persistent_cache_connection() -> None:
    """Close the thread-local SQLite connection if open (call on thread shutdown)."""
    conn: sqlite3.Connection | None = getattr(_tls, "db_conn", None)
    if conn is not None:
        with contextlib.suppress(Exception):
            conn.close()
        _tls.db_conn = None


# ---------------------------------------------------------------------------
# Public cache API
# ---------------------------------------------------------------------------

def warm_persistent_metadata_cache(
    limit: int | None = None,
    *,
    song_paths: list[Path] | tuple[Path, ...] | None = None,
) -> int:
    """Load persistent picker metadata into memory.

    Phase 3A: ``limit`` is now optional.  When omitted (the common case) we
    derive an adaptive ceiling that keeps the warm-up query tight while still
    retaining enough history for session-to-session cache hits:

        effective_limit = max(500, len(song_paths) * 8)   # ~8 historical sessions

    If ``song_paths`` is not supplied either, we fall back to a conservative
    500-row default (enough for the common ≤111-song library) instead of the
    old hard-coded 6000.

    This is safe to run from the picker cache worker.
    `peek_cached_song_ui_metadata` intentionally does not do disk I/O, so
    the UI thread stays responsive.
    """
    global _persistent_loaded
    with _cache_lock:
        if _persistent_loaded:
            return len(_persistent_cache)

    # Phase 3A: derive adaptive limit.
    if limit is not None:
        effective_limit = max(1, limit)
    elif song_paths is not None:
        effective_limit = max(500, len(song_paths) * 8)
    else:
        effective_limit = 500  # conservative default; beats old hard-coded 6000 for small libs

    loaded: dict[str, SongUiMetadata] = {}
    try:
        conn = _connect_persistent_cache()
        rows = conn.execute(
            """
            SELECT cache_key, payload
            FROM picker_metadata
            ORDER BY updated_at DESC
            LIMIT ?
            """,
            (effective_limit,),
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
        for k, v in loaded.items():
            _lru_set(_persistent_cache, k, v, maxsize=_PERSISTENT_CACHE_MAX)
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

    Returns the number of rows that became (re)paintable: the sum of full
    session metadata rows AND policy-independent raw rows loaded, so callers can
    trigger a UI repaint even when only raw stats were available on disk.
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
    conn = _connect_persistent_cache()
    try:
        rows = conn.execute(
            f"SELECT cache_key, payload FROM picker_metadata WHERE cache_key IN ({placeholders})",
            tuple(missing_keys),
        ).fetchall()
    except Exception:
        rows = []

    loaded: dict[str, SongUiMetadata] = {}
    for key, payload_text in rows:
        try:
            payload = json.loads(str(payload_text))
            meta = _metadata_from_payload(payload)
            if meta is not None:
                loaded[str(key)] = meta
        except Exception:
            continue

    # Seed full rows into RAM caches (stat/parse happens outside the lock).
    prepared_full: list[tuple[str, Path, SongUiMetadata]] = []
    for key, meta in loaded.items():
        song_path = key_to_path[key]
        prepared_full.append((key, song_path, meta))
    if prepared_full:
        with _cache_lock:
            for key, song_path, meta in prepared_full:
                try:
                    song_file_key = _song_repository.cache_key(song_path)
                    ram_key = session.metadata_cache_key(song_file_key, cfg)
                    _lru_set(_persistent_cache, key, meta, maxsize=_PERSISTENT_CACHE_MAX)
                    _lru_set(_metadata_cache, ram_key, meta, maxsize=_METADATA_CACHE_MAX)
                    _update_path_session_ram_cache(song_path, session, ram_key, song_file_key)
                except Exception:
                    continue

    # Raw (policy-independent) hydration — count and seed separately so a raw-only
    # hit still reports a positive change and triggers a UI repaint.
    raw_loaded = 0
    raw_keys_to_path: dict[str, Path] = {}
    for path in paths:
        raw_key = _persistent_raw_cache_key(path)
        if raw_key is not None:
            raw_keys_to_path[raw_key] = path

    if raw_keys_to_path:
        with _cache_lock:
            existing_raw_keys = set(_raw_cache)
        missing_raw: list[str] = []
        raw_sf_keys: dict[str, tuple[Any, ...]] = {}
        for rk in raw_keys_to_path:
            try:
                sf_key = _song_repository.cache_key(raw_keys_to_path[rk])
            except Exception:
                continue
            raw_sf_keys[rk] = sf_key
            if sf_key not in existing_raw_keys:
                missing_raw.append(rk)
            else:
                ram_key = session.metadata_cache_key(sf_key, cfg)
                if _update_path_session_ram_cache(raw_keys_to_path[rk], session, ram_key, sf_key):
                    raw_loaded += 1
        if missing_raw:
            raw_placeholders = ",".join("?" for _ in missing_raw)
            try:
                raw_rows = conn.execute(
                    f"SELECT cache_key, payload FROM picker_raw_metadata WHERE cache_key IN ({raw_placeholders})",
                    tuple(missing_raw),
                ).fetchall()
            except Exception:
                raw_rows = []

            prepared_raw: list[tuple[Path, tuple[Any, ...], tuple[Any, ...], SongUiMetadata]] = []
            for rk, payload_text in raw_rows:
                try:
                    payload = json.loads(str(payload_text))
                    meta = _metadata_from_payload(payload)
                    if meta is None:
                        continue
                    song_path = raw_keys_to_path[rk]
                    sf_key = raw_sf_keys[rk]
                    ram_key = session.metadata_cache_key(sf_key, cfg)
                    prepared_raw.append((song_path, sf_key, ram_key, meta))
                except Exception:
                    continue
            if prepared_raw:
                with _cache_lock:
                    for song_path, sf_key, ram_key, meta in prepared_raw:
                        _lru_set(_raw_cache, sf_key, meta, maxsize=_RAW_CACHE_MAX)
                        _update_path_session_ram_cache(song_path, session, ram_key, sf_key)
                        raw_loaded += 1

    return len(loaded) + raw_loaded


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
        "tempo_scale": session.tempo_scale,
        "fps": session.fps,
        "scan_code_mode": session.scan_code_mode,
        "same_key_conflict_policy": session.same_key_conflict_policy,
        "policy_overrides": list(session.policy_overrides),
    }


def _session_from_worker_payload(payload: dict[str, Any]) -> PlaybackSessionContext:
    raw_overrides = payload.get("policy_overrides", ())
    overrides: list[tuple[str, Any]] = [
        (str(item[0]), item[1])
        for item in raw_overrides
        if isinstance(item, list | tuple) and len(item) == 2
    ] if isinstance(raw_overrides, list | tuple) else []

    conflict_policy = str(payload.get("same_key_conflict_policy", "drop_chord"))
    if conflict_policy not in {"degraded", "drop_chord", "strict"}:
        conflict_policy = "drop_chord"

    return PlaybackSessionContext(
        profile_name=str(payload.get("profile_name", "balanced")),
        tempo_scale=float(payload.get("tempo_scale", 1.0)),
        fps=payload.get("fps"),  # type: ignore[arg-type]
        scan_code_mode=str(payload.get("scan_code_mode", "physical")),
        same_key_conflict_policy=conflict_policy,  # type: ignore[arg-type]
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


def worker_process_warmup() -> bool:
    """No-op submitted to ProcessPoolExecutor to pre-spawn the worker process.

    Phase 2B: On Windows, ProcessPoolExecutor uses the 'spawn' start method,
    meaning every new worker process must import the entire module tree from
    scratch (~200-500 ms).  By submitting this trivial task immediately after
    creating the executor we overlap that spawn latency with the first UI render
    instead of paying it when the first real analysis batch is dispatched.

    Returns True so the caller can confirm the worker is alive.
    """
    return True


def store_computed_song_ui_metadata_payloads(
    payloads: list[dict[str, Any]] | tuple[dict[str, Any], ...],
    session: PlaybackSessionContext,
    cfg: AppConfig | None = None,
) -> int:
    """Store metadata returned by a worker into parent RAM + SQLite caches.

    Optimized to open a single transaction to execute all batch updates together,
    yielding a massive speedup on disk writes.
    """
    stored = 0
    global _write_counter
    if not payloads:
        return 0

    conn = _connect_persistent_cache()
    try:
        # NOTE: do not call conn.execute("BEGIN") here. Other cache writers
        # (e.g. populate_raw_song_ui_metadata_for_paths) run on the same
        # thread-local connection and may leave an implicit transaction open,
        # which would make an explicit BEGIN raise "cannot start a transaction
        # within a transaction". We let SQLite open an implicit transaction and
        # commit once at the end.
        for payload in payloads:
            try:
                meta = _metadata_from_payload(payload)
                if meta is None:
                    continue
                song_file_key = _song_repository.cache_key(meta.path)
                ram_key = session.metadata_cache_key(song_file_key, cfg)
                with _cache_lock:
                    _lru_set(_metadata_cache, ram_key, meta, maxsize=_METADATA_CACHE_MAX)
                _update_path_session_ram_cache(meta.path, session, ram_key, song_file_key)

                # Inline _store_persistent_metadata logic to run inside the batch transaction
                if meta.analyzed:
                    key = _persistent_cache_key(meta.path, session, cfg, song_file_key=song_file_key)
                    if key is not None:
                        payload_str = json.dumps(_metadata_to_payload(meta), ensure_ascii=False, separators=(",", ":"))
                        with _cache_lock:
                            _lru_set(_persistent_cache, key, meta, maxsize=_PERSISTENT_CACHE_MAX)

                        conn.execute(
                            """
                            INSERT OR REPLACE INTO picker_metadata(cache_key, payload, updated_at)
                            VALUES (?, ?, ?)
                            """,
                            (key, payload_str, time.time()),
                        )
                        stored += 1

                raw_key = _persistent_raw_cache_key(meta.path, song_file_key=song_file_key)
                if raw_key is not None:
                    # NEVER persist full session metadata into the raw table:
                    # store only policy-independent fields with analyzed=False so
                    # a different session/later run re-runs full analysis.
                    raw_payload_str = json.dumps(_raw_metadata_payload(meta), ensure_ascii=False, separators=(",", ":"))
                    conn.execute(
                        """
                        INSERT OR REPLACE INTO picker_raw_metadata(cache_key, payload, updated_at)
                        VALUES (?, ?, ?)
                        """,
                        (raw_key, raw_payload_str, time.time()),
                    )
            except Exception:
                # A single malformed payload must not abort the whole batch.
                continue

        conn.commit()

        # Opportunistic pruning check after transaction completes
        if stored > 0:
            with _write_counter_lock:
                _write_counter += stored
                should_prune = (_write_counter // _PRUNE_EVERY_N_WRITES) > ((_write_counter - stored) // _PRUNE_EVERY_N_WRITES)

            if should_prune:
                try:
                    conn.execute(
                        f"""
                        DELETE FROM picker_metadata
                        WHERE cache_key NOT IN (
                            SELECT cache_key FROM picker_metadata
                            ORDER BY updated_at DESC
                            LIMIT {_PERSISTENT_CACHE_MAX_SQLITE}
                        )
                        """
                    )
                    conn.execute(
                        f"""
                        DELETE FROM picker_raw_metadata
                        WHERE cache_key NOT IN (
                            SELECT cache_key FROM picker_raw_metadata
                            ORDER BY updated_at DESC
                            LIMIT {_PERSISTENT_CACHE_MAX_SQLITE}
                        )
                        """
                    )
                    conn.commit()
                except Exception:
                    pass
    except Exception:
        with contextlib.suppress(Exception):
            conn.rollback()
        return 0

    return stored


def _store_persistent_metadata(
    song_path: Path,
    session: PlaybackSessionContext,
    cfg: AppConfig | None,
    meta: SongUiMetadata,
    *,
    song_file_key: tuple[Any, ...] | None = None,
) -> None:
    """Upsert analyzed metadata into the SQLite persistent cache.

    Phase 1B changes:
    - Reuse the thread-local SQLite connection instead of opening a new one.
    - Defer the expensive pruning DELETE to every _PRUNE_EVERY_N_WRITES writes
      rather than running it after every UPSERT.
    """
    # Never persist raw-only stats; only authoritative, fully analyzed metadata
    # belongs in the cross-session SQLite cache.
    if not meta.analyzed:
        return

    key = _persistent_cache_key(song_path, session, cfg, song_file_key=song_file_key)
    if key is None:
        return

    payload = json.dumps(_metadata_to_payload(meta), ensure_ascii=False, separators=(",", ":"))
    with _cache_lock:
        _lru_set(_persistent_cache, key, meta, maxsize=_PERSISTENT_CACHE_MAX)

    try:
        conn = _connect_persistent_cache()
        conn.execute(
            """
            INSERT OR REPLACE INTO picker_metadata(cache_key, payload, updated_at)
            VALUES (?, ?, ?)
            """,
            (key, payload, time.time()),
        )
        conn.commit()

        # Phase 1B: opportunistic pruning runs only every N writes to avoid the
        # expensive DELETE … ORDER BY … LIMIT scan after every single UPSERT.
        global _write_counter
        with _write_counter_lock:
            _write_counter += 1
            should_prune = (_write_counter % _PRUNE_EVERY_N_WRITES) == 0

        if should_prune:
            try:
                conn.execute(
                    f"""
                    DELETE FROM picker_metadata
                    WHERE cache_key NOT IN (
                        SELECT cache_key FROM picker_metadata
                        ORDER BY updated_at DESC
                        LIMIT {_PERSISTENT_CACHE_MAX_SQLITE}
                    )
                    """
                )
                conn.commit()
            except Exception:
                pass
    except Exception:
        return


def _peek_persistent_metadata(
    song_path: Path,
    session: PlaybackSessionContext,
    cfg: AppConfig | None,
    *,
    song_file_key: tuple[Any, ...] | None = None,
) -> SongUiMetadata | None:
    # Never load from disk here; this function is used in render paths.
    # It may still return entries hydrated by a targeted background cache read
    # before the full persistent cache warmup has completed.
    key = _persistent_cache_key(song_path, session, cfg, song_file_key=song_file_key)
    if key is None:
        return None
    with _cache_lock:
        return _lru_get(_persistent_cache, key)


def get_song_ui_metadata(
    song_path: Path,
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> SongUiMetadata:
    session = session or PlaybackSessionContext.balanced()
    try:
        from sky_music.domain.analyzer import analyze_schedule
        from sky_music.domain.scheduler import build_key_actions

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
        min_same_key_gap = (report.min_same_key_gap_us / 1000.0) if report.min_same_key_gap_us is not None else 0.0

        return SongUiMetadata(
            path=song_path,
            # Title shown in the picker is the filename stem (single, instantly
            # available, sortable source of truth) so the list collates A→Z and the
            # visible title always matches the sort/search key. The song's internal
            # JSON "name" can diverge wildly from the filename and is not used here.
            name=song_path.stem,
            duration_seconds=sched.source_duration_us / 1_000_000,
            note_count=sched.note_count,
            max_polyphony=report.max_polyphony,
            min_note_gap_ms=min_note_gap,
            min_same_key_gap_ms=min_same_key_gap,
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


def compute_raw_song_ui_metadata(song_path: Path) -> SongUiMetadata:
    """Cheap, policy-independent stats straight from parsed notes (no scheduler).

    Fills the columns that do not depend on the timing profile so the picker can
    show Time/Notes/Density/gaps immediately (~1ms) while the much heavier risk
    analysis (~5ms, schedule + analyze) runs in the background. ``analyzed`` is
    False so the UI knows risk/recommendation are still pending.

    Phase 1C: parse_song_file() already sorts notes by time_ms, so we no longer
    sort the slice three separate times.  We iterate the pre-sorted tuple once
    to build all needed metrics in O(n) time.
    """
    try:
        song = _song_repository.load(song_path)
        notes = song.notes  # already sorted by time_ms from parse_song_file()
        note_count = len(notes)
        if note_count == 0:
            return SongUiMetadata(
                path=song_path, name=song_path.stem, duration_seconds=0.0,
                note_count=0, max_polyphony=0, min_note_gap_ms=0.0,
                min_same_key_gap_ms=0.0, risk="low", recommended_profile="",
                recommended_tempo_scale=1.0, warnings=(), analyzed=False,
            )

        # Phase 1C: single pass – notes are pre-sorted, so times is already ordered.
        times: list[int] = [int(n.time_ms) for n in notes]
        duration_seconds = times[-1] / 1000.0

        # Min gap between any two *distinct* onset timestamps + onset_counts in one pass.
        onset_counts: dict[int, int] = {}
        prev_distinct: int | None = None
        min_gap_int: int = 0
        for t in times:
            onset_counts[t] = onset_counts.get(t, 0) + 1
            if prev_distinct is None:
                prev_distinct = t
            elif t != prev_distinct:
                gap = t - prev_distinct
                if min_gap_int == 0 or gap < min_gap_int:
                    min_gap_int = gap
                prev_distinct = t
        min_note_gap_ms = float(min_gap_int)

        max_chord_size = max(onset_counts.values())
        chords_count = sum(1 for c in onset_counts.values() if c > 1)

        # Same-key gap: notes already sorted, so no second sort needed.
        key_last: dict[Any, int] = {}
        same_key_gaps: list[int] = []
        for note in notes:  # Phase 1C: was sorted(notes, key=lambda n: n.time_ms)
            t = int(note.time_ms)
            if note.key in key_last:
                same_key_gaps.append(t - key_last[note.key])
            key_last[note.key] = t
        min_same_key_gap_ms = float(min(same_key_gaps)) if same_key_gaps else 0.0

        average_notes_per_second = note_count / duration_seconds if duration_seconds > 0 else 0.0

        # Sliding-window peak density (1-second window) – O(n), no extra sort.
        peak = 0
        left = 0
        for right in range(note_count):
            while times[right] - times[left] > 1000:
                left += 1
            peak = max(peak, right - left + 1)

        return SongUiMetadata(
            path=song_path, name=song_path.stem,
            duration_seconds=duration_seconds, note_count=note_count,
            max_polyphony=max_chord_size,  # exact lower bound; refined once analyzed
            min_note_gap_ms=min_note_gap_ms, min_same_key_gap_ms=min_same_key_gap_ms,
            risk="low", recommended_profile="", recommended_tempo_scale=1.0,
            warnings=(),
            average_notes_per_second=average_notes_per_second,
            peak_notes_per_second_1s=float(peak),
            impossible_repeats=0, max_chord_size=max_chord_size,
            chords_count=chords_count, timing_stress_rate=0.0,
            analyzed=False,
        )
    except Exception as exc:
        # A parse/read failure is terminal, not pending — mark analyzed so the UI
        # shows the error instead of an endless "analyzing…" state.
        return SongUiMetadata(
            path=song_path, name=song_path.stem, duration_seconds=0.0,
            note_count=0, max_polyphony=0, min_note_gap_ms=0.0,
            min_same_key_gap_ms=0.0, risk="error", recommended_profile="unplayable",
            recommended_tempo_scale=1.0, warnings=(f"Failed to read song: {exc}",),
            analyzed=True,
        )


def populate_raw_song_ui_metadata_for_paths(
    paths: list[Path] | tuple[Path, ...],
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> int:
    """Seed the RAM cache with raw stats for paths that have no entry yet.

    Runs in the lightweight cache worker. Never persists to SQLite (raw entries
    are not authoritative) and never clobbers a fully analyzed entry.
    """
    if not paths:
        return 0
    session = session or PlaybackSessionContext.balanced()
    filled = 0
    global _write_counter
    wrote = False
    conn: sqlite3.Connection | None = None
    for path in paths:
        try:
            song_file_key = _song_repository.cache_key(path)
        except Exception:
            continue
        ram_key = session.metadata_cache_key(song_file_key, cfg)
        with _cache_lock:
            if _lru_get(_metadata_cache, ram_key) is not None:
                if _update_path_session_ram_cache(path, session, ram_key, song_file_key):
                    filled += 1
                continue
            if _lru_get(_raw_cache, song_file_key) is not None:
                if _update_path_session_ram_cache(path, session, ram_key, song_file_key):
                    filled += 1
                continue
        meta = compute_raw_song_ui_metadata(path)
        with _cache_lock:
            if _lru_get(_metadata_cache, ram_key) is None:
                _lru_set(_raw_cache, song_file_key, meta, maxsize=_RAW_CACHE_MAX)
                filled += 1
        _update_path_session_ram_cache(path, session, ram_key, song_file_key)

        try:
            raw_key = _persistent_raw_cache_key(path, song_file_key=song_file_key)
            if raw_key is not None:
                raw_payload_str = json.dumps(_raw_metadata_payload(meta), ensure_ascii=False, separators=(",", ":"))
                conn = _connect_persistent_cache()
                conn.execute(
                    """
                    INSERT OR REPLACE INTO picker_raw_metadata(cache_key, payload, updated_at)
                    VALUES (?, ?, ?)
                    """,
                    (raw_key, raw_payload_str, time.time()),
                )
                wrote = True

                with _write_counter_lock:
                    _write_counter += 1
                    should_prune = (_write_counter % _PRUNE_EVERY_N_WRITES) == 0
                if should_prune:
                    conn.execute(
                        f"""
                        DELETE FROM picker_raw_metadata
                        WHERE cache_key NOT IN (
                            SELECT cache_key FROM picker_raw_metadata
                            ORDER BY updated_at DESC
                            LIMIT {_PERSISTENT_CACHE_MAX_SQLITE}
                        )
                        """
                    )
        except Exception:
            pass
    # Commit once so we never leave an implicit transaction open on the shared
    # thread-local connection (a dangling transaction makes a later explicit
    # BEGIN from another writer raise "cannot start a transaction ...").
    if wrote and conn is not None:
        with contextlib.suppress(Exception):
            conn.commit()
    return filled


def hydrate_and_fill_raw_metadata(
    paths: list[Path] | tuple[Path, ...],
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> int:
    """Cache-worker entry point: disk-cached full metadata first, then raw stats.

    Returns how many rows became (re)paintable so the caller can decide to repaint.
    """
    loaded = hydrate_persistent_metadata_for_paths(paths, session, cfg)
    raw = populate_raw_song_ui_metadata_for_paths(paths, session, cfg)
    return loaded + raw


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
    _update_path_session_ram_cache(song_path, session, ram_key, song_file_key)
    with _cache_lock:
        cached = _lru_get(_metadata_cache, ram_key)
    if cached is not None:
        return cached

    persistent = _peek_persistent_metadata(song_path, session, cfg, song_file_key=song_file_key)
    if persistent is not None:
        with _cache_lock:
            _lru_set(_metadata_cache, ram_key, persistent, maxsize=_METADATA_CACHE_MAX)
        return persistent

    meta = get_song_ui_metadata(song_path, session, cfg)
    with _cache_lock:
        _lru_set(_metadata_cache, ram_key, meta, maxsize=_METADATA_CACHE_MAX)
    _store_persistent_metadata(song_path, session, cfg, meta, song_file_key=song_file_key)
    return meta


def peek_cached_song_ui_metadata(
    song_path: Path,
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,  # noqa: ARG001
) -> SongUiMetadata | None:
    """Return cached metadata without parsing/analyzing the song.

    This function is safe in render paths: it checks RAM and already-warmed
    persistent cache only, never disk-loads synchronously.

    Phase 3B: We maintain a path+session → ram_key short-circuit cache so that
    on every frame repaint after the first we skip _song_repository.cache_key()
    (and its dict lookup) entirely for paths we have already seen.
    """
    session = session or PlaybackSessionContext.balanced()
    sig = _session_signature(session)
    path_str = str(song_path)
    ps_key = (path_str, sig)

    # Fast path: if we already know the ram_key for this path+session, skip
    # _song_repository.cache_key() entirely.
    with _path_session_ram_lock:
        keys = _lru_get(_path_session_ram_cache, ps_key)

    if keys is not None:
        ram_key, song_file_key = keys
        with _cache_lock:
            cached = _lru_get(_metadata_cache, ram_key)
        if cached is not None:
            return cached
            
        with _cache_lock:
            cached_raw = _lru_get(_raw_cache, song_file_key)
        if cached_raw is not None:
            return cached_raw

    # ZERO I/O ON RENDER:
    # Do not call _song_repository.cache_key(song_path) here because it calls stat().
    # The background worker (MetadataCoordinator) will call hydrate_persistent_metadata_for_paths
    # and populate_raw_song_ui_metadata_for_paths, which will compute the keys and populate
    # _metadata_cache and _path_session_ram_cache.
    return None


def clear_metadata_cache(*, clear_persistent: bool = False) -> None:
    global _persistent_loaded, _write_counter
    with _cache_lock:
        _metadata_cache.clear()
        _raw_cache.clear()
        _song_repository.clear()
        if clear_persistent:
            _persistent_cache.clear()
            _persistent_loaded = False
    # Phase 1A: also invalidate the SHA-256 RAM key cache.
    with _pkey_ram_lock:
        _pkey_ram_cache.clear()
    # Phase 3B: invalidate the path+session → ram_key short-circuit cache.
    with _path_session_ram_lock:
        _path_session_ram_cache.clear()
    # Theme search-index LRU (accent-insensitive match spans).
    with contextlib.suppress(Exception):
        from sky_music.ui.picker_theme import normalized_index_map

        normalized_index_map.cache_clear()
    if clear_persistent:
        # Phase 1B: close and discard the thread-local connection before unlinking.
        _close_persistent_cache_connection()
        with _write_counter_lock:
            _write_counter = 0
        with contextlib.suppress(Exception):
            PERSISTENT_CACHE_PATH.unlink(missing_ok=True)


def _get_song_recommendation(
    song_path: Path,
    session: PlaybackSessionContext | None = None,
    cfg: AppConfig | None = None,
) -> tuple[str, float]:
    meta = get_cached_song_ui_metadata(song_path, session, cfg)
    return meta.recommended_profile, meta.recommended_tempo_scale
