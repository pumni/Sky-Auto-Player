"""Application service adapter and bounded stdin/stdout server for Desktop Core."""

from __future__ import annotations

import os
import sys
import threading
import traceback
from collections.abc import Mapping
from contextlib import suppress
from dataclasses import asdict
from queue import Empty, Queue
from typing import Any

from sky_music import __version__
from sky_music.config import VALID_FPS
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.infrastructure.desktop_ipc.protocol import (
    DESKTOP_PROTOCOL_VERSION,
    ProtocolError,
    bounded_text,
    event,
    iter_bounded_frames,
    parse_request_frame,
    response_error,
    response_ok,
    write_frame,
)
from sky_music.orchestration.catalog_service import (
    CatalogError,
    CatalogGenerationError,
    CatalogLookupError,
    CatalogService,
)
from sky_music.orchestration.desktop_models import (
    BootstrapDto,
    NativeBuildDto,
    PlaybackConfigDto,
    PlaybackOptionSetsDto,
    PlaybackRecommendationDto,
    RiskSummaryDto,
    SongDetailDto,
    UpdatePreferencesDto,
)
from sky_music.orchestration.desktop_playback import (
    DesktopPlaybackError,
    DesktopPlaybackService,
)
from sky_music.orchestration.native_admission import RustBuildInfo
from sky_music.orchestration.settings_service import (
    HOLD_FRAME_OPTIONS,
    TEMPO_SCALE_OPTIONS,
    SettingsService,
)
from sky_music.orchestration.song_metadata_service import get_song_ui_metadata

MAX_OFFSET = 1_000_000_000
MAX_VIEWPORT_SPAN = 2_000
MAX_BUFFERED_EVENTS = 128
PATCH_FIELDS = frozenset({"theme", "telemetry_enabled", "verbose_hud", "playback_defaults"})
PLAYBACK_PATCH_FIELDS = frozenset({"hold_frames", "tempo_scale", "fps"})
SUPPORTED_METHODS = frozenset(
    {
        "app.bootstrap",
        "app.shutdown",
        "catalog.search",
        "catalog.detail",
        "catalog.reload",
        "catalog.set_viewport",
        "settings.get",
        "settings.patch",
        "playback.prepare",
        "playback.start",
        "playback.stop",
        "playback.pause",
        "playback.resume",
        "playback.skip",
    }
)


class CoreRequestError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


def parent_process_alive(pid: int) -> bool:
    """Return whether the supervising process still exists without touching it."""
    if type(pid) is not int or pid <= 0:
        return False
    if sys.platform == "win32":
        from sky_music.platform.win32.process_state import query_process_image

        return query_process_image(pid).alive
    try:
        os.kill(pid, 0)
    except PermissionError:
        return True
    except (OSError, ProcessLookupError):
        return False
    return True


def _object_params(raw_params: object, allowed: frozenset[str]) -> dict[str, object]:
    params = raw_params
    if not isinstance(params, dict):
        raise CoreRequestError("invalid_params", "params must be an object")
    unknown = set(params) - allowed
    if unknown:
        raise CoreRequestError("invalid_params", f"unknown params: {', '.join(sorted(unknown))}")
    return params


def _required_text(params: Mapping[str, object], name: str, *, max_bytes: int = 1024) -> str:
    value = params.get(name)
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > max_bytes:
        raise CoreRequestError("invalid_params", f"{name} must be a bounded non-empty string")
    return value


def _optional_generation(params: Mapping[str, object]) -> int | None:
    value = params.get("generation")
    if value is None:
        return None
    if type(value) is not int or value < 0:
        raise CoreRequestError("invalid_params", "generation must be a non-negative integer")
    return value


def _required_int(params: Mapping[str, object], name: str, *, minimum: int = 0) -> int:
    value = params.get(name)
    if type(value) is not int or value < minimum:
        raise CoreRequestError("invalid_params", f"{name} must be an integer >= {minimum}")
    return value


def _native_build_dto(info: RustBuildInfo) -> NativeBuildDto:
    return NativeBuildDto(
        native_build_commit=info.native_build_commit,
        native_version=info.native_version,
        schema_version=info.schema_version,
        native_abi=info.native_abi,
        rustc_version=info.rustc_version,
        win32_backend=info.win32_backend,
    )


def _native_build_dict(info: RustBuildInfo) -> dict[str, object]:
    return asdict(_native_build_dto(info))


def _settings_dict(service: SettingsService) -> dict[str, object]:
    settings = service.snapshot()
    return {
        "theme": settings.theme,
        "ui_background_mode": settings.ui_background_mode,
        "playback_defaults": asdict(
            PlaybackConfigDto(
                hold_frames=settings.default_hold_frames,
                tempo_scale=settings.default_tempo_scale,
                fps=settings.game_fps,
                dry_run=False,
            )
        ),
        "telemetry_enabled": settings.telemetry_enabled,
        "verbose_hud": settings.verbose_hud,
        "update_preferences": asdict(
            UpdatePreferencesDto(
                auto_check=settings.update_preferences.auto_check,
                channel=settings.update_preferences.channel,  # type: ignore[arg-type]
                skip_version=settings.update_preferences.skip_version,
            )
        ),
    }


def _bootstrap_dict(
    dto: BootstrapDto,
) -> dict[str, object]:
    return asdict(dto)


class DesktopCoreServer:
    """Single-threaded request dispatcher with a bounded I/O shell."""

    def __init__(
        self,
        *,
        settings_service: SettingsService,
        catalog_service: CatalogService,
        native_build_info: RustBuildInfo,
        app_version: str = __version__,
        parent_pid: int | None = None,
    ) -> None:
        self.settings_service = settings_service
        self.catalog_service = catalog_service
        self.native_build_info = native_build_info
        self.app_version = app_version
        self.parent_pid = parent_pid
        self._catalog_initialized = catalog_service.generation > 0
        self._shutdown_requested = False
        self._events: list[dict[str, object]] = []
        self._events_lock = threading.Lock()
        self._viewport: dict[str, object] | None = None
        self._stop_event = threading.Event()
        self.playback = DesktopPlaybackService(
            settings_service=settings_service,
            catalog_service=catalog_service,
            publish_event=self._publish_event,
        )

    def ready_event(self) -> dict[str, object]:
        return event(
            "core.ready",
            {
                "app_version": self.app_version,
                "protocol_version": DESKTOP_PROTOCOL_VERSION,
                "native_build": _native_build_dict(self.native_build_info),
            },
        )

    def drain_events(self) -> tuple[dict[str, object], ...]:
        with self._events_lock:
            events = tuple(self._events)
            self._events.clear()
        return events

    def _publish_event(self, name: str, payload: Mapping[str, object]) -> None:
        message = event(name, payload)
        with self._events_lock:
            if name == "playback.snapshot":
                session_id = payload.get("session_id")
                for index in range(len(self._events) - 1, -1, -1):
                    previous = self._events[index]
                    if (
                        previous.get("name") == name
                        and isinstance(previous.get("payload"), dict)
                        and previous["payload"].get("session_id") == session_id  # type: ignore[index]
                    ):
                        self._events[index] = message
                        return
                if len(self._events) >= MAX_BUFFERED_EVENTS:
                    # Snapshots are latest-wins telemetry. Never evict a
                    # state-transition event just to retain another frame.
                    return
            elif len(self._events) >= MAX_BUFFERED_EVENTS:
                snapshot_index = next(
                    (index for index, item in enumerate(self._events) if item.get("name") == "playback.snapshot"),
                    None,
                )
                if snapshot_index is not None:
                    self._events.pop(snapshot_index)
            self._events.append(message)

    def handle_request(self, request: Mapping[str, object]) -> dict[str, object]:
        """Dispatch a validated request and return exactly one response."""
        request_id = request.get("id")
        if type(request_id) is not int:
            raise ProtocolError("invalid_id", "request id must be an integer")
        try:
            result = self._dispatch(request["method"], request["params"])
        except CoreRequestError as exc:
            return response_error(request_id, exc.code, exc.message)
        except CatalogGenerationError:
            return response_error(request_id, "stale_generation", "catalog generation is stale")
        except CatalogLookupError:
            return response_error(request_id, "not_found", "song was not found in the catalog")
        except CatalogError:
            return response_error(request_id, "catalog_error", "catalog operation failed")
        except ValueError as exc:
            return response_error(request_id, "invalid_params", str(exc))
        except Exception:
            traceback.print_exc(file=sys.stderr)
            return response_error(request_id, "internal_error", "internal Core error")
        return response_ok(request_id, result)

    def _dispatch(self, method: object, raw_params: object) -> dict[str, object]:
        if not isinstance(method, str) or method not in SUPPORTED_METHODS:
            raise CoreRequestError("unknown_method", "unknown desktop Core method")
        if not isinstance(raw_params, dict):
            raise CoreRequestError("invalid_params", "params must be an object")
        params = raw_params
        if method == "app.bootstrap":
            return self._bootstrap(_object_params(params, frozenset()))
        if method == "app.shutdown":
            _object_params(params, frozenset())
            playback_clean = self.playback.shutdown()
            self._shutdown_requested = True
            self._stop_event.set()
            if not playback_clean:
                raise CoreRequestError(
                    "shutdown_timeout",
                    "playback cleanup did not complete within the shutdown budget",
                )
            return {"shutdown": True}
        if method == "catalog.search":
            return self._search(_object_params(params, frozenset({"query", "offset", "limit", "generation"})))
        if method == "catalog.detail":
            return self._detail(_object_params(params, frozenset({"song_id", "generation"})))
        if method == "catalog.reload":
            return self._reload(_object_params(params, frozenset()))
        if method == "catalog.set_viewport":
            return self._set_viewport(
                _object_params(
                    params,
                    frozenset({"generation", "first_index", "last_index", "selected_song_id"}),
                )
            )
        if method == "settings.get":
            _object_params(params, frozenset())
            return _settings_dict(self.settings_service)
        if method == "settings.patch":
            return self._patch_settings(params)
        if method == "playback.prepare":
            return self._prepare_playback(params)
        if method == "playback.start":
            return self._start_playback(params)
        if method in {"playback.stop", "playback.pause", "playback.resume", "playback.skip"}:
            return self._playback_command(method, params)
        raise CoreRequestError("unknown_method", "unknown desktop Core method")

    def _ensure_catalog(self) -> None:
        if not self._catalog_initialized:
            self.catalog_service.scan()
            self._catalog_initialized = True

    def _bootstrap(self, _params: Mapping[str, object]) -> dict[str, object]:
        self._ensure_catalog()
        settings = self.settings_service.snapshot()
        dto = BootstrapDto(
            app_version=self.app_version,
            protocol_version=DESKTOP_PROTOCOL_VERSION,
            native_build=_native_build_dto(self.native_build_info),
            playback_defaults=PlaybackConfigDto(
                hold_frames=settings.default_hold_frames,
                tempo_scale=settings.default_tempo_scale,
                fps=settings.game_fps,
                dry_run=False,
            ),
            option_sets=PlaybackOptionSetsDto(
                hold_frames=HOLD_FRAME_OPTIONS,
                tempo_scales=TEMPO_SCALE_OPTIONS,
                fps=VALID_FPS,
            ),
            theme=settings.theme,
            telemetry_enabled=settings.telemetry_enabled,
            update_preferences=UpdatePreferencesDto(
                auto_check=settings.update_preferences.auto_check,
                channel=settings.update_preferences.channel,  # type: ignore[arg-type]
                skip_version=settings.update_preferences.skip_version,
            ),
            catalog_generation=self.catalog_service.generation,
        )
        return _bootstrap_dict(dto)

    def _search(self, params: Mapping[str, object]) -> dict[str, object]:
        self._ensure_catalog()
        query = params.get("query", "")
        if not isinstance(query, str) or len(query.encode("utf-8")) > 4 * 1024:
            raise CoreRequestError("invalid_params", "query must be bounded text")
        offset = params.get("offset", 0)
        limit = params.get("limit", 100)
        if type(offset) is not int or not 0 <= offset <= MAX_OFFSET:
            raise CoreRequestError("invalid_params", "offset is outside the supported range")
        if type(limit) is not int or not 1 <= limit <= 200:
            raise CoreRequestError("invalid_params", "limit must be between 1 and 200")
        page = self.catalog_service.search_window(
            query,
            offset=offset,
            limit=limit,
            generation=_optional_generation(params),
        )
        return {
            "items": [
                {
                    "song_id": row.song_id,
                    "title": row.title,
                    "duration_us": None,
                    "note_count": None,
                    "risk_level": "unknown",
                    "metadata_state": "pending",
                }
                for row in page.items
            ],
            "offset": page.offset,
            "limit": page.limit,
            "total": page.total,
            "generation": page.generation,
        }

    def _detail(self, params: Mapping[str, object]) -> dict[str, object]:
        self._ensure_catalog()
        song_id = _required_text(params, "song_id", max_bytes=64)
        path = self.catalog_service.path_for_song_id(
            song_id,
            generation=_optional_generation(params),
        )
        settings = self.settings_service.snapshot()
        session = PlaybackSessionContext.default(
            hold_frames=settings.default_hold_frames,
            tempo_scale=settings.default_tempo_scale,
            fps=settings.game_fps,
        )
        metadata = get_song_ui_metadata(path, session, self.settings_service.config_snapshot())
        risk_level = metadata.risk if metadata.risk in {"low", "medium", "high"} else "unknown"
        recommendations = tuple(metadata.warnings) if risk_level != "unknown" else ()
        reasons = () if risk_level == "low" else recommendations
        risk = RiskSummaryDto(
            level=risk_level,  # type: ignore[arg-type]
            headline={
                "low": "Low timing risk",
                "medium": "Medium timing risk",
                "high": "High timing risk",
                "unknown": "Risk unavailable",
            }[risk_level],
            reasons=reasons,
            recommendations=recommendations,
        )
        recommendation = None
        if risk_level != "unknown":
            recommendation = PlaybackRecommendationDto(
                recommended_hold_frames=metadata.recommended_hold_frames,
                recommended_tempo_scale=metadata.recommended_tempo_scale,
                summary=(recommendations[0] if recommendations else "Keep the selected settings."),
            )
        dto = SongDetailDto(
            song_id=song_id,
            title=path.stem,
            duration_us=round(metadata.duration_seconds * 1_000_000),
            note_count=metadata.note_count,
            format_label=path.suffix.lower().lstrip(".").upper(),
            risk=risk,
            recommendation=recommendation,
        )
        return asdict(dto)

    def _reload(self, _params: Mapping[str, object]) -> dict[str, object]:
        snapshot = self.catalog_service.scan()
        self._catalog_initialized = True
        self.playback.invalidate_catalog(snapshot.generation)
        self._publish_event(
            "catalog.changed",
            {"generation": snapshot.generation, "total": snapshot.total},
        )
        return {"generation": snapshot.generation, "total": snapshot.total}

    def _set_viewport(self, params: Mapping[str, object]) -> dict[str, object]:
        self._ensure_catalog()
        # Viewport indices are positions in the full catalog snapshot. A
        # filtered UI result has a different index space, so the desktop
        # adapter suppresses that hint until the protocol carries a filtered
        # window identity.
        generation = _required_int(params, "generation")
        first_index = _required_int(params, "first_index")
        raw_last_index = params.get("last_index")
        if type(raw_last_index) is not int:
            raise CoreRequestError("invalid_params", "last_index must be an integer")
        last_index = raw_last_index
        selected = params.get("selected_song_id")
        if selected is not None and (
            not isinstance(selected, str)
            or len(selected) != 32
            or any(char not in "0123456789abcdef" for char in selected)
        ):
            raise CoreRequestError("invalid_params", "selected_song_id must be an opaque song ID or null")

        # Viewport indices are inclusive and deliberately fail closed.  The
        # only valid empty-catalog range is 0..-1; callers must not silently
        # overscan beyond a catalog generation that they have not observed.
        entries = self.catalog_service.entries(generation=generation)
        total = len(entries)
        if total == 0:
            if first_index != 0 or last_index != -1 or selected is not None:
                raise CoreRequestError(
                    "invalid_params",
                    "empty catalog viewport must be 0..-1 with no selected song",
                )
        elif (
            last_index < first_index
            or last_index >= total
            or last_index - first_index + 1 > MAX_VIEWPORT_SPAN
        ):
            raise CoreRequestError(
                "invalid_params",
                "viewport range must be within the catalog and at most 2000 rows",
            )
        if selected is not None and selected not in {entry.song_id for entry in entries}:
            raise CoreRequestError("invalid_params", "selected_song_id is not in the catalog generation")
        self._viewport = {
            "generation": generation,
            "first_index": first_index,
            "last_index": last_index,
            "selected_song_id": selected,
        }
        return {
            "accepted": True,
            "generation": generation,
            "first_index": first_index,
            "last_index": last_index,
            "selected_song_id": selected,
        }

    def _patch_settings(self, params: Mapping[str, object]) -> dict[str, object]:
        unknown = set(params) - PATCH_FIELDS
        if unknown:
            raise CoreRequestError("invalid_params", f"unsupported settings: {', '.join(sorted(unknown))}")
        translated: dict[str, object] = {}
        for field in ("theme", "telemetry_enabled", "verbose_hud"):
            if field in params:
                translated[field] = params[field]
        if "playback_defaults" in params:
            playback = params["playback_defaults"]
            if not isinstance(playback, dict):
                raise CoreRequestError("invalid_params", "playback_defaults must be an object")
            unknown_playback = set(playback) - PLAYBACK_PATCH_FIELDS
            if unknown_playback:
                raise CoreRequestError(
                    "invalid_params",
                    f"unsupported playback settings: {', '.join(sorted(unknown_playback))}",
                )
            field_map = {
                "hold_frames": "default_hold_frames",
                "tempo_scale": "default_tempo_scale",
                "fps": "game_fps",
            }
            translated.update({field_map[key]: value for key, value in playback.items()})
        try:
            self.settings_service.patch(translated)
            self.playback.invalidate_settings()
            return _settings_dict(self.settings_service)
        except (TypeError, ValueError) as exc:
            raise CoreRequestError("invalid_params", str(exc)) from exc

    def _prepare_playback(self, params: Mapping[str, object]) -> dict[str, object]:
        self._ensure_catalog()
        if set(params) != {"song_id", "generation", "config"}:
            raise CoreRequestError(
                "invalid_params",
                "playback.prepare requires song_id, generation, and config",
            )
        song_id = params["song_id"]
        generation = params["generation"]
        config = params["config"]
        if type(song_id) is not str or type(generation) is not int or not isinstance(config, dict):
            raise CoreRequestError("invalid_params", "invalid playback.prepare parameters")
        try:
            return self.playback.prepare(
                song_id=song_id,
                generation=generation,
                config=config,
                resolve_path=self.catalog_service.path_for_song_id,
            )
        except DesktopPlaybackError as exc:
            raise CoreRequestError(exc.code, exc.message) from exc

    def _start_playback(self, params: Mapping[str, object]) -> dict[str, object]:
        if set(params) != {"prepared_id", "decisions"}:
            raise CoreRequestError("invalid_params", "playback.start requires prepared_id and decisions")
        prepared_id = params["prepared_id"]
        decisions = params["decisions"]
        if type(prepared_id) is not str or not isinstance(decisions, list):
            raise CoreRequestError("invalid_params", "invalid playback.start parameters")
        try:
            return self.playback.start(prepared_id=prepared_id, decisions=decisions)
        except DesktopPlaybackError as exc:
            raise CoreRequestError(exc.code, exc.message) from exc

    def _playback_command(self, method: str, params: Mapping[str, object]) -> dict[str, object]:
        if set(params) != {"session_id"}:
            raise CoreRequestError("invalid_params", f"{method} requires session_id")
        session_id = params["session_id"]
        if type(session_id) is not str:
            raise CoreRequestError("invalid_params", "session_id must be text")
        try:
            return self.playback.command(
                session_id=session_id,
                command=method.removeprefix("playback."),
            )
        except DesktopPlaybackError as exc:
            raise CoreRequestError(exc.code, exc.message) from exc

    def serve(self, stdin: Any, stdout: Any, *, stderr: Any = None) -> int:
        """Run until shutdown, EOF, parent loss, or a fatal protocol violation."""
        error_stream = stderr or sys.stderr
        try:
            write_frame(stdout, self.ready_event())
        except (OSError, ProtocolError) as exc:
            print(f"desktop Core could not emit ready: {exc}", file=error_stream)
            return 2

        queue: Queue[bytes | ProtocolError | None] = Queue()

        def read_worker() -> None:
            try:
                for frame in iter_bounded_frames(stdin):
                    queue.put(frame)
            except ProtocolError as exc:
                queue.put(exc)
            finally:
                queue.put(None)

        reader = threading.Thread(target=read_worker, name="desktop-core-reader", daemon=True)
        reader.start()

        parent_watch_stop = threading.Event()

        def parent_worker() -> None:
            if self.parent_pid is None:
                return
            while not parent_watch_stop.wait(0.25):
                if not parent_process_alive(self.parent_pid):
                    self.playback.shutdown()
                    self._stop_event.set()
                    queue.put(None)
                    return

        parent_thread = threading.Thread(target=parent_worker, name="desktop-core-parent-watch", daemon=True)
        parent_thread.start()

        exit_code = 0
        try:
            while not self._stop_event.is_set():
                for notification in self.drain_events():
                    write_frame(stdout, notification)
                try:
                    item = queue.get(timeout=0.05)
                except Empty:
                    continue
                if item is None:
                    break
                if isinstance(item, ProtocolError):
                    print(f"desktop Core protocol error: {item.message}", file=error_stream)
                    write_frame(
                        stdout,
                        event(
                            "core.fatal",
                            {"code": bounded_text(item.code), "message": bounded_text(item.message)},
                        ),
                    )
                    exit_code = 2
                    break
                try:
                    request = parse_request_frame(item)
                except ProtocolError as exc:
                    if exc.request_id is not None:
                        write_frame(stdout, response_error(exc.request_id, exc.code, exc.message))
                        continue
                    print(f"desktop Core protocol error: {exc.message}", file=error_stream)
                    write_frame(
                        stdout,
                        event(
                            "core.fatal",
                            {"code": bounded_text(exc.code), "message": bounded_text(exc.message)},
                        ),
                    )
                    exit_code = 2
                    break
                response = self.handle_request(request)
                write_frame(stdout, response)
                for notification in self.drain_events():
                    write_frame(stdout, notification)
                if self._shutdown_requested:
                    break
        except (OSError, ProtocolError) as exc:
            print(f"desktop Core output failure: {exc}", file=error_stream)
            exit_code = 2
        finally:
            parent_watch_stop.set()
            self._stop_event.set()
            # Shutdown may be requested while the inherited stdin pipe is
            # still open. Close our end so the bounded reader can leave its
            # blocking read before interpreter finalization; the parent owns
            # the other end and remains responsible for child termination.
            with suppress(AttributeError, OSError, ValueError):
                stdin.close()
            reader.join(timeout=0.5)
        return exit_code


__all__ = [
    "MAX_OFFSET",
    "MAX_VIEWPORT_SPAN",
    "PATCH_FIELDS",
    "PLAYBACK_PATCH_FIELDS",
    "SUPPORTED_METHODS",
    "DesktopCoreServer",
    "parent_process_alive",
]
