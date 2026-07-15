"""PR-A through PR-E memory hygiene tests.

Covers all new tests specified in docs/2026-07_ram-memory-hygiene-plan.md that
are not already in test_post_play_memory_hygiene.py or
test_picker_metadata_optimizations.py.

These are reachable-object hygiene tests — not Task Manager RSS claims.
"""

from __future__ import annotations

import ast
import gc
import queue
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from sky_music.infrastructure.realtime import RealtimeProcessScope

# ---------------------------------------------------------------------------
# PR-A: test_hotkey_queue_drops_on_full
# ---------------------------------------------------------------------------


def _make_mock_controls(enabled: bool = True) -> MagicMock:
    controls = MagicMock()
    controls.enabled = enabled
    controls.quit = MagicMock(key_code=0x51, ctrl=False, alt=False, shift=False)
    return controls


class TestHotkeyQueueDropsOnFull:
    """Test that hotkey event_queue drops items on overflow instead of blocking."""

    def test_put_nowait_on_full_queue_does_not_raise(self) -> None:
        """queue.Full is silently caught in _hook_proc."""
        from sky_music.infrastructure.hotkey_hook import HotkeyHook

        hook = HotkeyHook(controls=_make_mock_controls())
        for _ in range(64):
            hook.event_queue.put_nowait("quit")

        with (
            patch.object(hook, "_hook_id", 1),
            patch("sky_music.infrastructure.hotkey_hook.user32.CallNextHookEx", return_value=0),
        ):
            kbd_struct = MagicMock()
            kbd_struct.vkCode = 0x51
            kbd_struct.dwExtraInfo = MagicMock()
            kbd_struct.dwExtraInfo[0] = 0

            lparam = MagicMock()
            lparam.contents = kbd_struct

            result = hook._hook_proc(0, 0x0100, lparam)

        assert result == 1
        assert hook.event_queue.qsize() == 64

    def test_queue_does_not_exceed_maxsize(self) -> None:
        """Event queue never exceeds maxsize even under synthetic flood."""
        from sky_music.infrastructure.hotkey_hook import HotkeyHook

        hook = HotkeyHook(controls=_make_mock_controls())
        assert hook.event_queue.maxsize == 64
        for i in range(64):
            hook.event_queue.put_nowait(f"event_{i}")
        with pytest.raises(queue.Full):
            hook.event_queue.put_nowait("overflow")
        assert hook.event_queue.qsize() == 64


# ---------------------------------------------------------------------------
# PR-A: test_hotkey_hook_stop_clears_proc_ref
# ---------------------------------------------------------------------------


class TestHotkeyHookClearsProcRef:
    """Verify ctypes callback and hook id are nulled after _run_pump exits."""

    def test_proc_ref_nulled_when_hook_fails(self) -> None:
        """When SetWindowsHookExW returns falsy, _run_pump nulls _hook_proc_ref immediately."""
        from sky_music.infrastructure.hotkey_hook import HotkeyHook

        hook = HotkeyHook(controls=_make_mock_controls())
        assert hook._hook_proc_ref is None

        with patch("sky_music.infrastructure.hotkey_hook.user32.SetWindowsHookExW", return_value=0):
            hook._run_pump()

        assert hook._hook_proc_ref is None
        assert hook._hook_id is None
        assert hook._thread_id > 0

    def test_proc_ref_nulled_after_unhook(self) -> None:
        from sky_music.infrastructure.hotkey_hook import HotkeyHook

        hook = HotkeyHook(controls=_make_mock_controls())
        fake_hhook = MagicMock()

        def set_hook(_type, _proc, _mod, _tid):
            hook._hook_id = fake_hhook
            hook._hook_proc_ref = _proc
            return fake_hhook

        def get_message(msg_ptr, _hwnd, _msg_min, _msg_max):
            return 0

        with (
            patch("sky_music.infrastructure.hotkey_hook.user32.SetWindowsHookExW", side_effect=set_hook),
            patch("sky_music.infrastructure.hotkey_hook.user32.GetMessageW", side_effect=get_message),
            patch("sky_music.infrastructure.hotkey_hook.user32.UnhookWindowsHookEx"),
        ):
            hook._run_pump()

        assert hook._hook_proc_ref is None
        assert hook._hook_id is None


# ---------------------------------------------------------------------------
# PR-A: test_hotkey_hook_stop_clears_thread
# ---------------------------------------------------------------------------


class TestHotkeyHookStop:
    """stop() nulls _thread but does not touch _hook_proc_ref directly."""

    def test_stop_nulls_thread(self) -> None:
        from sky_music.infrastructure.hotkey_hook import HotkeyHook

        hook = HotkeyHook(controls=_make_mock_controls())
        mock_thread = MagicMock()
        mock_thread.is_alive.return_value = True

        with (
            patch.object(hook, "_thread", mock_thread),
            patch.object(hook, "_thread_id", 12345),
            patch("sky_music.infrastructure.hotkey_hook.user32.PostThreadMessageW"),
        ):
            hook.stop()

        assert hook._thread is None

    def test_stop_does_not_null_proc_ref(self) -> None:
        from sky_music.infrastructure.hotkey_hook import HotkeyHook

        hook = HotkeyHook(controls=_make_mock_controls())
        hook._hook_proc_ref = "still_alive"
        hook._thread = MagicMock()
        hook._thread.is_alive.return_value = True
        hook._thread_id = 12345

        with patch("sky_music.infrastructure.hotkey_hook.user32.PostThreadMessageW"):
            hook.stop()

        assert hook._hook_proc_ref is not None


# ---------------------------------------------------------------------------
# PR-A: test_realtime_scope_restores_gc_on_del
# PR-A: test_realtime_scope_del_idempotent_after_exit
# ---------------------------------------------------------------------------


class TestRealtimeScopeGcFallback:
    """GC is restored when RealtimeProcessScope is abandoned or explicitly exited."""

    def test_scope_restores_gc_on_del(self) -> None:
        gc_enabled_before = gc.isenabled()
        scope = RealtimeProcessScope(enabled=True)
        scope.__enter__()
        assert not gc.isenabled()
        scope.__del__()
        assert gc.isenabled()
        if not gc_enabled_before:
            gc.disable()

    def test_del_idempotent_after_exit(self) -> None:
        gc_enabled_before = gc.isenabled()
        scope = RealtimeProcessScope(enabled=True)
        scope.__enter__()
        assert not gc.isenabled()
        scope.__exit__(None, None, None)
        assert gc.isenabled()
        scope.__del__()
        assert gc.isenabled()
        if not gc_enabled_before:
            gc.disable()

    def test_disabled_scope_does_not_touch_gc(self) -> None:
        was_enabled = gc.isenabled()
        scope = RealtimeProcessScope(enabled=False)
        scope.__enter__()
        assert gc.isenabled() == was_enabled
        scope.__exit__(None, None, None)
        assert gc.isenabled() == was_enabled
        scope.__del__()
        assert gc.isenabled() == was_enabled


# ---------------------------------------------------------------------------
# PR-B: Telemetry flush chunk tests
# ---------------------------------------------------------------------------


class TestTelemetryFlushChunk:
    """Incremental CSV flush bounds peak RAM during playback."""

    @staticmethod
    def _make_logger(tmp_path: Path, name: str, *, retain: bool = False):
        from sky_music.orchestration.telemetry import TelemetryLogger

        logger = TelemetryLogger(
            "flush_test",
            enabled=True,
            fps=20,
            min_hold_us=0,
            retain_records_after_save=retain,
        )
        logger.log_filepath = tmp_path / f"{name}.csv"
        return logger

    def test_flush_chunk_clears_records(self, tmp_path: Path) -> None:
        from sky_music.orchestration.telemetry import _TELEMETRY_FLUSH_CHUNK

        logger = self._make_logger(tmp_path, "flush_test")

        for _ in range(_TELEMETRY_FLUSH_CHUNK + 1):
            logger.record(0, "down", 0, 0, 0, 0, (), "test")

        assert len(logger.records) <= 1

    def test_retain_mode_preserves_records_past_flush(self, tmp_path: Path) -> None:
        from sky_music.orchestration.telemetry import _TELEMETRY_FLUSH_CHUNK

        logger = self._make_logger(tmp_path, "retain_test", retain=True)

        for _ in range(_TELEMETRY_FLUSH_CHUNK + 5):
            logger.record(0, "down", 0, 0, 0, 0, (), "test")

        assert len(logger.records) > 0

    def test_csv_written_incrementally(self, tmp_path: Path) -> None:
        from sky_music.orchestration.telemetry import _TELEMETRY_FLUSH_CHUNK

        csv_path = tmp_path / "incr_test.csv"
        logger = self._make_logger(tmp_path, "incr_test")

        for _ in range(_TELEMETRY_FLUSH_CHUNK):
            logger.record(0, "down", 0, 0, 0, 0, (), "test")
        logger.record(1, "up", 1000, 1000, 0, 0, (), "test")

        assert csv_path.exists()
        data = csv_path.read_text(encoding="utf-8")
        assert len(data.strip().splitlines()) >= _TELEMETRY_FLUSH_CHUNK + 1

    def test_save_closes_csv_file(self, tmp_path: Path) -> None:
        csv_path = tmp_path / "save_close_test.csv"
        logger = self._make_logger(tmp_path, "save_close_test")

        for _ in range(3):
            logger.record(0, "down", 0, 0, 0, 0, (), "test")

        logger.save()

        assert logger._csv_file is None
        assert logger._csv_writer is None
        assert csv_path.exists()
        summary_path = csv_path.with_suffix(".summary.json")
        assert summary_path.exists()


# ---------------------------------------------------------------------------
# PR-B: Summary double-hold fix
# ---------------------------------------------------------------------------


class TestTelemetrySummaryUsesRows:
    """get_summary() no longer walks self.records a second time."""

    def test_deferred_release_count_matches(self, tmp_path: Path) -> None:
        from sky_music.orchestration.telemetry import TelemetryLogger

        logger = TelemetryLogger(
            "summary_test",
            enabled=True,
            fps=20,
            min_hold_us=0,
            retain_records_after_save=True,
        )
        logger.log_filepath = tmp_path / "summary_test.csv"

        for i in range(10):
            deferred = 500 if i % 3 == 0 else 0
            logger.record(
                i, "down", 0, 0, 0, 0, (), "test",
                deferred_by_us=deferred,
            )

        summary = logger.get_summary()
        assert summary is not None
        assert summary["deferred_release_count"] == 4


# ---------------------------------------------------------------------------
# PR-C: release_song_data empties actions
# ---------------------------------------------------------------------------


class TestEngineReleaseSongData:
    """release_song_data() drops per-song schedule data."""

    def test_release_song_data_empties_actions(self) -> None:
        from sky_music.domain import Song
        from sky_music.domain.scheduler_types import (
            ActionKind,
            KeyAction,
            Microseconds,
            ScanCode,
        )
        from sky_music.infrastructure.backend import DryRunBackend
        from sky_music.orchestration.engine import PlaybackEngine

        song = Song(name="test", notes=())
        actions = (
            KeyAction(kind=ActionKind("down"), scan_codes=(ScanCode(42),), at_us=Microseconds(0), reason="t"),
        )
        engine = PlaybackEngine(
            song=song,
            actions=actions,
            backend=DryRunBackend(),
            telemetry_enabled=False,
        )

        assert len(engine.actions) > 0
        assert engine.runtime_schedule is not None

        engine.release_song_data()

        assert engine.actions == ()
        assert engine.runtime_schedule is None
        assert engine._runtime_coordinator is None


# ---------------------------------------------------------------------------
# PR-D: Picker cache LRU bounded
# ---------------------------------------------------------------------------


class TestMetadataCacheLru:
    """_metadata_cache is bounded by _METADATA_CACHE_MAX with LRU eviction."""

    def test_lru_evicts_oldest(self) -> None:
        from sky_music.ui.picker_metadata import (
            _METADATA_CACHE_MAX,
            SongUiMetadata,
            _cache_lock,
            _lru_set,
            _metadata_cache,
        )

        _metadata_cache.clear()
        try:
            dummy = SongUiMetadata(
                path=Path("d"), name="d", duration_seconds=1.0, note_count=1,
                max_polyphony=1, min_note_gap_ms=0.0, min_same_key_gap_ms=0.0,
                risk="low", recommended_profile="balanced", recommended_tempo_scale=1.0,
                warnings=(),
            )
            for i in range(_METADATA_CACHE_MAX + 1):
                key = ("evict_test", i)
                with _cache_lock:
                    _lru_set(_metadata_cache, key, dummy, maxsize=_METADATA_CACHE_MAX)

            oldest_key = ("evict_test", 0)
            newest_key = ("evict_test", _METADATA_CACHE_MAX)

            with _cache_lock:
                assert oldest_key not in _metadata_cache
                assert newest_key in _metadata_cache
                assert len(_metadata_cache) == _METADATA_CACHE_MAX
        finally:
            _metadata_cache.clear()

    def test_lru_promotes_on_get(self) -> None:
        from sky_music.ui.picker_metadata import (
            _METADATA_CACHE_MAX,
            SongUiMetadata,
            _cache_lock,
            _lru_get,
            _lru_set,
            _metadata_cache,
        )

        _metadata_cache.clear()
        try:
            dummy = SongUiMetadata(
                path=Path("d"), name="d", duration_seconds=1.0, note_count=1,
                max_polyphony=1, min_note_gap_ms=0.0, min_same_key_gap_ms=0.0,
                risk="low", recommended_profile="balanced", recommended_tempo_scale=1.0,
                warnings=(),
            )
            for i in range(_METADATA_CACHE_MAX):
                key = ("promote_test", i)
                with _cache_lock:
                    _lru_set(_metadata_cache, key, dummy, maxsize=_METADATA_CACHE_MAX)

            oldest_key = ("promote_test", 0)
            with _cache_lock:
                val = _lru_get(_metadata_cache, oldest_key)
            assert val is not None

            extra_key = ("promote_test", "extra")
            with _cache_lock:
                _lru_set(_metadata_cache, extra_key, dummy, maxsize=_METADATA_CACHE_MAX)

            with _cache_lock:
                assert oldest_key in _metadata_cache
                never_accessed = ("promote_test", 1)
                assert never_accessed not in _metadata_cache
        finally:
            _metadata_cache.clear()


class TestPkeyRamCacheNoFullClear:
    """_pkey_ram_cache uses LRU eviction, not full clear."""

    def test_no_full_clear_cliff(self) -> None:
        from sky_music.ui.picker_metadata import (
            _PKEY_RAM_CACHE_MAX,
            _lru_set,
            _pkey_ram_cache,
            _pkey_ram_lock,
        )

        _pkey_ram_cache.clear()
        try:
            for i in range(_PKEY_RAM_CACHE_MAX):
                key = ("pkey_cliff", i)
                with _pkey_ram_lock:
                    _lru_set(_pkey_ram_cache, key, f"hash_{i}", maxsize=_PKEY_RAM_CACHE_MAX)

            extra_key = ("pkey_cliff", "extra")
            with _pkey_ram_lock:
                _lru_set(_pkey_ram_cache, extra_key, "hash_extra", maxsize=_PKEY_RAM_CACHE_MAX)

            assert len(_pkey_ram_cache) == _PKEY_RAM_CACHE_MAX
            recent_key = ("pkey_cliff", _PKEY_RAM_CACHE_MAX - 1)
            with _pkey_ram_lock:
                assert recent_key in _pkey_ram_cache
                assert extra_key in _pkey_ram_cache
        finally:
            _pkey_ram_cache.clear()


class TestPersistentCacheBounded:
    """_persistent_cache is bounded by _PERSISTENT_CACHE_MAX."""

    def test_persistent_cache_bounded(self) -> None:
        from sky_music.ui.picker_metadata import (
            _PERSISTENT_CACHE_MAX,
            SongUiMetadata,
            _cache_lock,
            _lru_set,
            _persistent_cache,
        )

        _persistent_cache.clear()
        try:
            dummy = SongUiMetadata(
                path=Path("d"), name="d", duration_seconds=1.0, note_count=1,
                max_polyphony=1, min_note_gap_ms=0.0, min_same_key_gap_ms=0.0,
                risk="low", recommended_profile="balanced", recommended_tempo_scale=1.0,
                warnings=(),
            )
            for i in range(_PERSISTENT_CACHE_MAX + 10):
                key = f"pers_bound_{i}"
                with _cache_lock:
                    _lru_set(_persistent_cache, key, dummy, maxsize=_PERSISTENT_CACHE_MAX)

            assert len(_persistent_cache) == _PERSISTENT_CACHE_MAX
        finally:
            _persistent_cache.clear()


class TestClearMetadataCacheClearsThemeLru:
    """clear_metadata_cache() clears the theme normalized_index_map LRU."""

    def test_theme_lru_cache_cleared(self) -> None:
        from sky_music.ui.picker_metadata import clear_metadata_cache
        from sky_music.ui.picker_theme import normalized_index_map

        normalized_index_map("Hello World")
        normalized_index_map("Another Song")
        assert normalized_index_map.cache_info().currsize > 0

        clear_metadata_cache()

        assert normalized_index_map.cache_info().currsize == 0


# ---------------------------------------------------------------------------
# PR-E: DryRunBackend history bounded
# ---------------------------------------------------------------------------


class TestDryRunHistoryBounded:
    """DryRunBackend.history is capped at maxlen=10_000."""

    def test_history_capped_at_maxlen(self) -> None:
        from sky_music.infrastructure.backend import DryRunBackend

        backend = DryRunBackend()

        for i in range(10_005):
            backend.history.append(("test", (i,)))

        assert len(backend.history) == 10_000
        assert ("test", (0,)) not in backend.history
        assert ("test", (9999,)) in backend.history


# ---------------------------------------------------------------------------
# PR-E: BackgroundScope clears lists after close_all
# ---------------------------------------------------------------------------


class TestBackgroundScopeClearsLists:
    """close_all() clears _resources and _retired_resources."""

    @staticmethod
    def _make_dummy_resource(name: str) -> MagicMock:
        r = MagicMock()
        r.name = name
        r.phase = "test"
        return r

    def test_clears_resources_after_close_all(self) -> None:
        from sky_music.infrastructure.background import BackgroundScope

        scope = BackgroundScope(phase="test")
        r1 = self._make_dummy_resource("r1")
        r2 = self._make_dummy_resource("r2")
        scope.register(r1)
        scope.register(r2)
        scope.retire(r1)

        assert len(scope._resources) > 0
        assert len(scope._retired_resources) > 0

        scope.close_all(wait=True)

        assert len(scope._resources) == 0
        assert len(scope._retired_resources) == 0

    def test_close_all_clears_lists_even_with_errors(self) -> None:
        from sky_music.infrastructure.background import (
            BackgroundCleanupError,
            BackgroundScope,
        )

        scope = BackgroundScope(phase="test")
        r1 = self._make_dummy_resource("r1")
        r1.close.side_effect = RuntimeError("boom")
        scope.register(r1)

        with pytest.raises(BackgroundCleanupError):
            scope.close_all(wait=True)

        assert len(scope._resources) == 0
        assert len(scope._retired_resources) == 0


# ---------------------------------------------------------------------------
# PR-E: Debug log buffer auto-flush
# ---------------------------------------------------------------------------


class TestDebugLogBufferAutoFlush:
    """debug_log() flushes when DEBUG_LOG_BUFFER reaches 500 entries."""

    def test_buffer_auto_flush(self, monkeypatch: pytest.MonkeyPatch) -> None:
        import main

        buffer: list[str] = []
        monkeypatch.setattr(main, "PLAYBACK_DEBUG", True)
        monkeypatch.setattr(main, "DEBUG_LOG_BUFFER", buffer)
        monkeypatch.setattr(main, "DEBUG_LOG_PATH", Path("NUL"))
        monkeypatch.setattr(main, "DEBUG_START_PERF", 0.0)

        flush_called = False

        def fake_flush():
            nonlocal flush_called
            flush_called = True
            buffer.clear()

        monkeypatch.setattr(main, "flush_debug_log", fake_flush)

        buffer.extend(f"line_{i}" for i in range(500))
        main.debug_log("line_500")

        assert flush_called
        assert len(buffer) <= 1

    def test_buffer_does_not_flush_below_threshold(self, monkeypatch: pytest.MonkeyPatch) -> None:
        import main

        buffer: list[str] = []
        monkeypatch.setattr(main, "PLAYBACK_DEBUG", True)
        monkeypatch.setattr(main, "DEBUG_LOG_BUFFER", buffer)
        monkeypatch.setattr(main, "DEBUG_LOG_PATH", Path("NUL"))
        monkeypatch.setattr(main, "DEBUG_START_PERF", 0.0)

        flush_called = False

        def fake_flush():
            nonlocal flush_called
            flush_called = True

        monkeypatch.setattr(main, "flush_debug_log", fake_flush)

        buffer.extend(f"line_{i}" for i in range(10))
        main.debug_log("line_10")

        assert not flush_called
        assert len(buffer) == 11


# ---------------------------------------------------------------------------
# PR-A: _peek_persistent_metadata has only one lock block
# ---------------------------------------------------------------------------


class TestPeekPersistentMetadataSingleLock:
    """_peek_persistent_metadata has exactly one 'with _cache_lock' path."""

    def test_single_lock_path(self) -> None:
        from sky_music.ui import picker_metadata

        source = Path(picker_metadata.__file__).read_text(encoding="utf-8")
        tree = ast.parse(source)

        for node in ast.walk(tree):
            if isinstance(node, ast.FunctionDef) and node.name == "_peek_persistent_metadata":
                count = 0
                for child in ast.walk(node):
                    if isinstance(child, ast.With):
                        for item in child.items:
                            if (
                                isinstance(item.context_expr, ast.Name)
                                and item.context_expr.id == "_cache_lock"
                            ):
                                count += 1
                assert count == 1, (
                    f"_peek_persistent_metadata must contain exactly one "
                    f"'with _cache_lock' block, found {count}"
                )
                return
        pytest.fail("_peek_persistent_metadata function not found in source")


# ---------------------------------------------------------------------------
# PR-E: RUNTIME_STATE session cleared after release_song_data
# ---------------------------------------------------------------------------


class TestRuntimeStateSessionCleared:
    """RUNTIME_STATE.session is None after clear_session()."""

    def test_clear_session_nulls_session(self) -> None:
        from sky_music.domain.session_context import PlaybackSessionContext
        from sky_music.orchestration.runtime_session import RUNTIME_STATE

        session = PlaybackSessionContext.balanced()
        assert RUNTIME_STATE.session is None

        RUNTIME_STATE.session = session  # type: ignore[assignment]
        assert RUNTIME_STATE.session is not None

        RUNTIME_STATE.clear_session()
        assert RUNTIME_STATE.session is None
