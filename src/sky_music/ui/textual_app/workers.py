"""Background metadata coordination for the Textual picker."""

from __future__ import annotations

from concurrent.futures import Future, ProcessPoolExecutor, ThreadPoolExecutor
from pathlib import Path
import sys
from typing import Any, Protocol

from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.ui.picker_metadata import (
    compute_song_ui_metadata_payloads,
    hydrate_and_fill_raw_metadata,
    peek_cached_song_ui_metadata,
    session_to_worker_payload,
    store_computed_song_ui_metadata_payloads,
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
    """Hydrate raw metadata and compute risk without touching UI off-thread."""

    def __init__(
        self,
        app: MetadataApp,
        session: PlaybackSessionContext,
        cfg: AppConfig | None,
    ) -> None:
        self._app = app
        self._session = session
        self._cfg = cfg
        self._process_executor: ProcessPoolExecutor | None = None
        self._thread_executor: ThreadPoolExecutor | None = None
        self._risk_future: Future[list[dict[str, Any]]] | None = None
        self._closed = False
        if self._can_spawn_process_worker():
            try:
                self._process_executor = ProcessPoolExecutor(max_workers=1)
                self._risk_executor: ProcessPoolExecutor | ThreadPoolExecutor = self._process_executor
            except Exception:
                self._thread_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-textual-risk")
                self._risk_executor = self._thread_executor
        else:
            self._thread_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-textual-risk")
            self._risk_executor = self._thread_executor

    @staticmethod
    def _can_spawn_process_worker() -> bool:
        try:
            return Path(sys.argv[0]).exists()
        except Exception:
            return False

    def refresh(self, paths: list[Path]) -> None:
        visible_paths = list(paths)
        if not visible_paths:
            return
        self._app.run_worker(
            lambda: self._hydrate_then_analyze(visible_paths),
            name="metadata",
            group="metadata",
            exclusive=True,
            thread=True,
            exit_on_error=False,
        )

    def close(self) -> None:
        self._closed = True
        if self._process_executor is not None:
            self._process_executor.shutdown(wait=False, cancel_futures=True)
        if self._thread_executor is not None:
            self._thread_executor.shutdown(wait=False, cancel_futures=True)

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

    def _hydrate_then_analyze(self, paths: list[Path]) -> None:
        changed = hydrate_and_fill_raw_metadata(paths, self._session, self._cfg)
        if changed:
            self._refresh_ui_from_thread()

        pending = [
            path
            for path in paths
            if (meta := peek_cached_song_ui_metadata(path, self._session, self._cfg)) is None
            or not meta.analyzed
        ]
        if not pending:
            return

        if self._risk_future is not None and not self._risk_future.done():
            self._risk_future.cancel()
        future = self._risk_executor.submit(
            compute_song_ui_metadata_payloads,
            [str(path) for path in pending],
            session_to_worker_payload(self._session),
            self._cfg,
        )
        self._risk_future = future
        future.add_done_callback(self._store_risk_result)

    def _store_risk_result(self, future: Future[list[dict[str, Any]]]) -> None:
        try:
            payloads = future.result()
            changed = store_computed_song_ui_metadata_payloads(payloads, self._session, self._cfg)
        except Exception:
            return
        if changed:
            self._refresh_ui_from_thread()
