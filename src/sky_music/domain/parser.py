import json
from pathlib import Path
from typing import Any
from sky_music.domain.domain import Song, Note, NoteKey, Millis
from sky_music.domain.validation import SongParseError, SongValidationError, validate_song_structure
from sky_music.layouts import SKY_15_KEY_PROFILE

def _parse_note(idx: int, raw_note: Any, profile: Any, filepath_str: str) -> Note:
    """Helper to parse a single note object."""
    if not isinstance(raw_note, dict):
        raise SongValidationError(f"[{filepath_str}] Note index {idx} must be a JSON object, got {type(raw_note).__name__}")
        
    if "time" not in raw_note:
        raise SongValidationError(f"[{filepath_str}] Note index {idx} is missing 'time'")
    if "key" not in raw_note:
        raise SongValidationError(f"[{filepath_str}] Note index {idx} is missing 'key'")
        
    t = raw_note["time"]
    k = raw_note["key"]
    
    # Verify timestamp
    if not isinstance(t, int):
        try:
            t = int(t)
        except (ValueError, TypeError):
            raise SongValidationError(f"[{filepath_str}] Note index {idx} has invalid time: {t!r} (expected integer)")
            
    if t < 0:
        raise SongValidationError(f"[{filepath_str}] Note index {idx} has negative timestamp: {t}")
        
    # Verify note mapping
    if k not in profile.key_map:
        raise SongValidationError(
            f"[{filepath_str}] Note index {idx} has unmapped key: {k!r}. "
            f"Must be one of: {', '.join(sorted(profile.key_map.keys()))}"
        )
        
    return Note(time_ms=Millis(t), key=NoteKey(k))

def parse_song_file(filepath: Path, profile=None) -> Song:
    """Parses a song file (JSON or skysheet) strictly and validates all notes against the profile keymap."""
    if profile is None:
        profile = SKY_15_KEY_PROFILE
    filepath_str = filepath.name
    
    if not filepath.exists():
        raise SongParseError(f"File not found: {filepath}")
        
    try:
        with filepath.open('r', encoding='utf-8') as f:
            data = json.load(f)
    except json.JSONDecodeError as exc:
        raise SongParseError(f"[{filepath_str}] Invalid JSON formatting: {exc}")
        
    # Support lists at root (e.g. legacy structure was an array [song_dict])
    if isinstance(data, list):
        if not data:
            raise SongValidationError(f"[{filepath_str}] Empty song list")
        song_dict = data[0]
    else:
        song_dict = data
        
    validate_song_structure(song_dict, filepath_str)
    
    song_name = song_dict.get("name", filepath.stem)
    notes_list = [
        _parse_note(idx, raw_note, profile, filepath_str) 
        for idx, raw_note in enumerate(song_dict["songNotes"])
    ]
        
    # Sort stably by time
    notes_list.sort(key=lambda n: n.time_ms)
    
    return Song(name=song_name, notes=tuple(notes_list))
