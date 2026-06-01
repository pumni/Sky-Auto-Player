from dataclasses import dataclass
from pathlib import Path
from typing import Any

from sky_music.domain.domain import InstrumentProfile, Song
from sky_music.domain.parser import parse_song_file
from sky_music.domain.validation import SongParseError
from sky_music.layouts import SKY_15_KEY_PROFILE


@dataclass(frozen=True, slots=True)
class SongFileIdentity:
    path: Path
    mtime_ns: int
    size: int
    profile_id: int


class SongRepository:
    """Small file-backed cache for parsed songs."""

    def __init__(self) -> None:
        self._cache: dict[SongFileIdentity, Song] = {}
        self._identity_cache: dict[tuple[Path, int], SongFileIdentity] = {}

    def load(self, song_path: Path, profile: InstrumentProfile | None = None) -> Song:
        resolved_profile = profile or SKY_15_KEY_PROFILE
        identity = self._identity(song_path, resolved_profile)
        cached = self._cache.get(identity)
        if cached is not None:
            return cached

        # Drop stale entries for the same path/profile before caching the fresh parse.
        stale = [
            key for key in self._cache
            if key.path == identity.path and key.profile_id == identity.profile_id
        ]
        for key in stale:
            self._cache.pop(key, None)

        song = parse_song_file(song_path, resolved_profile)
        self._cache[identity] = song
        return song

    def clear(self) -> None:
        self._cache.clear()
        self._identity_cache.clear()

    def cache_key(self, song_path: Path, profile: InstrumentProfile | None = None) -> tuple[Any, ...]:
        identity = self._identity(song_path, profile or SKY_15_KEY_PROFILE)
        return (identity.path, identity.mtime_ns, identity.size, identity.profile_id)

    def _identity(self, song_path: Path, profile: InstrumentProfile) -> SongFileIdentity:
        cache_key = (song_path, id(profile))
        if cache_key in self._identity_cache:
            return self._identity_cache[cache_key]

        try:
            stat = song_path.stat()
        except FileNotFoundError as exc:
            raise SongParseError(f"File not found: {song_path}") from exc
        
        identity = SongFileIdentity(
            path=song_path,
            mtime_ns=stat.st_mtime_ns,
            size=stat.st_size,
            profile_id=id(profile),
        )
        self._identity_cache[cache_key] = identity
        return identity


_shared_song_repository = SongRepository()


def get_shared_song_repository() -> SongRepository:
    return _shared_song_repository
