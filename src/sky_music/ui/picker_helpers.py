import time
from pathlib import Path

from sky_music.orchestration.catalog_service import (
    SUPPORTED_EXTENSIONS as CATALOG_SUPPORTED_EXTENSIONS,
)
from sky_music.orchestration.catalog_service import (
    CatalogService,
    normalize_search_text,
)

SONG_DIR: Path = Path("songs")
SUPPORTED_EXTENSIONS: set[str] = set(CATALOG_SUPPORTED_EXTENSIONS)

_song_choices_cache: list[Path] = []
_song_choices_mtime_ns: int | None = None

def load_saved_theme() -> str:
    from sky_music.config import load_config
    return load_config().theme

def save_theme(theme_name: str) -> None:
    # Legacy helper retained for CLI/TUI callers; persistence remains owned by
    # the shared application settings service.
    from sky_music.orchestration.settings_service import SettingsService

    SettingsService().set_theme(theme_name)

def load_song_choices() -> list[Path]:
    # Keep this legacy path-based helper for CLI/TUI callers, while the shared
    # service owns filtering and deterministic catalog ordering.
    return [entry.path for entry in CatalogService(SONG_DIR).scan_entries()]

def get_song_choices(force_refresh: bool = False) -> list[Path]:
    global _song_choices_cache, _song_choices_mtime_ns
    if not SONG_DIR.exists():
        _song_choices_cache = []
        _song_choices_mtime_ns = None
        return _song_choices_cache

    current_mtime_ns = SONG_DIR.stat().st_mtime_ns
    if force_refresh or _song_choices_mtime_ns != current_mtime_ns:
        _song_choices_cache = load_song_choices()
        _song_choices_mtime_ns = current_mtime_ns
    return _song_choices_cache

def resolve_song_selection(selection_text: str, song_choices: list[Path]) -> Path | None:
    selection = selection_text.strip()
    if not selection:
        return None

    if selection.isdigit():
        selected_index = int(selection) - 1
        if selected_index in range(len(song_choices)):
            return song_choices[selected_index]
        print(f"Invalid song number: {selection}")
        return None

    candidate_path = Path(selection)
    if candidate_path.exists() and candidate_path.suffix.lower() in SUPPORTED_EXTENSIONS:
        return candidate_path

    normalized = normalize_search_text(selection)
    
    exact_matches = [
        path for path in song_choices
        if normalize_search_text(path.stem) == normalized or normalize_search_text(path.name) == normalized
    ]
    if len(exact_matches) == 1:
        return exact_matches[0]

    partial_matches = [
        path for path in song_choices
        if normalized in normalize_search_text(path.stem) or normalized in normalize_search_text(path.name)
    ]
    if len(partial_matches) == 1:
        return partial_matches[0]

    if len(exact_matches) > 1 or len(partial_matches) > 1:
        matches = exact_matches or partial_matches
        print("Multiple songs matched. Be more specific:")
        for path in matches:
            print(f"  - {path.stem}")
        return None

    print(f"Song not found: {selection!r}")
    return None

def countdown_before_playback(seconds: int) -> None:
    for remaining in range(max(seconds, 0), 0, -1):
        print(f"\rPlaying song in {remaining}", end='', flush=True)
        time.sleep(1)
    if seconds > 0:
        print("\r" + " " * 32 + "\r", end='', flush=True)

def ensure_sky_ready() -> bool:
    from sky_music.platform.win32 import window_target

    window_target.reset_window_cache()
    if window_target.get_sky_window() is None:
        print("Sky was not detected. Open Sky before playing a song.")
        return False
    window_target.focus_window()
    if not window_target.is_sky_active():
        print("Sky is not focused yet. Bring Sky to the foreground, then press Enter to continue.")
        input()
    return True
