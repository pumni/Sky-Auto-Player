"""Application facade for the single Rust production dispatch session."""

from __future__ import annotations

import logging
from typing import Any

from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import (
    DEFAULT_FOCUS_RESTORE_GRACE_US,
    KeyAction,
)
from sky_music.infrastructure.focus import (
    FocusGuard,
    NoopFocusGuard,
    Win32SkyFocusGuard,
)
from sky_music.orchestration.native_dispatch import (
    NativeDispatchError,
    RustDispatchRuntime,
)
from sky_music.orchestration.native_models import (
    PLAYBACK_ERROR,
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SHUTDOWN_TIMEOUT,
    PLAYBACK_SKIPPED,
    RUST_DISPATCH_SCHEMA_VERSION,
    BackendHealth,
    ReleaseAllOutcome,
)
from sky_music.orchestration.telemetry import TelemetryLogger

_LOGGER = logging.getLogger(__name__)


class PlaybackEngine:
    """Small application facade around Rust native playback.

    The only non-native mode is an explicitly named preview. Preview never
    constructs a scheduler, sender, retry loop, or input backend; it simply
    reports a simulated completion to the UI.
    """

    def __init__(
        self,
        song: Song,
        actions: tuple[KeyAction, ...],
        controls: Any = None,
        renderer: Any = None,
        telemetry_enabled: bool = False,
        require_focus: bool = True,
        focus_restore_grace_us: int = DEFAULT_FOCUS_RESTORE_GRACE_US,
        focus_guard: FocusGuard | None = None,
        hold_label: str = "hold 1.00f",
        hold_frames: float = 1.0,
        game_fps: int = 60,
        tempo_scale: float = 1.0,
        min_hold_us: int = 0,
        min_hold_margin_us: int = 500,
        min_hold_margin_source: str = "default_500",
        dry_run: bool = False,
        pre_roll_us: int = 0,
        target_hwnd: int | None = None,
    ) -> None:
        self.song = song
        self.actions = tuple(actions)
        self.controls = controls
        self.renderer = renderer
        self.require_focus = bool(require_focus)
        if not 0 <= int(focus_restore_grace_us) <= 60_000_000:
            raise ValueError("focus_restore_grace_us must be in 0..=60000000")
        self.focus_restore_grace_us = int(focus_restore_grace_us)
        self.dry_run = bool(dry_run)
        if type(pre_roll_us) is not int or pre_roll_us < 0:
            raise ValueError("pre_roll_us must be a non-negative integer")
        self.pre_roll_us = pre_roll_us
        self.game_fps = int(game_fps)
        if not 15 <= self.game_fps <= 240:
            raise ValueError("game_fps must be in 15..=240")
        self.min_hold_us = max(0, int(min_hold_us))
        self.hold_label = hold_label
        self.hold_frames = hold_frames
        self.tempo_scale = tempo_scale
        self.total_time_us = max((int(action.at_us) for action in self.actions), default=0)
        self._input_path_degraded = False
        self._last_snapshot: dict[str, Any] = {}
        if target_hwnd is not None and (type(target_hwnd) is not int or target_hwnd <= 0):
            raise ValueError("target_hwnd must be a positive HWND when provided")
        self._prepared_target_hwnd: int | None = target_hwnd

        self.telemetry = TelemetryLogger(
            song.name,
            enabled=telemetry_enabled,
            hold_frames=hold_frames,
            hold_label=hold_label,
            fps=self.game_fps,
            min_hold_us=self.min_hold_us,
            min_hold_margin_us=min_hold_margin_us,
            min_hold_margin_source=min_hold_margin_source,
            tempo_scale=tempo_scale,
            retain_records_after_save=False,
        )
        self.telemetry.record_runtime_options(
            {
                "dispatch_engine": "preview" if self.dry_run else "rust",
                "rust_dispatch_schema_version": RUST_DISPATCH_SCHEMA_VERSION,
                "dry_run": self.dry_run,
                "min_hold_us": self.min_hold_us,
                "focus_required": self.require_focus,
            }
        )

        self.focus_guard = focus_guard or (
            Win32SkyFocusGuard() if self.require_focus else NoopFocusGuard()
        )

    @property
    def input_path_degraded(self) -> bool:
        return self._input_path_degraded

    def _play_native(self) -> str:
        runtime = RustDispatchRuntime(
            actions=self.actions,
            song_name=self.song.name,
            game_fps=self.game_fps,
            min_hold_us=self.min_hold_us,
            require_focus=self.require_focus,
            focus_restore_grace_us=self.focus_restore_grace_us,
            pre_roll_us=self.pre_roll_us,
            target_hwnd=self._prepared_target_hwnd,
            focus_guard=self.focus_guard,
            controls=self.controls,
            renderer=self.renderer,
            poll_s=0.01,
            telemetry_enabled=self.telemetry.enabled,
        )
        try:
            outcome, snapshot, native_telemetry = runtime.run()
        except NativeDispatchError as exc:
            # The native supervisor has already joined and materialized the
            # final report before raising a terminal worker error. Ingest it
            # here so diagnostics are not lost on the error path.
            if exc.snapshot is not None and exc.telemetry is not None:
                snapshot = dict(exc.snapshot)
                native_telemetry = dict(exc.telemetry)
                self._last_snapshot = snapshot
                self._input_path_degraded = bool(
                    snapshot.get("input_path_degraded", False)
                )
                self._ingest_native_report(snapshot, native_telemetry)
            raise
        self._last_snapshot = snapshot
        self._input_path_degraded = bool(snapshot.get("input_path_degraded", False))
        self._ingest_native_report(snapshot, native_telemetry)
        if outcome == PLAYBACK_ERROR:
            raise NativeDispatchError(
                str(snapshot.get("terminal_error") or "native dispatch failed")
            )
        return outcome

    def prepare_focus_for_playback(self) -> int | bool:
        """Resolve, focus, and verify one exact target at playback commit."""
        if self.dry_run or not self.require_focus:
            return True
        try:
            from sky_music.platform.win32 import window_target

            window_target.reset_window_cache()
            if not window_target.is_sky_window_valid():
                return False
            if not bool(self.focus_guard.focus()):
                return False
            if not bool(window_target.is_foreground_cached_hwnd()):
                return False
            target_hwnd = int(window_target.cached_target_hwnd())
            if target_hwnd <= 0:
                return False
            self._prepared_target_hwnd = target_hwnd
            return target_hwnd
        except (AttributeError, OSError, RuntimeError, TypeError, ValueError):
            return False

    def _play_preview(self) -> str:
        """Complete an explicit UI preview without touching the input path."""
        self._last_snapshot = {
            "status": "finished",
            "outcome": PLAYBACK_FINISHED,
            "elapsed_us": self.total_time_us,
            "total_us": self.total_time_us,
        }
        self.telemetry.save()
        return PLAYBACK_FINISHED

    def _ingest_native_report(
        self, snapshot: dict[str, Any], native_telemetry: dict[str, Any]
    ) -> None:
        self.telemetry.record_runtime_options(
            {
                "native_build_version": snapshot.get("native_build_version"),
                "native_build_commit": snapshot.get("native_build_commit"),
                "native_abi": snapshot.get("native_abi"),
                "rt_priority_acquired": snapshot.get("rt_priority_acquired"),
                "wait_strategy_acquired": snapshot.get("wait_strategy_acquired"),
                "input_path_degraded": snapshot.get("input_path_degraded", False),
            }
        )
        self.telemetry.record_backend_health(
            BackendHealth.from_native(snapshot)
        )
        self.telemetry.record_generation_status_counts(
            {str(k): int(v) for k, v in snapshot.get("generation_status_counts", {}).items()}
        )
        self.telemetry.record_abort_counts(
            {str(k): int(v) for k, v in snapshot.get("abort_counts_by_reason", {}).items()}
        )
        release = snapshot.get("release_outcome")
        if isinstance(release, dict):
            self.telemetry.record_release_outcome(
                ReleaseAllOutcome(
                    attempted=tuple(int(v) for v in release.get("attempted", ())),
                    released_successfully=bool(release.get("released_successfully", False)),
                    stuck_keys=tuple(int(v) for v in release.get("stuck_keys", ())),
                    verification_inconclusive=bool(
                        release.get("verification_inconclusive", False)
                    ),
                )
            )
        self.telemetry.ingest_native_output(native_telemetry)
        self.telemetry.save()

    def play(self) -> str:
        """Run one native session, or one explicit preview."""
        if self.dry_run:
            return self._play_preview()
        if not self.actions:
            self.telemetry.save()
            return PLAYBACK_FINISHED
        return self._play_native()

    def release_song_data(self) -> None:
        self.actions = ()
        self.total_time_us = 0
        try:
            from sky_music.orchestration.runtime_session import RUNTIME_STATE

            RUNTIME_STATE.clear_session()
        except AttributeError:
            _LOGGER.debug("runtime session cleanup was unavailable")


__all__ = [
    "PLAYBACK_ERROR",
    "PLAYBACK_FINISHED",
    "PLAYBACK_QUIT",
    "PLAYBACK_SHUTDOWN_TIMEOUT",
    "PLAYBACK_SKIPPED",
    "PlaybackEngine",
]
