"""Background metadata coordination for the Textual picker."""

from __future__ import annotations

from concurrent.futures import Future, ThreadPoolExecutor
from pathlib import Path
from typing import Any, Protocol

from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.infrastructure.background import ResourceState, WorkerSnapshot
from sky_music.ui.picker_metadata import (
    compute_song_ui_metadata_payloads,
    hydrate_and_fill_raw_metadata,
    hydrate_persistent_metadata_for_paths,
    peek_cached_song_ui_metadata,
    session_to_worker_payload,
    store_computed_song_ui_metadata_payloads,
)


class MetadataHandle(Protocol):
    """Interface for metadata coordinator used by UI code."""
    @property
    def name(self) -> str: ...
    @property
    def phase(self) -> str: ...
    def refresh(self, paths: list[Path]) -> None: ...
    def cancel(self) -> None: ...
    def close(self, *, wait: bool) -> None: ...
    def snapshot(self) -> WorkerSnapshot: ...


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
    def get_metadata_priority_paths(self) -> list[Path]: ...


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
        self._state = ResourceState.OPEN
        self._last_error: str | None = None
        self._latest_request_id = 0
        
        # Dedicated sequential coordinator executor
        self._coord_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-metadata-coord")
        self._coord_future: Future[None] | None = None

    @property
    def name(self) -> str:
        return "textual-picker-metadata"

    @property
    def phase(self) -> str:
        return "picker"

    def _should_stop(self, request_id: int | None = None) -> bool:
        if self._state is not ResourceState.OPEN:
            return True
        return bool(request_id is not None and request_id != self._latest_request_id)

    def refresh(self, paths: list[Path]) -> None:
        """Begin progressive background metadata warming, hydration, and analysis for all paths."""
        if self._should_stop():
            return
        all_paths = list(paths)
        if not all_paths:
            return

        self._latest_request_id += 1
        request_id = self._latest_request_id

        # Cancel the pending job if it has not started yet.
        # This keeps the queue length bounded to at most 1 pending job.
        if self._coord_future is not None and not self._coord_future.done():
            self._coord_future.cancel()

        # Submit the new list of paths to be processed in the background thread.
        self._coord_future = self._coord_executor.submit(self._warm_and_process_all_paths, all_paths, request_id)

    def cancel(self) -> None:
        if self._state == ResourceState.OPEN:
            self._state = ResourceState.CLOSING
        if self._coord_future is not None and not self._coord_future.done():
            self._coord_future.cancel()

    def close(self, *, wait: bool = False) -> None:
        if self._state == ResourceState.CLOSED:
            return
        if self._state == ResourceState.OPEN:
            self._state = ResourceState.CLOSING
        if self._coord_future is not None and not self._coord_future.done():
            self._coord_future.cancel()

        if not wait:
            # Do not call executor shutdown with wait=False to avoid Python 3.14 manager-thread trap.
            # Simply request cancellation and transition to CLOSING.
            return

        if self._coord_executor is not None:
            try:
                try:
                    self._coord_executor.shutdown(wait=wait, cancel_futures=True)
                except TypeError:
                    self._coord_executor.shutdown(wait=wait)
                self._state = ResourceState.CLOSED
            except Exception as exc:
                self._state = ResourceState.FAILED
                self._last_error = str(exc)
                raise exc

    def snapshot(self) -> WorkerSnapshot:
        pending = 0
        running = 0
        if self._coord_future is not None and not self._coord_future.done():
            if self._coord_future.running():
                running = 1
            else:
                pending = 1
        return WorkerSnapshot(
            name=self.name,
            phase=self.phase,
            closed=self._state == ResourceState.CLOSED,
            pending_count=pending,
            running_count=running,
            state=self._state.value,
            last_error=self._last_error,
        )

    def _refresh_ui_from_thread(self, request_id: int | None = None) -> None:
        if self._should_stop(request_id):
            return
        loop = getattr(self._app, "_loop", None)
        if loop is not None and loop.is_closed():
            self._state = ResourceState.CLOSED
            return
        try:
            self._app.call_from_thread(self._app.refresh_metadata_rows)
        except RuntimeError:
            self._state = ResourceState.CLOSED

    def _warm_and_process_all_paths(self, paths: list[Path], request_id: int) -> None:
        if self._should_stop(request_id):
            return

        priority_paths: list[Path] = []
        try:
            get_priority = getattr(self._app, "get_metadata_priority_paths", None)
            if callable(get_priority):
                priority_paths = get_priority()
        except Exception:
            pass

        ordered_paths = []
        seen = set()
        for p in priority_paths:
            if p in paths and p not in seen:
                ordered_paths.append(p)
                seen.add(p)
        for p in paths:
            if p not in seen:
                ordered_paths.append(p)
                seen.add(p)
        paths = ordered_paths

        # Step 1: Hydrate SQLite cache in small batches so shutdown/playback handoff can stop
        # promptly instead of waiting for one large library-wide cache operation.
        warm_batch_size = 25
        try:
            for i in range(0, len(paths), warm_batch_size):
                if self._should_stop(request_id):
                    return
                changed = hydrate_persistent_metadata_for_paths(
                    paths[i : i + warm_batch_size],
                    self._session,
                    self._cfg,
                )
                if changed:
                    self._refresh_ui_from_thread(request_id)
        except Exception as exc:
            self._last_error = f"Step 1 (warm) failed: {exc}"

        if self._should_stop(request_id):
            return

        # Step 2: Hydrate cache from SQLite for specific paths and populate raw note stats
        try:
            changed = hydrate_and_fill_raw_metadata(paths, self._session, self._cfg)
            if changed:
                self._refresh_ui_from_thread(request_id)
        except Exception as exc:
            self._last_error = f"Step 2 (hydrate) failed: {exc}"

        if self._should_stop(request_id):
            return

        # Step 3: Find any paths that still need full scheduler analysis
        pending = []
        for path in paths:
            if self._should_stop(request_id):
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
            if self._should_stop(request_id):
                return
            batch = pending[i : i + batch_size]
            try:
                payloads = compute_song_ui_metadata_payloads(
                    [str(path) for path in batch],
                    session_payload,
                    self._cfg,
                )
                if self._should_stop(request_id):
                    return
                changed_risk = store_computed_song_ui_metadata_payloads(payloads, self._session, self._cfg)
                if changed_risk:
                    self._refresh_ui_from_thread(request_id)
            except Exception as exc:
                self._last_error = f"Step 4 (compute) failed for batch {i}: {exc}"
                continue
