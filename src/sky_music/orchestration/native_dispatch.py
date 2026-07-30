"""Single Python boundary for the native Rust dispatch runtime.

The Rust worker owns scheduling, waits, focus-gated dispatch, SendInput state,
and cleanup. This adapter only converts immutable inputs, polls commands/focus,
and maps latest-wins snapshots into the existing renderer contract.
"""

from __future__ import annotations

import json
import os
import sys
from collections.abc import Sequence
from typing import Any

from sky_music.domain.scheduler_types import KeyAction
from sky_music.infrastructure.backend import BackendHealth
from sky_music.layouts import SKY_15_SCAN_CODES
from sky_music.orchestration.core.ports import (
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SHUTDOWN_TIMEOUT,
    PLAYBACK_SKIPPED,
    RUST_DISPATCH_SCHEMA_VERSION,
    ProgressCounters,
)

_RUST_AVAILABLE: bool | None = None


def python_dispatch_explicitly_requested() -> bool:
    """Return whether the internal rollback switch explicitly selects Python."""
    value = os.environ.get("SKY_USE_PYTHON_DISPATCH", "").strip().casefold()
    if value in {"1", "true", "yes", "on"}:
        return True
    legacy = os.environ.get("SKY_USE_RUST_DISPATCH", "").strip().casefold()
    return legacy in {"0", "false", "no", "off"}


def native_dispatch_explicitly_requested() -> bool:
    """Return whether the pre-soak internal feature flag opts into Rust."""
    value = os.environ.get("SKY_USE_RUST_DISPATCH", "").strip().casefold()
    return value in {"1", "true", "yes", "on"}


def is_native_dispatch_available() -> bool:
    """Validate the installed extension and no-GIL ABI once per process."""
    global _RUST_AVAILABLE
    if _RUST_AVAILABLE is not None:
        return _RUST_AVAILABLE
    try:
        import sky_player_rs

        info = sky_player_rs.build_info()  # type: ignore[attr-defined]
        gil_enabled = getattr(sys, "_is_gil_enabled", lambda: True)()
        _RUST_AVAILABLE = (
            info.get("schema_version") == RUST_DISPATCH_SCHEMA_VERSION
            and info.get("free_threaded") is True
            and info.get("win32_backend") is True
            and gil_enabled is False
        )
    except (ImportError, AttributeError, RuntimeError, TypeError):
        _RUST_AVAILABLE = False
    return _RUST_AVAILABLE


def reset_native_dispatch_availability_cache() -> None:
    global _RUST_AVAILABLE
    _RUST_AVAILABLE = None


class RustDispatchRuntime:
    """Supervisor-side adapter; never participates in the real-time hot path."""

    __slots__ = (
        "_actions",
        "_controls",
        "_focus_guard",
        "_has_played",
        "_last_focus_active",
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
        max_lead_us: int,
        focus_restore_grace_us: int,
        spin_threshold_us: int,
        late_pulse_drop_threshold_us: int | None,
        same_key_conflict_policy: str,
        telemetry_enabled: bool,
        rt_priority_mode: str,
        enable_waitable_timer: bool,
        enable_event_wait: bool,
        enable_adaptive_spin: bool,
        spin_floor_us: int,
        enable_adaptive_lead: bool,
        estimator_state_json: str | None,
        require_focus: bool,
        focus_guard: Any,
        controls: Any,
        renderer: Any,
        poll_s: float,
        core_warmup_budget_us: int = 200,
        dispatch_lead_us: int = 0,
    ) -> None:
        import sky_player_rs

        native_actions = [
            (
                index,
                str(action.kind),
                int(action.at_us),
                [int(scan_code) for scan_code in action.scan_codes],
                action.reason,
            )
            for index, action in enumerate(actions)
        ]
        self._session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            native_actions,
            list(SKY_15_SCAN_CODES),
            min_hold_us=min_hold_us,
            max_lead_us=max_lead_us,
            dispatch_lead_us=dispatch_lead_us,
            mock_backend=False,
            require_focus=require_focus,
            focus_restore_grace_us=focus_restore_grace_us,
            spin_threshold_us=spin_threshold_us,
            core_warmup_budget_us=core_warmup_budget_us,
            late_pulse_drop_threshold_us=late_pulse_drop_threshold_us,
            same_key_conflict_policy=same_key_conflict_policy,
            telemetry_enabled=telemetry_enabled,
            telemetry_capacity=200_000,
            rt_priority_mode=rt_priority_mode,
            enable_waitable_timer=enable_waitable_timer,
            enable_event_wait=enable_event_wait,
            enable_adaptive_spin=enable_adaptive_spin,
            enable_spin_reprobe=enable_adaptive_spin,
            spin_floor_us=spin_floor_us,
            enable_adaptive_lead=enable_adaptive_lead,
            estimator_state_json=estimator_state_json,
        )
        self._actions = actions
        self._song_name = song_name
        self._min_hold_us = min_hold_us
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

    def _publish_focus(self) -> bool:
        if not self._require_focus:
            if self._last_focus_active is not True:
                self._session.update_focus(True)
                self._last_focus_active = True
            return True
        active = bool(self._focus_guard.is_active())
        if active:
            try:
                from sky_music.platform.win32 import inputs

                hwnd = int(inputs.sky) if inputs.sky is not None else 0
            except (AttributeError, TypeError, ValueError):
                hwnd = 0
            if hwnd != self._last_hwnd:
                self._session.set_target_hwnd(hwnd)
                self._last_hwnd = hwnd
        if active != self._last_focus_active:
            self._session.update_focus(active)
            self._last_focus_active = active
        return active

    def _set_initial_target(self) -> None:
        if not self._require_focus:
            return
        try:
            from sky_music.platform.win32 import inputs

            inputs.reset_window_cache()
            hwnd = inputs.get_sky_window()
            target = 0 if hwnd is None else int(hwnd)
            self._session.set_target_hwnd(target)
            self._last_hwnd = target
        except (AttributeError, OSError, RuntimeError, TypeError, ValueError):
            self._session.set_target_hwnd(0)
            self._last_hwnd = 0

    def _handle_command(self, command: str | None) -> str | None:
        if command == "quit":
            self._session.quit()
            return PLAYBACK_QUIT
        if command == "skip":
            self._session.skip()
            return PLAYBACK_SKIPPED
        if command == "pause":
            self._manual_paused = not self._manual_paused
            if self._manual_paused:
                self._session.pause()
            else:
                self._session.resume()
        elif command == "panic":
            self._session.panic_release()
        elif command == "refocus":
            self._focus_guard.focus()
            self._set_initial_target()
        return None

    @staticmethod
    def _health(snapshot: dict[str, Any]) -> BackendHealth:
        return BackendHealth(
            active_count=int(snapshot["active_count"]),
            possibly_active_count=int(snapshot["possibly_active_count"]),
            failed_release_count=int(snapshot["failed_release_count"]),
            last_error=snapshot["last_error"],
            keys_dropped=int(snapshot["keys_dropped"]),
            chord_split_events=int(snapshot["chord_split_events"]),
        )

    def run(self) -> tuple[str, dict[str, Any], dict[str, Any], str | None]:
        """Run supervisor polling until the native worker reaches a terminal state."""
        import time

        self._set_initial_target()
        self._publish_focus()
        self._session.start()
        requested_outcome: str | None = None
        latest: dict[str, Any] = self._session.snapshot()

        while not latest["is_finished"]:
            command = self._controls.poll() if self._controls is not None else None
            requested_outcome = self._handle_command(command) or requested_outcome
            self._publish_focus()
            latest = self._session.snapshot()

            if self._renderer is not None:
                if hasattr(self._renderer, "update_counters_batch"):
                    self._renderer.update_counters_batch(
                        ProgressCounters(
                            max_lateness_us=int(latest["max_lateness_us"]),
                            late_2ms=int(latest["late_2ms"]),
                            late_5ms=int(latest["late_5ms"]),
                            late_10ms=int(latest["late_10ms"]),
                            release_max_us=int(latest["release_max_us"]),
                            release_late_2ms=int(latest["release_late_2ms"]),
                            recent_latencies_us=tuple(
                                int(value)
                                for value in latest["recent_latencies_us"]
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
                    input_path_degraded=bool(latest["keys_dropped"]),
                    backend_health=self._health(latest),
                )
            time.sleep(self._sleep_s)

        joined = bool(self._session.join(timeout_ms=2_000))
        latest = self._session.snapshot()
        if not joined:
            outcome = PLAYBACK_SHUTDOWN_TIMEOUT
        elif latest["status"] in {"panicked", "poisoned"}:
            detail = latest.get("terminal_error") or latest["status"]
            raise RuntimeError(f"native dispatch terminated: {detail}")
        else:
            outcome = str(latest.get("outcome") or requested_outcome or PLAYBACK_FINISHED)
            if outcome not in {
                PLAYBACK_FINISHED,
                PLAYBACK_QUIT,
                PLAYBACK_SKIPPED,
                PLAYBACK_SHUTDOWN_TIMEOUT,
            }:
                raise RuntimeError(f"unknown native playback outcome: {outcome}")

        if self._renderer is not None:
            verb = {
                PLAYBACK_FINISHED: "Finished",
                PLAYBACK_QUIT: "Stopped",
                PLAYBACK_SKIPPED: "Skipped",
                PLAYBACK_SHUTDOWN_TIMEOUT: "Shutdown timeout",
            }[outcome]
            self._renderer.finish(f"{verb}: {self._song_name}")
        if not joined:
            # A timed-out/poisoned worker still owns its buffers and handles.
            # Do not inspect or tear down worker-owned state.
            telemetry = {
                "records": [],
                "attempted": 0,
                "accepted": 0,
                "dropped": 0,
                "truncated": False,
            }
            return outcome, latest, telemetry, None
        telemetry = json.loads(self._session.take_telemetry_json())
        return outcome, latest, telemetry, self._session.estimator_state_json()
