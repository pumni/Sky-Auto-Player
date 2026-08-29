"""Bounded Desktop Core calibration lifecycle.

Calibration is deliberately an application service.  The native calibration
adapter remains the only owner of calibration execution and its process/input
boundary; this module only validates intent, serializes one operation, and
publishes bounded progress/results to the desktop protocol.
"""

from __future__ import annotations

import math
import threading
import uuid
from collections.abc import Callable, Mapping
from dataclasses import asdict
from typing import Any, cast

from sky_music.orchestration.desktop_models import (
    CalibrationCancelAckDto,
    CalibrationFinishedDto,
    CalibrationMode,
    CalibrationProgressDto,
    CalibrationStartAckDto,
    CalibrationStartDto,
    CalibrationState,
)

MAX_CALIBRATION_TEXT_BYTES = 4096
MAX_CALIBRATION_SAMPLES = 5_000
MAX_CALIBRATION_TIMEOUT_SECONDS = 120.0
CALIBRATION_JOIN_TIMEOUT_SECONDS = 2.0


class DesktopCalibrationError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


class CalibrationCancelled(RuntimeError):
    """Raised by a cancellable native runner when its operation is stopped."""


CalibrationProgressCallback = Callable[[str, int, int, str], None]
CalibrationRunner = Callable[
    [CalibrationStartDto, threading.Event, CalibrationProgressCallback],
    Mapping[str, Any],
]


def _bounded_text(value: object) -> str:
    text = str(value).replace("\x00", "")
    encoded = text.encode("utf-8", errors="replace")
    if len(encoded) <= MAX_CALIBRATION_TEXT_BYTES:
        return text
    return (
        encoded[: MAX_CALIBRATION_TEXT_BYTES - 3].decode("utf-8", errors="ignore")
        + "..."
    )


def _validate_operation_id(operation_id: object) -> str:
    if (
        not isinstance(operation_id, str)
        or len(operation_id) != 32
        or any(char not in "0123456789abcdef" for char in operation_id)
    ):
        raise DesktopCalibrationError(
            "invalid_params", "operation_id must be an opaque ID"
        )
    return operation_id


def _validate_start(params: Mapping[str, object]) -> CalibrationStartDto:
    allowed = {"mode", "class_name", "polyphony", "samples", "timeout_seconds"}
    unknown = set(params) - allowed
    if unknown:
        raise DesktopCalibrationError(
            "invalid_params",
            f"unknown calibration params: {', '.join(sorted(unknown))}",
        )
    raw_mode = params.get("mode", "quick")
    if raw_mode not in {"quick", "full", "diagnostic"}:
        raise DesktopCalibrationError(
            "invalid_params", "mode must be quick, full, or diagnostic"
        )
    mode = cast(CalibrationMode, raw_mode)
    class_name = params.get("class_name")
    if class_name is not None and class_name not in {"hot", "cold"}:
        raise DesktopCalibrationError(
            "invalid_params", "class_name must be hot or cold"
        )
    polyphony = params.get("polyphony")
    if polyphony is not None and (
        type(polyphony) is not int or polyphony not in {1, 5, 15}
    ):
        raise DesktopCalibrationError("invalid_params", "polyphony must be 1, 5, or 15")
    samples = params.get("samples")
    if samples is not None and (
        type(samples) is not int or not 1 <= samples <= MAX_CALIBRATION_SAMPLES
    ):
        raise DesktopCalibrationError(
            "invalid_params", "samples must be an integer between 1 and 5000"
        )
    timeout = params.get("timeout_seconds")
    if timeout is not None and (
        isinstance(timeout, bool)
        or not isinstance(timeout, (int, float))
        or not math.isfinite(float(timeout))
        or not 0 < float(timeout) <= MAX_CALIBRATION_TIMEOUT_SECONDS
    ):
        raise DesktopCalibrationError(
            "invalid_params", "timeout_seconds must be finite and in (0, 120]"
        )
    if mode == "diagnostic" and (
        class_name is None or polyphony is None or samples is None
    ):
        raise DesktopCalibrationError(
            "invalid_params",
            "diagnostic mode requires class_name, polyphony, and samples",
        )
    return CalibrationStartDto(
        mode=mode,
        class_name=class_name if isinstance(class_name, str) else None,
        polyphony=polyphony if type(polyphony) is int else None,
        samples=samples if type(samples) is int else None,
        timeout_seconds=float(timeout) if timeout is not None else None,
    )


class DesktopCalibrationService:
    """One-at-a-time cancellable calibration operation."""

    def __init__(
        self,
        *,
        publish_event: Callable[[str, dict[str, object]], None],
        physical_playback_active: Callable[[], bool],
        on_success: Callable[[], None] | None = None,
        runner: CalibrationRunner | None = None,
    ) -> None:
        self._publish_event = publish_event
        self._physical_playback_active = physical_playback_active
        self._on_success = on_success
        self._runner = runner or self._run_native
        self._lock = threading.RLock()
        self._state: CalibrationState = "idle"
        self._operation_id: str | None = None
        self._cancel: threading.Event | None = None
        self._worker: threading.Thread | None = None

    @property
    def state(self) -> CalibrationState:
        with self._lock:
            return self._state

    @property
    def operation_id(self) -> str | None:
        with self._lock:
            return self._operation_id

    def start(self, params: Mapping[str, object]) -> dict[str, object]:
        request = _validate_start(params)
        with self._lock:
            if self._state in {"starting", "running", "cancelling"}:
                raise DesktopCalibrationError(
                    "already_running", "a calibration operation is already active"
                )
            if self._physical_playback_active():
                raise DesktopCalibrationError(
                    "playback_active", "calibration cannot run during physical playback"
                )
            operation_id = uuid.uuid4().hex
            cancel = threading.Event()
            self._operation_id = operation_id
            self._cancel = cancel
            self._state = "running"
            self._progress_locked(
                operation_id, "starting", 0, 1, "Starting calibration"
            )
            worker = threading.Thread(
                target=self._run,
                args=(operation_id, request, cancel),
                name="desktop-calibration",
                daemon=True,
            )
            self._worker = worker
            worker.start()
            return asdict(CalibrationStartAckDto(operation_id, "running"))

    def cancel(self, operation_id: object) -> dict[str, object]:
        requested = _validate_operation_id(operation_id)
        with self._lock:
            if requested != self._operation_id:
                raise DesktopCalibrationError(
                    "stale_operation", "calibration operation is stale"
                )
            if self._state in {"starting", "running"}:
                assert self._cancel is not None
                self._cancel.set()
                self._state = "cancelling"
                return asdict(CalibrationCancelAckDto(requested, "cancelling", True))
            if self._state == "cancelling":
                return asdict(CalibrationCancelAckDto(requested, "cancelling", True))
            return asdict(CalibrationCancelAckDto(requested, self._state, False))

    def shutdown(self) -> bool:
        """Request cancellation and wait a bounded time during Core shutdown."""
        with self._lock:
            worker = self._worker
            if worker is None or not worker.is_alive():
                return True
            if self._cancel is not None:
                self._cancel.set()
            self._state = "cancelling"
        worker.join(timeout=CALIBRATION_JOIN_TIMEOUT_SECONDS)
        return not worker.is_alive()

    def _progress_locked(
        self, operation_id: str, phase: str, completed: int, total: int, message: str
    ) -> None:
        payload = asdict(
            CalibrationProgressDto(
                operation_id=operation_id,
                state=self._state,
                phase=_bounded_text(phase),
                completed=max(0, min(int(completed), max(1, min(int(total), 10_000)))),
                total=max(1, min(int(total), 10_000)),
                message=_bounded_text(message),
            )
        )
        self._publish_event("calibration.progress", payload)

    def _progress(
        self, operation_id: str, phase: str, completed: int, total: int, message: str
    ) -> None:
        with self._lock:
            if operation_id != self._operation_id or self._state not in {
                "running",
                "cancelling",
            }:
                return
            self._progress_locked(operation_id, phase, completed, total, message)

    @staticmethod
    def _run_native(
        request: CalibrationStartDto,
        cancel: threading.Event,
        progress: CalibrationProgressCallback,
    ) -> Mapping[str, Any]:
        if cancel.is_set():
            raise CalibrationCancelled()
        from sky_music.platform.win32.native_calibration import run_native_calibration

        progress("measuring", 0, 1, "Measuring sender timing")
        if request.mode == "diagnostic":
            result = run_native_calibration(
                mode=request.mode,
                timeout_seconds=request.timeout_seconds,
                cancel_event=cancel,
                class_name=request.class_name,
                polyphony=request.polyphony,
                samples=request.samples,
            )
        else:
            result = run_native_calibration(
                mode=request.mode,
                timeout_seconds=request.timeout_seconds,
                cancel_event=cancel,
            )
        if cancel.is_set():
            raise CalibrationCancelled()
        progress("applying", 1, 1, "Validating calibration result")
        return result

    def _run(
        self, operation_id: str, request: CalibrationStartDto, cancel: threading.Event
    ) -> None:
        try:
            result = self._runner(
                request, cancel, lambda *args: self._progress(operation_id, *args)
            )
            if cancel.is_set():
                raise CalibrationCancelled()
            raw_status = result.get("status", "completed")
            status = _bounded_text(raw_status)
            margin = result.get(
                "transport_margin_us", result.get("applied_transport_margin_us")
            )
            margin_us = margin if type(margin) is int and margin >= 0 else None
            sample_count = result.get(
                "sample_count", result.get("measured_attempts", 0)
            )
            if type(sample_count) is not int or sample_count < 0:
                sample_count = 0
            source = _bounded_text(result.get("source", "native"))
            self._finish(
                operation_id,
                outcome="succeeded",
                status=status,
                margin_us=margin_us,
                sample_count=sample_count,
                source=source,
                message="Calibration completed successfully.",
                applied=False,
            )
        except CalibrationCancelled:
            self._finish(
                operation_id,
                outcome="cancelled",
                status="cancelled",
                margin_us=None,
                sample_count=0,
                source="none",
                message="Calibration was cancelled.",
                applied=False,
            )
        except Exception as exc:
            if cancel.is_set():
                self._finish(
                    operation_id,
                    outcome="cancelled",
                    status="cancelled",
                    margin_us=None,
                    sample_count=0,
                    source="none",
                    message="Calibration was cancelled.",
                    applied=False,
                )
                return
            self._finish(
                operation_id,
                outcome="failed",
                status="failed",
                margin_us=None,
                sample_count=0,
                source="none",
                message=_bounded_text(exc),
                applied=False,
            )

    def _finish(
        self,
        operation_id: str,
        *,
        outcome: str,
        status: str,
        margin_us: int | None,
        sample_count: int,
        source: str,
        message: str,
        applied: bool,
    ) -> None:
        with self._lock:
            if operation_id != self._operation_id or self._state in {
                "succeeded",
                "failed",
                "cancelled",
            }:
                return
            final_state: CalibrationState = {
                "succeeded": "succeeded",
                "failed": "failed",
                "cancelled": "cancelled",
            }[outcome]  # type: ignore[index]
            self._state = final_state
            if outcome == "succeeded" and self._on_success is not None:
                self._on_success()
            self._publish_event(
                "calibration.finished",
                asdict(
                    CalibrationFinishedDto(
                        operation_id=operation_id,
                        outcome=outcome,  # type: ignore[arg-type]
                        status=status,
                        margin_us=margin_us,
                        sample_count=sample_count,
                        source=source,
                        message=_bounded_text(message),
                        applied=applied,
                    )
                ),
            )
            self._worker = None


__all__ = [
    "CALIBRATION_JOIN_TIMEOUT_SECONDS",
    "DesktopCalibrationError",
    "DesktopCalibrationService",
]
