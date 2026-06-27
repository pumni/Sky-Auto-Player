from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

import sky_music.ui.picker_metadata as pm
from sky_music.config import AppConfig, clear_config_cache
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.ui.picker_metadata import (
    _path_session_ram_cache,
    _persistent_cache_key,
    _pkey_ram_cache,
    clear_metadata_cache,
    get_cached_song_ui_metadata,
    peek_cached_song_ui_metadata,
    warm_persistent_metadata_cache,
    worker_process_warmup,
)


@pytest.fixture(autouse=True)
def _reset_caches():
    clear_config_cache()
    clear_metadata_cache(clear_persistent=False)
    # Clear internal global caches explicitly before and after each test
    _pkey_ram_cache.clear()
    _path_session_ram_cache.clear()
    pm._persistent_loaded = False
    yield
    _pkey_ram_cache.clear()
    _path_session_ram_cache.clear()
    pm._persistent_loaded = False


def test_pkey_ram_cache_saves_recomputation(tmp_path: Path) -> None:
    # Create a dummy song file in valid format
    song_path = tmp_path / "dummy_song.json"
    song_path.write_text('{"name": "Dummy", "songNotes": []}', encoding="utf-8")

    session = PlaybackSessionContext.balanced()
    cfg = AppConfig()

    # Verify cache is initially empty
    assert len(_pkey_ram_cache) == 0

    # Call _persistent_cache_key to populate cache
    key1 = _persistent_cache_key(song_path, session, cfg)
    assert len(_pkey_ram_cache) == 1
    assert key1 is not None

    # Call again and ensure it hits the cache
    # We patch _stable_file_identity to verify it is NOT called again
    with patch("sky_music.ui.picker_metadata._stable_file_identity") as mock_identity:
        key2 = _persistent_cache_key(song_path, session, cfg)
        assert key1 == key2
        mock_identity.assert_not_called()

    # Modify the file (change size or mtime)
    song_path.write_text('{"name": "Dummy - modified with longer text", "songNotes": []}', encoding="utf-8")
    
    # Verify a new call computes a new key and updates the cache (or overrides it)
    key3 = _persistent_cache_key(song_path, session, cfg)
    assert key3 != key1


def test_path_session_ram_cache_short_circuits_cache_key(tmp_path: Path) -> None:
    song_path = tmp_path / "dummy_song_2.json"
    song_path.write_text('{"name": "Dummy 2", "songNotes": []}', encoding="utf-8")

    session = PlaybackSessionContext.balanced()
    cfg = AppConfig()

    # Initially empty
    assert len(_path_session_ram_cache) == 0

    # Get UI metadata first to populate the cache & database
    meta = get_cached_song_ui_metadata(song_path, session, cfg)
    assert meta is not None

    # Peek to populate _path_session_ram_cache
    res1 = peek_cached_song_ui_metadata(song_path, session, cfg)
    assert res1 is not None
    assert len(_path_session_ram_cache) == 1

    # Subsequent peeks should bypass _song_repository.cache_key()
    with patch("sky_music.ui.picker_metadata._song_repository.cache_key") as mock_cache_key:
        res2 = peek_cached_song_ui_metadata(song_path, session, cfg)
        assert res2 is not None
        mock_cache_key.assert_not_called()


def test_warm_persistent_metadata_cache_adaptive_limit(tmp_path: Path) -> None:
    # Verify that warm_persistent_metadata_cache handles None and adaptive limits
    # We can mock _connect_persistent_cache and check the limit in SQL execution
    with patch("sky_music.ui.picker_metadata._connect_persistent_cache") as mock_connect:
        mock_conn = MagicMock()
        mock_connect.return_value = mock_conn
        mock_conn.execute.return_value.fetchall.return_value = []

        # 1. Default fallback: 500
        pm._persistent_loaded = False
        warm_persistent_metadata_cache(limit=None, song_paths=None)
        # Check the execute call
        mock_conn.execute.assert_called()
        args, _kwargs = mock_conn.execute.call_args
        _sql, params = args
        assert params == (500,)

        # 2. Explicit limit
        pm._persistent_loaded = False
        warm_persistent_metadata_cache(limit=123, song_paths=None)
        args, params = mock_conn.execute.call_args_list[-1][0]
        assert params == (123,)

        # 3. Adaptive limit based on song_paths
        pm._persistent_loaded = False
        fake_paths = [Path(f"song_{i}.json") for i in range(100)]
        warm_persistent_metadata_cache(limit=None, song_paths=fake_paths)
        args, params = mock_conn.execute.call_args_list[-1][0]
        # max(500, len(song_paths) * 8) -> max(500, 800) = 800
        assert params == (800,)


def test_worker_process_warmup_returns_true() -> None:
    assert worker_process_warmup() is True


def test_compute_raw_song_ui_metadata_correctness(tmp_path: Path) -> None:
    import json

    from sky_music.ui.picker_metadata import compute_raw_song_ui_metadata

    # Test cases mapping input notes to expected outputs
    test_scenarios = [
        # 1. Simple sequential notes
        (
            [(0, "Key0"), (200, "Key1"), (400, "Key2")],
            {
                "note_count": 3,
                "duration_seconds": 0.4,
                "min_note_gap_ms": 200.0,
                "min_same_key_gap_ms": 0.0,
                "max_chord_size": 1,
                "chords_count": 0,
                "average_notes_per_second": 7.5,
                "peak_notes_per_second_1s": 3.0,
            }
        ),
        # 2. Chord (simultaneous notes)
        (
            [(0, "Key0"), (0, "Key1"), (500, "Key2"), (500, "Key3"), (500, "Key4")],
            {
                "note_count": 5,
                "duration_seconds": 0.5,
                "min_note_gap_ms": 500.0,
                "min_same_key_gap_ms": 0.0,
                "max_chord_size": 3,
                "chords_count": 2,
                "average_notes_per_second": 10.0,
                "peak_notes_per_second_1s": 5.0,
            }
        ),
        # 3. Same key repeat gap
        (
            [(0, "Key0"), (100, "Key0"), (300, "Key0")],
            {
                "note_count": 3,
                "duration_seconds": 0.3,
                "min_note_gap_ms": 100.0,
                "min_same_key_gap_ms": 100.0,
                "max_chord_size": 1,
                "chords_count": 0,
                "average_notes_per_second": 10.0,
                "peak_notes_per_second_1s": 3.0,
            }
        ),
        # 4. Dense notes in 1s window (sliding window peak test)
        (
            [(0, "Key0"), (100, "Key1"), (200, "Key2"), (900, "Key3"), (1000, "Key4"), (1001, "Key0")],
            {
                "note_count": 6,
                "duration_seconds": 1.001,
                "min_note_gap_ms": 1.0,
                "min_same_key_gap_ms": 1001.0,
                "max_chord_size": 1,
                "chords_count": 0,
                "average_notes_per_second": 6 / 1.001,
                "peak_notes_per_second_1s": 5.0,
            }
        )
    ]

    for idx, (note_list, expected) in enumerate(test_scenarios):
        song_data = {
            "name": f"Test Song {idx}",
            "songNotes": [{"time": t, "key": k} for t, k in note_list]
        }
        song_path = tmp_path / f"test_song_{idx}.json"
        song_path.write_text(json.dumps(song_data), encoding="utf-8")

        meta = compute_raw_song_ui_metadata(song_path)
        assert meta.note_count == expected["note_count"]
        assert pytest.approx(meta.duration_seconds) == expected["duration_seconds"]
        assert pytest.approx(meta.min_note_gap_ms) == expected["min_note_gap_ms"]
        assert pytest.approx(meta.min_same_key_gap_ms) == expected["min_same_key_gap_ms"]
        assert meta.max_chord_size == expected["max_chord_size"]
        assert meta.chords_count == expected["chords_count"]
        assert pytest.approx(meta.average_notes_per_second) == expected["average_notes_per_second"]
        assert pytest.approx(meta.peak_notes_per_second_1s) == expected["peak_notes_per_second_1s"]


def test_background_threads_populate_path_session_ram_cache(tmp_path: Path) -> None:
    from sky_music.ui.picker_metadata import (
        _path_session_ram_cache,
        hydrate_persistent_metadata_for_paths,
        populate_raw_song_ui_metadata_for_paths,
    )
    song_path = tmp_path / "dummy_song_background.json"
    song_path.write_text('{"name": "Background test", "songNotes": []}', encoding="utf-8")

    session = PlaybackSessionContext.balanced()
    cfg = AppConfig()

    assert len(_path_session_ram_cache) == 0

    # 1. Test raw metadata population seeds the cache
    populate_raw_song_ui_metadata_for_paths([song_path], session, cfg)
    assert len(_path_session_ram_cache) == 1

    # Clear RAM cache
    _path_session_ram_cache.clear()

    # 2. Test persistent metadata hydration seeds the cache
    # First, let's store it as persistent metadata
    from sky_music.ui.picker_metadata import (
        SongUiMetadata,
        store_computed_song_ui_metadata_payloads,
    )
    meta = SongUiMetadata(
        path=song_path,
        name="Background test",
        duration_seconds=10.0,
        note_count=5,
        max_polyphony=1,
        min_note_gap_ms=100.0,
        min_same_key_gap_ms=100.0,
        risk="low",
        recommended_profile="balanced",
        recommended_tempo_scale=1.0,
        warnings=(),
        analyzed=True
    )
    from sky_music.ui.picker_metadata import _metadata_to_payload
    store_computed_song_ui_metadata_payloads([_metadata_to_payload(meta)], session, cfg)

    # Invalidate persistent cache RAM layer to force hit database on hydration
    from sky_music.ui.picker_metadata import _persistent_cache
    _persistent_cache.clear()
    _path_session_ram_cache.clear()

    # Hydrate from SQLite and check if it seeds path+session cache
    hydrate_persistent_metadata_for_paths([song_path], session, cfg)
    assert len(_path_session_ram_cache) == 1
