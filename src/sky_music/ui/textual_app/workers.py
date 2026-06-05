"""Background metadata coordination for the Textual picker."""

from __future__ import annotations

from concurrent.futures import Future, ThreadPoolExecutor
from pathlib import Path
from typing import Any, Protocol

from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.ui.picker_metadata import (
    compute_song_ui_metadata_payloads,
    hydrate_and_fill_raw_metadata,
    peek_cached_song_ui_metadata,
    session_to_worker_payload,
    store_computed_song_ui_metadata_payloads,
    warm_persistent_metadata_cache,
)


class MetadataApp(Protocol):
    def run_worker(
        self,
        work: Any,
        *,
        name: str | None = "",
        group: str = "default",
        description: str = "",
        exit_on_error: bool = True,
        start: bool = True,
        exclusive: bool = False,
        thread: bool = False,
    ) -> Any: ...

    def call_from_thread(self, callback: Any, *args: Any, **kwargs: Any) -> Any: ...

    def refresh_metadata_rows(self) -> None: ...


class MetadataCoordinator:
    """Hydrate raw metadata and compute risk without touching UI off-thread.

    This uses a single-threaded background ThreadPoolExecutor to sequentially
    process metadata requests. It avoids spawning child processes (which is slow
    on Windows and has compatibility issues with PyInstaller) and uses debouncing
    to prevent queue buildup.
    """

    def __init__(
        self,
        app: MetadataApp,
        session: PlaybackSessionContext,
        cfg: AppConfig | None,
    ) -> None:
        self._app = app
        self._session = session
        self._cfg = cfg
        self._closed = False
        
        # Dedicated sequential coordinator executor
        self._coord_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-metadata-coord")
        self._coord_future: Future[None] | None = None

    def refresh(self, paths: list[Path]) -> None:
        """Begin progressive background metadata warming, hydration, and analysis for all paths."""
        if self._closed:
            return
        all_paths = list(paths)
        if not all_paths:
            return

        # Cancel the pending job if it has not started yet.
        # This keeps the queue length bounded to at most 1 pending job.
        if self._coord_future is not None and not self._coord_future.done():
            self._coord_future.cancel()

        # Submit the new list of paths to be processed in the background thread.
        self._coord_future = self._coord_executor.submit(self._warm_and_process_all_paths, all_paths)

    def close(self) -> None:
        self._closed = True
        if self._coord_executor is not None:
            self._coord_executor.shutdown(wait=False, cancel_futures=True)

    def _refresh_ui_from_thread(self) -> None:
        if self._closed:
            return
        loop = getattr(self._app, "_loop", None)
        if loop is not None and loop.is_closed():
            self._closed = True
            return
        try:
            self._app.call_from_thread(self._app.refresh_metadata_rows)
        except RuntimeError:
            self._closed = True

    def _warm_and_process_all_paths(self, paths: list[Path]) -> None:
        if self._closed:
            return

        # Step 1: Warm SQLite cache for all paths in a single efficient operation
        try:
            warm_persistent_metadata_cache(song_paths=paths)
            self._refresh_ui_from_thread()
        except Exception:
            pass

        if self._closed:
            return

        # Step 2: Hydrate cache from SQLite for specific paths and populate raw note stats
        try:
            changed = hydrate_and_fill_raw_metadata(paths, self._session, self._cfg)
            if changed:
                self._refresh_ui_from_thread()
        except Exception:
            pass

        if self._closed:
            return

        # Step 3: Find any paths that still need full scheduler analysis
        pending = []
        for path in paths:
            if self._closed:
                return
            meta = peek_cached_song_ui_metadata(path, self._session, self._cfg)
            if meta is None or not meta.analyzed:
                pending.append(path)

        if not pending:
            return

        # Step 4: Run the heavier scheduler analysis progressively in batches of 10
        # This avoids blocking and provides incremental UI updates for large libraries
        batch_size = 10
        session_payload = session_to_worker_payload(self._session)
        
        for i in range(0, len(pending), batch_size):
            if self._closed:
                return
            batch = pending[i : i + batch_size]
            try:
                payloads = compute_song_ui_metadata_payloads(
                    [str(path) for path in batch],
                    session_payload,
                    self._cfg,
                )
                if self._closed:
                    return
                changed_risk = store_computed_song_ui_metadata_payloads(payloads, self._session, self._cfg)
                if changed_risk:
                    self._refresh_ui_from_thread()
            except Exception:
                continue
