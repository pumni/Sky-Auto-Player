"""Application facade for the single Rust production dispatch session."""

from __future__ import annotations

import logging
from typing import Any

from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import KeyAction
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
        focus_guard: FocusGuard | None = None,
        hold_label: str = "hold 1.00f",
        hold_frames: float = 1.0,
        game_fps: int = 60,
        tempo_scale: float = 1.0,
        min_hold_us: int = 0,
        min_hold_margin_us: int = 500,
        min_hold_margin_source: str = "default_500",
        dry_run: bool = False,
    ) -> None:
        self.song = song
        self.actions = tuple(actions)
        self.controls = controls
        self.renderer = renderer
        self.require_focus = bool(require_focus)
        self.dry_run = bool(dry_run)
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
            focus_guard=self.focus_guard,
            controls=self.controls,
            renderer=self.renderer,
            poll_s=0.01,
            telemetry_enabled=self.telemetry.enabled,
        )
        outcome, snapshot, native_telemetry, _estimator_state_json = runtime.run()
        self._last_snapshot = snapshot
        self._input_path_degraded = bool(snapshot.get("input_path_degraded", False))
        self._ingest_native_report(snapshot, native_telemetry)
        if outcome == PLAYBACK_ERROR:
            raise NativeDispatchError(
                str(snapshot.get("terminal_error") or "native dispatch failed")
            )
        return outcome

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
            BackendHealth(
                active_count=int(RustDispatchRuntime._required(snapshot, "active_count")),
                possibly_active_count=int(
                    RustDispatchRuntime._required(snapshot, "possibly_active_count")
                ),
                failed_release_count=int(
                    RustDispatchRuntime._required(snapshot, "failed_release_count")
                ),
                last_error=RustDispatchRuntime._required(snapshot, "last_error"),
                keys_dropped=int(RustDispatchRuntime._required(snapshot, "keys_dropped")),
                chord_split_events=int(
                    RustDispatchRuntime._required(snapshot, "chord_split_events")
                ),
                sendinput_partial_events=int(
                    RustDispatchRuntime._required(snapshot, "sendinput_partial_events")
                ),
                sendinput_zero_progress_failures=int(
                    RustDispatchRuntime._required(
                        snapshot, "sendinput_zero_progress_failures"
                    )
                ),
                chords_rejected=int(
                    RustDispatchRuntime._required(snapshot, "chords_rejected")
                ),
                authored_conflict_events=int(
                    RustDispatchRuntime._required(snapshot, "authored_conflict_events")
                ),
                authored_chords_rejected=int(
                    RustDispatchRuntime._required(snapshot, "authored_chords_rejected")
                ),
                authored_keys_rejected=int(
                    RustDispatchRuntime._required(snapshot, "authored_keys_rejected")
                ),
                keys_inserted_before_failure=int(
                    RustDispatchRuntime._required(
                        snapshot, "keys_inserted_before_failure"
                    )
                ),
                keys_rolled_back=int(
                    RustDispatchRuntime._required(snapshot, "keys_rolled_back")
                ),
                rollback_residue_keys=int(
                    RustDispatchRuntime._required(snapshot, "rollback_residue_keys")
                ),
            )
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
