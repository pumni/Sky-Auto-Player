import json

import pytest

from sky_music.domain.parser import parse_song_file
from sky_music.domain.validation import SongValidationError

@pytest.fixture
def tmp_song_file(tmp_path):
    """Utility fixture to write temporary song files."""
    def _create(data):
        file = tmp_path / "test_song.json"
        with file.open('w', encoding='utf-8') as f:
            json.dump(data, f)
        return file
    return _create

def test_valid_song_parses(tmp_song_file):
    song_data = {
        "name": "Beautiful Song",
        "songNotes": [
            {"time": 0, "key": "Key0"},
            {"time": 200, "key": "Key5"},
        ]
    }
    file = tmp_song_file(song_data)
    song = parse_song_file(file)
    
    assert song.name == "Beautiful Song"
    assert len(song.notes) == 2
    assert song.notes[0].time_ms == 0
    assert song.notes[0].key == "Key0"
    assert song.notes[1].time_ms == 200
    assert song.notes[1].key == "Key5"

def test_unknown_key_fails(tmp_song_file):
    song_data = {
        "name": "Invalid Key Song",
        "songNotes": [
            {"time": 0, "key": "InvalidKeyName"},
        ]
    }
    file = tmp_song_file(song_data)
    with pytest.raises(SongValidationError, match="unmapped key: 'InvalidKeyName'"):
        parse_song_file(file)

def test_missing_song_notes_fails(tmp_song_file):
    song_data = {
        "name": "No Notes Song"
    }
    file = tmp_song_file(song_data)
    with pytest.raises(SongValidationError, match="Missing required key: 'songNotes'"):
        parse_song_file(file)

def test_negative_time_fails(tmp_song_file):
    song_data = {
        "name": "Negative Time Song",
        "songNotes": [
            {"time": -100, "key": "Key0"}
        ]
    }
    file = tmp_song_file(song_data)
    with pytest.raises(SongValidationError, match="negative timestamp"):
        parse_song_file(file)

def test_unordered_notes_are_sorted(tmp_song_file):
    song_data = {
        "name": "Unordered Song",
        "songNotes": [
            {"time": 500, "key": "Key1"},
            {"time": 100, "key": "Key0"},
        ]
    }
    file = tmp_song_file(song_data)
    song = parse_song_file(file)
    
    assert len(song.notes) == 2
    assert song.notes[0].time_ms == 100
    assert song.notes[0].key == "Key0"
    assert song.notes[1].time_ms == 500
    assert song.notes[1].key == "Key1"

def test_chord_notes_with_same_timestamp_preserved(tmp_song_file):
    song_data = {
        "name": "Chord Song",
        "songNotes": [
            {"time": 100, "key": "Key0"},
            {"time": 100, "key": "Key1"},
        ]
    }
    file = tmp_song_file(song_data)
    song = parse_song_file(file)
    
    assert len(song.notes) == 2
    assert song.notes[0].time_ms == 100
    assert song.notes[1].time_ms == 100

def test_third_instrument_legacy_key_parses(tmp_song_file):
    song_data = {
        "name": "Third Instrument",
        "songNotes": [
            {"time": 100, "key": "3Key5"},
        ]
    }
    file = tmp_song_file(song_data)
    song = parse_song_file(file)

    assert song.notes[0].key == "3Key5"


