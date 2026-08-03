"""Single Python boundary for the native Rust dispatch runtime.

The Rust worker owns scheduling, waits, focus-gated dispatch, SendInput state,
and cleanup. This adapter only converts immutable inputs, polls commands/focus,
and maps latest-wins snapshots into the existing renderer contract.
"""

from __future__ import annotations

import contextlib
import json
from collections.abc import Sequence
from typing import Any

from sky_music.domain.scheduler_types import KeyAction
from sky_music.layouts import SKY_15_SCAN_CODES
from sky_music.orchestration.native_models import (
    PLAYBACK_ERROR,
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SHUTDOWN_TIMEOUT,
    PLAYBACK_SKIPPED,
    BackendHealth,
    ProgressCounters,
)


class NativeDispatchError(RuntimeError):
    """A controlled native worker failure after cleanup and telemetry capture."""


class RustDispatchRuntime:
    """Supervisor-side adapter; never participates in the real-time hot path."""

    __slots__ = (
        "_controls",
        "_focus_guard",
        "_has_played",
        "_last_hwnd",
        "_manual_paused",
        "_min_hold_us",
        "_renderer",
        "_require_focus",
        "_session",
        "_sleep_s",
        "_song_name",
        "_total_us",
    )

    def __init__(
        self,
        *,
        actions: Sequence[KeyAction],
        song_name: str,
        min_hold_us: int,
        require_focus: bool,
        focus_guard: Any,
        controls: Any,
        renderer: Any,
        poll_s: float,
        telemetry_enabled: bool = False,
    ) -> None:
        import sky_player_rs  # type: ignore[import-not-found]

        native_actions = (
            (
                index,
                str(action.kind),
                int(action.at_us),
                [int(scan_code) for scan_code in action.scan_codes],
                action.reason,
            )
            for index, action in enumerate(actions)
        )
        self._session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            native_actions,
            list(SKY_15_SCAN_CODES),
            config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
                min_hold_us=min_hold_us,
                require_focus=require_focus,
                telemetry=telemetry_enabled,
                profile="production",
            ),
        )
        self._song_name = song_name
        self._min_hold_us = min_hold_us
        self._require_focus = require_focus
        self._focus_guard = focus_guard
        self._controls = controls
        self._renderer = renderer
        self._sleep_s = max(0.002, min(0.05, poll_s))
        self._total_us = max((int(action.at_us) for action in actions), default=0)
        self._manual_paused = False
        self._last_hwnd: int | None = None
        self._has_played = False

    def _publish_focus(self) -> None:
        if not self._require_focus:
            return
        try:
            from sky_music.platform.win32 import window_target

            hwnd = window_target.cached_target_hwnd()
        except (AttributeError, TypeError, ValueError):
            hwnd = 0
        if hwnd != self._last_hwnd:
            self._session.set_target_hwnd(hwnd)
            self._last_hwnd = hwnd

    def _set_initial_target(self) -> None:
        if not self._require_focus:
            return
        try:
            from sky_music.platform.win32 import window_target

            window_target.reset_window_cache()
            target = (
                int(window_target.cached_target_hwnd())
                if window_target.is_sky_window_valid()
                else 0
            )
            self._session.set_target_hwnd(target)
            self._last_hwnd = target
        except (AttributeError, OSError, RuntimeError, TypeError, ValueError):
            self._session.set_target_hwnd(0)
            self._last_hwnd = 0

    def _handle_command(self, command: str | None) -> str | None:
        def terminal_race_is_done() -> bool:
            try:
                return bool(self._session.snapshot_lite().get("is_finished"))
            except (AttributeError, RuntimeError, TypeError, ValueError):
                return False

        if command == "quit":
            try:
                self._session.quit()
                return PLAYBACK_QUIT
            except RuntimeError:
                if terminal_race_is_done():
                    return None
                raise
        if command == "skip":
            try:
                self._session.skip()
                return PLAYBACK_SKIPPED
            except RuntimeError:
                if terminal_race_is_done():
                    return None
                raise
        if command == "pause":
            next_paused = not self._manual_paused
            try:
                if next_paused:
                    self._session.pause()
                else:
                    self._session.resume()
                self._manual_paused = next_paused
            except RuntimeError:
                if not terminal_race_is_done():
                    raise
        elif command == "panic":
            try:
                self._session.panic_release()
            except RuntimeError:
                if not terminal_race_is_done():
                    raise
        elif command == "refocus":
            self._focus_guard.focus()
            self._set_initial_target()
        return None

    def _join_owned(self) -> bool:
        """Join the worker, escalating once if the first bounded wait expires."""
        joined = bool(self._session.join(timeout_ms=5_000))
        if not joined:
            with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                self._session.quit()
            joined = bool(self._session.join(timeout_ms=5_000))
        return joined

    @staticmethod
    def _health(snapshot: dict[str, Any]) -> BackendHealth:
        return BackendHealth(
            active_count=int(snapshot.get("active_count", snapshot.get("active_keys", 0))),
            possibly_active_count=int(snapshot.get("possibly_active_count", 0)),
            failed_release_count=int(snapshot.get("failed_release_count", 0)),
            last_error=snapshot.get("last_error"),
            keys_dropped=int(snapshot.get("keys_dropped", 0)),
            chord_split_events=int(snapshot.get("chord_split_events", 0)),
            sendinput_partial_events=int(snapshot.get("sendinput_partial_events", 0)),
            sendinput_zero_progress_failures=int(
                snapshot.get("sendinput_zero_progress_failures", 0)
            ),
            chords_rejected=int(snapshot.get("chords_rejected", 0)),
            authored_conflict_events=int(snapshot.get("authored_conflict_events", 0)),
            authored_chords_rejected=int(snapshot.get("authored_chords_rejected", 0)),
            authored_keys_rejected=int(snapshot.get("authored_keys_rejected", 0)),
            keys_inserted_before_failure=int(
                snapshot.get("keys_inserted_before_failure", 0)
            ),
            keys_rolled_back=int(snapshot.get("keys_rolled_back", 0)),
            rollback_residue_keys=int(snapshot.get("rollback_residue_keys", 0)),
        )

    def run(self) -> tuple[str, dict[str, Any], dict[str, Any], str | None]:
        """Run supervisor polling until the native worker reaches a terminal state."""
        import time

        started = False
        joined = False
        requested_outcome: str | None = None
        latest: dict[str, Any] = {}
        report: dict[str, Any] | None = None
        try:
            self._set_initial_target()
            self._publish_focus()
            self._session.start()
            started = True
            
            latest = dict(self._session.snapshot_lite())

            while not latest["is_finished"]:
                command = self._controls.poll() if self._controls is not None else None
                requested_outcome = self._handle_command(command) or requested_outcome
                self._publish_focus()
                self._session.heartbeat()
                latest = dict(self._session.snapshot_lite())

                if self._renderer is not None:
                    if hasattr(self._renderer, "update_counters_batch"):
                        self._renderer.update_counters_batch(
                            ProgressCounters(
                                max_lateness_us=int(latest.get("max_completion_error_us", 0)),
                                late_2ms=0,
                                late_5ms=0,
                                late_10ms=0,
                                release_max_us=0,
                                release_late_2ms=0,
                                recent_latencies_us=tuple(
                                    int(value) for value in latest.get("recent_latencies_us", ())
                                ),
                            )
                        )
                    if latest["is_paused"]:
                        status = (
                            "paused"
                            if self._manual_paused
                            else "focus_lost"
                            if self._has_played
                            else "waiting_for_focus"
                        )
                    else:
                        status = "playing"
                        self._has_played = True
                    self._renderer.render(
                        latest["elapsed_us"] / 1_000_000,
                        max(self._total_us, 1) / 1_000_000,
                        self._song_name,
                        status=status,
                        input_path_degraded=bool(latest.get("input_path_degraded", False)),
                        backend_health=self._health(latest),
                    )
                time.sleep(self._sleep_s)

            joined = self._join_owned()
            if not joined:
                outcome = PLAYBACK_SHUTDOWN_TIMEOUT
            else:
                report = dict(self._session.session_report())
                latest = dict(report["snapshot"])
                if latest.get("status") in {"panicked", "poisoned"}:
                    detail = latest.get("terminal_error") or latest["status"]
                    raise RuntimeError(f"native dispatch terminated: {detail}")
                outcome = str(latest.get("outcome") or requested_outcome or PLAYBACK_FINISHED)
                if outcome not in {
                    PLAYBACK_FINISHED,
                    PLAYBACK_ERROR,
                    PLAYBACK_QUIT,
                    PLAYBACK_SKIPPED,
                    PLAYBACK_SHUTDOWN_TIMEOUT,
                }:
                    raise RuntimeError(f"unknown native playback outcome: {outcome}")

            if self._renderer is not None:
                verb = {
                    PLAYBACK_ERROR: "Error",
                    PLAYBACK_FINISHED: "Finished",
                    PLAYBACK_QUIT: "Stopped",
                    PLAYBACK_SKIPPED: "Skipped",
                    PLAYBACK_SHUTDOWN_TIMEOUT: "Shutdown timeout",
                }[outcome]
                self._renderer.finish(f"{verb}: {self._song_name}")
            if not joined:
                telemetry = {
                    "records": [],
                    "attempted": 0,
                    "accepted": 0,
                    "dropped": 0,
                    "truncated": False,
                }
                return outcome, latest, telemetry, None
            assert report is not None
            telemetry = json.loads(str(report["telemetry_json"]))
            return outcome, latest, telemetry, str(report["estimator_state_json"])
        except BaseException:
            if started:
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    self._session.panic_release()
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    self._session.quit()
                if not joined:
                    with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                        joined = self._join_owned()
            raise
        finally:
            if started and not joined:
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    self._session.quit()
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    self._join_owned()
