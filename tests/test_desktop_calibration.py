from __future__ import annotations

import threading
import time
from collections.abc import Mapping

import pytest

from sky_music.orchestration.desktop_calibration import (
    DesktopCalibrationError,
    DesktopCalibrationService,
)
from sky_music.orchestration.desktop_models import CalibrationStartDto


def _wait_for(predicate) -> None:
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.005)
    assert predicate()


def test_calibration_success_emits_progress_and_one_terminal_event() -> None:
    events: list[tuple[str, dict[str, object]]] = []

    def runner(
        request: CalibrationStartDto, cancel: threading.Event, progress
    ) -> Mapping[str, object]:
        assert request.mode == "quick"
        assert not cancel.is_set()
        progress("measuring", 1, 2, "sample")
        progress("measuring", 99, 2, "bounded")
        return {"status": "ready", "sample_count": 12, "transport_margin_us": 800}

    service = DesktopCalibrationService(
        publish_event=lambda name, payload: events.append((name, payload)),
        physical_playback_active=lambda: False,
        runner=runner,
    )
    ack = service.start({"mode": "quick"})
    _wait_for(lambda: service.state == "succeeded")

    assert ack["state"] == "running"
    assert [name for name, _ in events].count("calibration.finished") == 1
    progress = [payload for name, payload in events if name == "calibration.progress"]
    assert progress[-1]["completed"] == 2
    finished = next(
        payload for name, payload in events if name == "calibration.finished"
    )
    assert finished["outcome"] == "succeeded"
    assert finished["sample_count"] == 12


def test_calibration_cancel_is_idempotent_and_stale_ids_fail_closed() -> None:
    entered = threading.Event()
    release = threading.Event()
    events: list[tuple[str, dict[str, object]]] = []

    def runner(
        _request: CalibrationStartDto, cancel: threading.Event, _progress
    ) -> Mapping[str, object]:
        entered.set()
        while not release.wait(0.01):
            if cancel.is_set():
                return {}
        return {"status": "ready"}

    service = DesktopCalibrationService(
        publish_event=lambda name, payload: events.append((name, payload)),
        physical_playback_active=lambda: False,
        runner=runner,
    )
    operation_id = str(service.start({"mode": "quick"})["operation_id"])
    _wait_for(entered.is_set)
    assert service.cancel(operation_id)["accepted"] is True
    assert service.cancel(operation_id)["accepted"] is True
    with pytest.raises(DesktopCalibrationError, match="stale"):
        service.cancel("f" * 32)
    release.set()
    _wait_for(lambda: service.state == "cancelled")

    finished = [payload for name, payload in events if name == "calibration.finished"]
    assert len(finished) == 1
    assert finished[0]["outcome"] == "cancelled"


def test_calibration_rejects_duplicate_and_active_playback() -> None:
    entered = threading.Event()
    release = threading.Event()

    def runner(
        _request: CalibrationStartDto, _cancel: threading.Event, _progress
    ) -> Mapping[str, object]:
        entered.set()
        release.wait(1)
        return {"status": "ready"}

    service = DesktopCalibrationService(
        publish_event=lambda _name, _payload: None,
        physical_playback_active=lambda: False,
        runner=runner,
    )
    operation_id = str(service.start({"mode": "quick"})["operation_id"])
    _wait_for(entered.is_set)
    with pytest.raises(DesktopCalibrationError, match="already active"):
        service.start({"mode": "quick"})
    release.set()
    _wait_for(lambda: service.state == "succeeded")

    blocked = DesktopCalibrationService(
        publish_event=lambda _name, _payload: None,
        physical_playback_active=lambda: True,
        runner=runner,
    )
    with pytest.raises(DesktopCalibrationError, match="physical playback"):
        blocked.start({"mode": "quick"})
    assert operation_id


@pytest.mark.parametrize(
    "params",
    [
        {"mode": "nope"},
        {"mode": "diagnostic", "class_name": "hot", "polyphony": 5},
        {"mode": "quick", "timeout_seconds": float("nan")},
        {"mode": "quick", "samples": 0},
    ],
)
def test_calibration_start_validates_boundary(params: dict[str, object]) -> None:
    service = DesktopCalibrationService(
        publish_event=lambda _name, _payload: None,
        physical_playback_active=lambda: False,
        runner=lambda *_args: {"status": "ready"},
    )
    with pytest.raises(DesktopCalibrationError):
        service.start(params)
