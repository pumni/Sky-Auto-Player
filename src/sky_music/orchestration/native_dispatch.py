"""Single Python boundary for the native Rust dispatch runtime.

The Rust worker owns scheduling, waits, focus-gated dispatch, SendInput state,
and cleanup. This adapter only converts immutable inputs, polls commands/focus,
and maps latest-wins snapshots into the existing renderer contract.
"""

from __future__ import annotations

import contextlib
import json
import time
from collections.abc import Sequence
from typing import Any, Protocol, cast

from sky_music.domain.scheduler_types import KeyAction
from sky_music.orchestration.native_models import (
    LIVE_NATIVE_STATUSES,
    PLAYBACK_ERROR,
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SHUTDOWN_TIMEOUT,
    PLAYBACK_SKIPPED,
    TERMINAL_NATIVE_STATUSES,
    BackendHealth,
    NativeDispatchError,
    NativeSessionStatus,
    PlaybackOutcome,
    ProgressCounters,
    parse_native_session_status,
)

FOCUS_PLATFORM_ERRORS = (AttributeError, OSError, RuntimeError, TypeError, ValueError)


class NativeBackendHealthProtocol(Protocol):
    @property
    def active_count(self) -> int: ...
    @property
    def possibly_active_count(self) -> int: ...
    @property
    def failed_release_count(self) -> int: ...
    @property
    def last_error(self) -> str | None: ...
    @property
    def keys_dropped(self) -> int: ...
    @property
    def chord_split_events(self) -> int: ...
    @property
    def sendinput_partial_events(self) -> int: ...
    @property
    def sendinput_zero_progress_failures(self) -> int: ...
    @property
    def chords_rejected(self) -> int: ...
    @property
    def authored_conflict_events(self) -> int: ...
    @property
    def authored_chords_rejected(self) -> int: ...
    @property
    def authored_keys_rejected(self) -> int: ...
    @property
    def keys_inserted_before_failure(self) -> int: ...
    @property
    def keys_rolled_back(self) -> int: ...
    @property
    def rollback_residue_keys(self) -> int: ...


class NativeProgressSnapshotProtocol(Protocol):
    elapsed_us: int
    total_us: int
    pre_roll_remaining_us: int
    max_completion_error_us: int
    late_2ms: int
    late_5ms: int
    late_10ms: int
    max_sendinput_pre_call_lateness_us: int
    pre_call_late_2ms: int
    pre_call_late_5ms: int
    pre_call_late_10ms: int
    release_max_us: int
    release_late_2ms: int
    recent_latencies_us: Sequence[int]
    recent_latency_samples_available: bool
    is_finished: bool
    is_paused: bool
    input_path_degraded: bool
    sendinput_path_degraded: bool
    core_post_send_degraded: bool
    post_send_metrics_available: bool
    wait_path_degraded: bool
    recovered_zero_progress_but_late: int
    recovered_zero_progress_retries: int
    recovered_partial_up_retries: int
    wait_backend_failures: int
    wait_clock_failures: int
    status: str
    @property
    def backend_health(self) -> NativeBackendHealthProtocol: ...


class NativeFocusHintProtocol(Protocol):
    def set_focus_hint(self, active: bool) -> None: ...


class RustDispatchRuntime:
    """Supervisor-side adapter; never participates in the real-time hot path."""

    __slots__ = (
        "_controls",
        "_focus_guard",
        "_has_played",
        "_last_focus_active",
        "_last_hwnd",
        "_manual_paused",
        "_min_hold_us",
        "_pre_roll_us",
        "_renderer",
        "_require_focus",
        "_session",
        "_sleep_s",
        "_song_name",
        "_target_hwnd",
        "_total_us",
    )

    def __init__(
        self,
        *,
        actions: Sequence[KeyAction],
        song_name: str,
        game_fps: int,
        min_hold_us: int,
        require_focus: bool,
        focus_guard: Any,
        controls: Any,
        renderer: Any,
        poll_s: float,
        telemetry_enabled: bool = False,
        focus_restore_grace_us: int = 100_000,
        pre_roll_us: int = 0,
        target_hwnd: int | None = None,
    ) -> None:
        import sky_player_rs  # type: ignore[import-not-found]

        if require_focus and (type(target_hwnd) is not int or target_hwnd <= 0):
            raise ValueError("target_hwnd must be the validated positive HWND when focus is required")

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
        session_config_type = cast(Any, sky_player_rs.SessionConfig)  # type: ignore[attr-defined]
        self._session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            native_actions,
            config=session_config_type(
                game_fps=int(game_fps),
                min_hold_us=min_hold_us,
                require_focus=require_focus,
                focus_restore_grace_us=focus_restore_grace_us,
                target_hwnd=target_hwnd or 0,
                telemetry=telemetry_enabled,
                profile="production",
            ),
        )
        self._song_name = song_name
        self._min_hold_us = min_hold_us
        if type(pre_roll_us) is not int or pre_roll_us < 0:
            raise ValueError("pre_roll_us must be a non-negative integer")
        self._pre_roll_us = pre_roll_us
        self._target_hwnd = target_hwnd if require_focus else None
        self._require_focus = require_focus
        self._focus_guard = focus_guard
        self._controls = controls
        self._renderer = renderer
        self._sleep_s = max(0.002, min(0.05, poll_s))
        self._total_us = max((int(action.at_us) for action in actions), default=0)
        self._manual_paused = False
        self._last_focus_active: bool | None = None
        self._last_hwnd: int | None = None
        self._has_played = False

    def _attempt_refocus_and_refresh(self) -> None:
        with contextlib.suppress(*FOCUS_PLATFORM_ERRORS):
            self._focus_guard.focus()
        self._refresh_target_after_explicit_refocus()
        self._publish_focus()

    def _publish_focus(self) -> None:
        if not self._require_focus:
            return
        try:
            from sky_music.platform.win32 import window_target

            expected_hwnd = self._target_hwnd or 0
            cached_hwnd = int(window_target.cached_target_hwnd())
            hwnd = expected_hwnd
            focus_active = (
                expected_hwnd > 0
                and cached_hwnd == expected_hwnd
                and bool(window_target.is_foreground_cached_hwnd())
            )
        except FOCUS_PLATFORM_ERRORS:
            hwnd = 0
            focus_active = False
        if hwnd != self._last_hwnd:
            self._session.set_target_hwnd(hwnd)
            self._last_hwnd = hwnd
        if focus_active != self._last_focus_active:
            cast(NativeFocusHintProtocol, self._session).set_focus_hint(focus_active)
            self._last_focus_active = focus_active

    def _set_initial_target(self) -> None:
        if not self._require_focus:
            return
        target = int(self._target_hwnd or 0)
        self._session.set_target_hwnd(target)
        self._last_hwnd = target

    def _refresh_target_after_explicit_refocus(self) -> None:
        """Resolve a replacement HWND only after the explicit refocus command."""
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
        except FOCUS_PLATFORM_ERRORS:
            target = 0
        self._target_hwnd = target if target > 0 else None
        self._session.set_target_hwnd(target)
        self._last_hwnd = target

    def _handle_command(self, command: str | None) -> str | None:
        def terminal_race_is_done() -> bool:
            try:
                return bool(self._session.snapshot_lite().is_finished)
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
            self._attempt_refocus_and_refresh()
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
    def _required(snapshot: object, field: str) -> Any:
        """Read a correctness-critical native field without a silent default."""
        try:
            return getattr(snapshot, field)
        except AttributeError as exc:
            raise NativeDispatchError(
                f"native snapshot is missing required field: {field}"
            ) from exc

    @classmethod
    def _health(cls, snapshot: NativeBackendHealthProtocol) -> BackendHealth:
        try:
            return BackendHealth.from_native(snapshot)
        except ValueError as exc:
            raise NativeDispatchError(str(exc)) from exc

    def _native_is_finished(self) -> bool:
        try:
            return bool(self._session.snapshot_lite().is_finished)
        except (AttributeError, RuntimeError, TypeError, ValueError):
            return False

    def run(self) -> tuple[PlaybackOutcome, dict[str, Any], dict[str, Any]]:
        """Run supervisor polling until the native worker reaches a terminal state."""
        started = False
        joined = False
        requested_outcome: str | None = None
        live: NativeProgressSnapshotProtocol
        latest: dict[str, Any] = {}
        report: dict[str, Any] | None = None
        next_render_at = 0.0
        try:
            if self._controls is not None:
                start_controls = getattr(self._controls, "start", None)
                if callable(start_controls):
                    start_controls()
            self._set_initial_target()
            self._publish_focus()
            self._session.arm(self._pre_roll_us)  # type: ignore[attr-defined]
            started = True

            live = cast(NativeProgressSnapshotProtocol, self._session.snapshot_lite())
            initial_status = parse_native_session_status(str(live.status))
            if initial_status == NativeSessionStatus.PLAYING:
                self._has_played = True

            while True:
                if live.is_finished:
                    break
                native_status = parse_native_session_status(str(live.status))
                if native_status not in LIVE_NATIVE_STATUSES:
                    raise NativeDispatchError(
                        f"unexpected live native session status: {native_status}"
                    )
                command = self._controls.poll() if self._controls is not None else None
                requested_outcome = self._handle_command(command) or requested_outcome
                self._publish_focus()
                self._session.heartbeat()
                live = cast(NativeProgressSnapshotProtocol, self._session.snapshot_lite())
                native_status = parse_native_session_status(str(live.status))
                if native_status == NativeSessionStatus.PLAYING:
                    self._has_played = True

                now = time.monotonic()
                if self._renderer is not None and now >= next_render_at:
                    if hasattr(self._renderer, "update_counters_batch"):
                        self._renderer.update_counters_batch(
                            ProgressCounters(
                                max_lateness_us=max(
                                    live.max_completion_error_us,
                                    int(
                                        getattr(
                                            live,
                                            "max_sendinput_pre_call_lateness_us",
                                            0,
                                        )
                                    ),
                                ),
                                late_2ms=max(
                                    live.late_2ms,
                                    int(getattr(live, "pre_call_late_2ms", 0)),
                                ),
                                late_5ms=max(
                                    live.late_5ms,
                                    int(getattr(live, "pre_call_late_5ms", 0)),
                                ),
                                late_10ms=max(
                                    live.late_10ms,
                                    int(getattr(live, "pre_call_late_10ms", 0)),
                                ),
                                release_max_us=live.release_max_us,
                                release_late_2ms=live.release_late_2ms,
                                recent_latencies_us=tuple(
                                    int(value) for value in live.recent_latencies_us
                                ),
                            )
                        )
                    if native_status == NativeSessionStatus.PREROLL:
                        status = "countdown"
                    elif live.is_paused:
                        status = (
                            "paused"
                            if self._manual_paused
                            else "focus_lost"
                            if self._has_played
                            else "waiting_for_focus"
                        )
                    else:
                        status = "playing"
                    self._renderer.render(
                        0.0 if native_status == NativeSessionStatus.PREROLL else live.elapsed_us / 1_000_000,
                        max(self._total_us, 1) / 1_000_000,
                        self._song_name,
                        status=status,
                        pre_roll_remaining_us=int(live.pre_roll_remaining_us),
                        input_path_degraded=live.input_path_degraded,
                        sendinput_path_degraded=live.sendinput_path_degraded,
                        core_post_send_degraded=live.core_post_send_degraded,
                        wait_path_degraded=live.wait_path_degraded,
                        wait_backend_failures=live.wait_backend_failures,
                        wait_clock_failures=live.wait_clock_failures,
                        recovered_zero_progress_but_late=live.recovered_zero_progress_but_late,
                        recovered_partial_up_retries=live.recovered_partial_up_retries,
                        backend_health=self._health(live.backend_health),
                    )
                    next_render_at = now + (1.0 / 30.0)
                time.sleep(self._sleep_s)

            joined = self._join_owned()
            if not joined:
                outcome = PLAYBACK_SHUTDOWN_TIMEOUT
            else:
                report = dict(self._session.session_report())
                latest = dict(report["snapshot"])
                final_status = parse_native_session_status(str(latest.get("status")))
                if final_status not in TERMINAL_NATIVE_STATUSES:
                    raise NativeDispatchError(
                        f"unexpected terminal native session status: {final_status}"
                    )
                raw_outcome = latest.get("outcome") or requested_outcome or PLAYBACK_FINISHED
                try:
                    outcome = PlaybackOutcome(str(raw_outcome))
                except ValueError as exc:
                    raise NativeDispatchError(
                        f"unknown native playback outcome: {raw_outcome}"
                    ) from exc

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
                return outcome, latest, telemetry
            assert report is not None
            telemetry = json.loads(str(report["telemetry_json"]))
            final_status = parse_native_session_status(str(latest.get("status")))
            if final_status in {
                NativeSessionStatus.ERROR,
                NativeSessionStatus.PANICKED,
                NativeSessionStatus.POISONED,
            }:
                detail = latest.get("terminal_error") or final_status.value
                raise NativeDispatchError(
                    str(detail),
                    snapshot=latest,
                    telemetry=telemetry,
                )
            return outcome, latest, telemetry
        except BaseException:
            if started and not self._native_is_finished():
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    self._session.panic_release()
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    self._session.quit()
            if started and not joined:
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    joined = self._join_owned()
            raise
        finally:
            if started and not joined:
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    self._session.quit()
                with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                    self._join_owned()
            if self._controls is not None:
                close_controls = getattr(self._controls, "close", None)
                if callable(close_controls):
                    with contextlib.suppress(AttributeError, RuntimeError, TypeError, ValueError):
                        close_controls()
