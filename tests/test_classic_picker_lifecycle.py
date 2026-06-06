from __future__ import annotations

import pytest
from pathlib import Path
from unittest.mock import MagicMock
import sky_music.ui.picker as picker_mod
import sky_music.ui.picker_background as pb_module
from sky_music.ui.picker import choose_song_interactively
from sky_music.ui.picker_background import ClassicPickerBackgroundHelper


def test_classic_picker_lifecycle_cleanup(monkeypatch) -> None:
    # 1. Mock song choices so it doesn't return early.
    monkeypatch.setattr(picker_mod, "get_song_choices", lambda force_refresh=False: [Path("songs/Alpha.json")])
    
    # 2. Mock prompt_toolkit Application
    class FakeApp:
        def __init__(self, *args, **kwargs) -> None:
            self.is_done = False
            self.loop = MagicMock()
        def run(self, pre_run=None) -> None:
            if pre_run:
                pre_run()
            return None
    monkeypatch.setattr(picker_mod, "Application", FakeApp)
    monkeypatch.setattr(picker_mod, "HAS_PROMPT_TOOLKIT", True)

    # 3. Track calls to ProcessPoolExecutor / ThreadPoolExecutor shutdown.
    shutdown_calls = []

    class MockProcessPoolExecutor:
        def __init__(self, max_workers: int) -> None:
            self.max_workers = max_workers
        def submit(self, fn, *args, **kwargs) -> MagicMock:
            return MagicMock()
        def shutdown(self, wait: bool = True, *, cancel_futures: bool = False) -> None:
            shutdown_calls.append(("process", wait, cancel_futures))

    class MockThreadPoolExecutor:
        def __init__(self, max_workers: int, thread_name_prefix: str | None = None) -> None:
            self.max_workers = max_workers
            self.thread_name_prefix = thread_name_prefix
        def submit(self, fn, *args, **kwargs) -> MagicMock:
            return MagicMock()
        def shutdown(self, wait: bool = True, *, cancel_futures: bool = False) -> None:
            shutdown_calls.append((self.thread_name_prefix or "thread", wait, cancel_futures))

    monkeypatch.setattr(pb_module, "ProcessPoolExecutor", MockProcessPoolExecutor)
    monkeypatch.setattr(pb_module, "ThreadPoolExecutor", MockThreadPoolExecutor)

    # Run choose_song_interactively
    choose_song_interactively()

    # Assertions:
    # We expect the process pool and the cache thread pool to be closed.
    # The cache thread pool name is "sky-picker-cache".
    # Both must be closed with wait=True.
    assert len(shutdown_calls) >= 2
    
    found_process = False
    found_cache = False
    for name, wait, cancel in shutdown_calls:
        assert wait is True
        assert cancel is True
        if name == "process":
            found_process = True
        elif name == "sky-picker-cache":
            found_cache = True
            
    assert found_process
    assert found_cache


def test_classic_picker_lifecycle_fallback_retirement(monkeypatch) -> None:
    shutdown_calls = []
    submitted_futures = []

    class MockProcessPoolExecutor:
        def __init__(self, max_workers: int) -> None:
            self.max_workers = max_workers
        def submit(self, fn, *args, **kwargs):
            from concurrent.futures import Future
            f = Future()
            submitted_futures.append(f)
            return f
        def shutdown(self, wait: bool = True, *, cancel_futures: bool = False) -> None:
            shutdown_calls.append(("process", wait, cancel_futures))

    class MockThreadPoolExecutor:
        def __init__(self, max_workers: int, thread_name_prefix: str | None = None) -> None:
            self.max_workers = max_workers
            self.thread_name_prefix = thread_name_prefix
        def submit(self, fn, *args, **kwargs):
            from concurrent.futures import Future
            f = Future()
            return f
        def shutdown(self, wait: bool = True, *, cancel_futures: bool = False) -> None:
            shutdown_calls.append((self.thread_name_prefix or "thread", wait, cancel_futures))

    monkeypatch.setattr(pb_module, "ProcessPoolExecutor", MockProcessPoolExecutor)
    monkeypatch.setattr(pb_module, "ThreadPoolExecutor", MockThreadPoolExecutor)

    helper = ClassicPickerBackgroundHelper()
    helper.setup_resources(lambda: None)

    # 1. Process metadata resource starts open.
    assert helper.metadata_resource.name == "classic-picker-metadata-process"
    assert helper.metadata_resource.snapshot().closed is False
    assert helper.metadata_resource.snapshot().state == "open"

    # 2. Fallback retire does not call ProcessPoolExecutor.shutdown(wait=False).
    helper.handle_fallback()
    assert helper.metadata_resource.name == "classic-picker-metadata-thread"
    assert not any(x[0] == "process" for x in shutdown_calls)

    # 3. Final cleanup calls real wait close.
    helper.close()
    assert ("process", True, True) in shutdown_calls
    assert ("sky-picker-meta", True, True) in shutdown_calls
    # Cache resource closes with final wait.
    assert ("sky-picker-cache", True, True) in shutdown_calls
