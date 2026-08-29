"""Bounded, presentation-neutral song catalog access.

The catalog owns path discovery and the opaque IDs used by future desktop
adapters. Callers that need a real path must resolve an ID through this service;
paths are never part of the public row/page values.
"""

from __future__ import annotations

import hashlib
import os
import threading
import unicodedata
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path

from rapidfuzz import fuzz, process

SUPPORTED_EXTENSIONS: frozenset[str] = frozenset({".json", ".skysheet", ".txt"})
DEFAULT_PAGE_SIZE = 100
MAX_PAGE_SIZE = 200
MAX_QUERY_LENGTH = 1024
FUZZY_SCORE_CUTOFF = 60.0


class CatalogError(RuntimeError):
    """Base error for invalid or unavailable catalog operations."""


class CatalogCollisionError(CatalogError):
    """Raised when two different canonical paths produce one song ID."""


class CatalogGenerationError(CatalogError):
    """Raised when a caller uses a stale catalog generation."""


class CatalogLookupError(CatalogError):
    """Raised when an unknown or malformed song ID is resolved."""


@dataclass(frozen=True, slots=True)
class CatalogRow:
    """A path-free catalog row safe to serialize to another process."""

    song_id: str
    title: str


@dataclass(frozen=True, slots=True)
class CatalogEntry:
    """Internal catalog entry retaining the backend-only path mapping."""

    song_id: str
    title: str
    path: Path
    search_key: str


@dataclass(frozen=True, slots=True)
class CatalogPage:
    """A bounded page of path-free rows from one catalog generation."""

    items: tuple[CatalogRow, ...]
    page: int
    page_size: int
    total: int
    generation: int

    @property
    def has_next(self) -> bool:
        return (self.page + 1) * self.page_size < self.total


@dataclass(frozen=True, slots=True)
class CatalogSnapshot:
    """The complete path-free catalog view after one successful scan."""

    items: tuple[CatalogRow, ...]
    generation: int
    total: int


def normalize_search_text(value: str) -> str:
    """Case-fold and remove accents using the picker’s established semantics."""
    if not value:
        return ""
    decomposed = unicodedata.normalize("NFKD", value)
    without_marks = "".join(char for char in decomposed if not unicodedata.combining(char))
    return without_marks.replace("đ", "d").replace("Đ", "D").casefold()


def canonical_path(path: Path | str) -> Path:
    """Return the non-strict canonical path used for IDs and collision checks."""
    return Path(path).resolve(strict=False)


def normalized_canonical_path(path: Path | str) -> str:
    """Return the platform-normalized canonical path used as the hash input."""
    return os.path.normcase(os.path.normpath(str(canonical_path(path))))


def song_id_for_path(path: Path | str) -> str:
    """Return the stable 32-hex ID required by the desktop catalog contract."""
    normalized = normalized_canonical_path(path)
    return hashlib.sha256(normalized.encode("utf-8")).hexdigest()[:32]


def _validate_query(query: str) -> str:
    if not isinstance(query, str):
        raise CatalogError("catalog query must be text")
    if len(query) > MAX_QUERY_LENGTH:
        raise CatalogError(f"catalog query exceeds {MAX_QUERY_LENGTH} characters")
    return normalize_search_text(query).strip()


def _validate_page(page: int, page_size: int) -> None:
    if type(page) is not int or page < 0:
        raise CatalogError("catalog page must be a non-negative integer")
    if type(page_size) is not int or not 1 <= page_size <= MAX_PAGE_SIZE:
        raise CatalogError(f"catalog page_size must be between 1 and {MAX_PAGE_SIZE}")


class CatalogService:
    """Own catalog generations, search, paging, and opaque-ID path lookup."""

    def __init__(self, songs_dir: Path | str = Path("songs")) -> None:
        self._songs_dir = Path(songs_dir)
        self._lock = threading.RLock()
        self._generation = 0
        self._entries: tuple[CatalogEntry, ...] = ()
        self._by_id: dict[str, CatalogEntry] = {}

    @property
    def songs_dir(self) -> Path:
        return self._songs_dir

    @property
    def generation(self) -> int:
        with self._lock:
            return self._generation

    def scan(self, songs_dir: Path | str | None = None) -> CatalogSnapshot:
        """Scan the configured directory and publish a new generation."""
        root = Path(songs_dir) if songs_dir is not None else self._songs_dir
        if not root.exists():
            paths: list[Path] = []
        elif not root.is_dir():
            raise CatalogError(f"songs directory is not a directory: {root}")
        else:
            paths = [
                path
                for path in root.iterdir()
                if path.is_file() and path.suffix.lower() in SUPPORTED_EXTENSIONS
            ]
        return self.replace_paths(paths)

    def scan_entries(self, songs_dir: Path | str | None = None) -> tuple[CatalogEntry, ...]:
        """Scan and return trusted in-process entries for legacy adapters."""
        self.scan(songs_dir)
        return self.entries()

    def replace_paths(self, paths: Iterable[Path | str]) -> CatalogSnapshot:
        """Publish a validated path list as a new catalog generation."""
        entries: list[CatalogEntry] = []
        by_id: dict[str, CatalogEntry] = {}
        seen_canonical: set[str] = set()

        for raw_path in paths:
            path = Path(raw_path)
            if path.suffix.lower() not in SUPPORTED_EXTENSIONS:
                continue
            normalized_path = normalized_canonical_path(path)
            if normalized_path in seen_canonical:
                continue
            seen_canonical.add(normalized_path)
            entry = CatalogEntry(
                song_id=song_id_for_path(path),
                title=path.stem,
                path=path,
                search_key=normalize_search_text(path.stem),
            )
            previous = by_id.get(entry.song_id)
            if previous is not None and normalized_canonical_path(previous.path) != normalized_path:
                raise CatalogCollisionError(
                    f"song ID collision for distinct paths: {previous.path} and {path}"
                )
            by_id[entry.song_id] = entry
            entries.append(entry)

        # Keep the legacy stable sort key.  ``list.sort`` preserves the scan
        # order for exact ties, which is part of the existing picker behavior.
        entries.sort(key=lambda entry: (entry.search_key, entry.title.casefold()))
        with self._lock:
            self._generation += 1
            self._entries = tuple(entries)
            self._by_id = by_id
            return self.snapshot()

    def snapshot(self) -> CatalogSnapshot:
        with self._lock:
            rows = tuple(CatalogRow(entry.song_id, entry.title) for entry in self._entries)
            return CatalogSnapshot(rows, self._generation, len(rows))

    def entries(self, *, generation: int | None = None) -> tuple[CatalogEntry, ...]:
        """Return backend entries for trusted in-process adapters such as Textual."""
        _generation, entries = self._entries_snapshot(generation)
        return entries

    def search(
        self,
        query: str = "",
        *,
        page: int = 0,
        page_size: int = DEFAULT_PAGE_SIZE,
        generation: int | None = None,
    ) -> CatalogPage:
        """Return an accent-insensitive fuzzy-search page without paths."""
        _validate_page(page, page_size)
        normalized = _validate_query(query)
        catalog_generation, entries = self._entries_snapshot(generation)
        if normalized:
            ranked_indices = self.rank_search_keys(
                [entry.search_key for entry in entries],
                normalized,
                score_cutoff=FUZZY_SCORE_CUTOFF,
            )
            entries = tuple(entries[index] for index in ranked_indices)
        start = page * page_size
        items = tuple(CatalogRow(entry.song_id, entry.title) for entry in entries[start : start + page_size])
        return CatalogPage(items, page, page_size, len(entries), catalog_generation)

    def search_entries(
        self,
        query: str = "",
        *,
        generation: int | None = None,
        score_cutoff: float = FUZZY_SCORE_CUTOFF,
    ) -> tuple[CatalogEntry, ...]:
        """Return ranked backend entries for an in-process presentation adapter."""
        normalized = _validate_query(query)
        if not 0 <= score_cutoff <= 100:
            raise CatalogError("catalog score_cutoff must be between 0 and 100")
        _catalog_generation, entries = self._entries_snapshot(generation)
        if not normalized:
            return entries
        ranked_indices = self.rank_search_keys(
            [entry.search_key for entry in entries],
            normalized,
            score_cutoff=score_cutoff,
        )
        return tuple(entries[index] for index in ranked_indices)

    @staticmethod
    def rank_search_keys(
        search_keys: list[str],
        query: str = "",
        *,
        score_cutoff: float = FUZZY_SCORE_CUTOFF,
    ) -> tuple[int, ...]:
        """Rank normalized keys with the same fuzzy/substring policy as search."""
        normalized = _validate_query(query)
        if not 0 <= score_cutoff <= 100:
            raise CatalogError("catalog score_cutoff must be between 0 and 100")
        if not normalized:
            return tuple(range(len(search_keys)))
        if len(normalized) == 1:
            return tuple(index for index, key in enumerate(search_keys) if normalized in key)

        by_index = dict(enumerate(search_keys))
        matches = process.extract(
            normalized,
            by_index,
            scorer=fuzz.WRatio,
            score_cutoff=score_cutoff,
            limit=None,
        )
        scores: dict[int, float] = {int(index): float(score) for _key, score, index in matches}
        for index, key in enumerate(search_keys):
            if normalized in key:
                scores[index] = max(scores.get(index, 0.0), 100.0)
        return tuple(sorted(scores, key=lambda index: (-scores[index], index)))

    def path_for_song_id(self, song_id: str, *, generation: int | None = None) -> Path:
        """Resolve an opaque ID to its backend path, rejecting malformed IDs."""
        if (
            not isinstance(song_id, str)
            or len(song_id) != 32
            or any(char not in "0123456789abcdef" for char in song_id)
        ):
            raise CatalogLookupError("malformed song ID")
        with self._lock:
            self._check_generation_locked(generation)
            entry = self._by_id.get(song_id)
        if entry is None:
            raise CatalogLookupError("unknown song ID")
        return entry.path

    def _entries_snapshot(
        self,
        generation: int | None,
    ) -> tuple[int, tuple[CatalogEntry, ...]]:
        """Read one generation and its entries as a linearizable snapshot."""
        with self._lock:
            self._check_generation_locked(generation)
            return self._generation, self._entries

    def _check_generation_locked(self, generation: int | None) -> None:
        if generation is not None and generation != self._generation:
            raise CatalogGenerationError("catalog generation is stale")


__all__ = [
    "DEFAULT_PAGE_SIZE",
    "FUZZY_SCORE_CUTOFF",
    "MAX_PAGE_SIZE",
    "SUPPORTED_EXTENSIONS",
    "CatalogCollisionError",
    "CatalogEntry",
    "CatalogError",
    "CatalogGenerationError",
    "CatalogLookupError",
    "CatalogPage",
    "CatalogRow",
    "CatalogService",
    "CatalogSnapshot",
    "canonical_path",
    "normalize_search_text",
    "normalized_canonical_path",
    "song_id_for_path",
]
