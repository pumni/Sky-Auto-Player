import time
from pathlib import Path

from sky_music.platform.win32 import inputs

SONG_DIR: Path = Path("songs")
SUPPORTED_EXTENSIONS: set[str] = {".json", ".skysheet", ".txt"}

_song_choices_cache: list[Path] = []
_song_choices_mtime_ns: int | None = None

def load_saved_theme() -> str:
    from sky_music.config import load_config
    return load_config().theme

def save_theme(theme_name: str) -> None:
    from sky_music.config import load_config, save_config
    cfg = load_config()
    cfg.theme = theme_name
    save_config(cfg)

def load_song_choices() -> list[Path]:
    if not SONG_DIR.exists():
        return []
    # Sort by the same key the picker displays (filename stem) so the visible list
    # is genuinely A→Z. Accents are normalized so Vietnamese titles collate naturally
    # and the order matches the search keys built from the stem.
    from sky_music.ui.picker_theme import remove_accents
    return sorted(
        [path for path in SONG_DIR.iterdir() if path.suffix.lower() in SUPPORTED_EXTENSIONS],
        key=lambda path: (remove_accents(path.stem).casefold(), path.stem.casefold()),
    )

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

    from sky_music.ui.picker_theme import remove_accents
    normalized = remove_accents(selection).casefold()
    
    exact_matches = [
        path for path in song_choices
        if remove_accents(path.stem).casefold() == normalized or remove_accents(path.name).casefold() == normalized
    ]
    if len(exact_matches) == 1:
        return exact_matches[0]

    partial_matches = [
        path for path in song_choices
        if normalized in remove_accents(path.stem).casefold() or normalized in remove_accents(path.name).casefold()
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
    inputs.sky = inputs.get_sky_window()
    if inputs.sky is None:
        print("Sky was not detected. Open Sky before playing a song.")
        return False
    inputs.focusWindow()
    if not inputs.is_sky_active():
        print("Sky is not focused yet. Bring Sky to the foreground, then press Enter to continue.")
        input()
    return True
