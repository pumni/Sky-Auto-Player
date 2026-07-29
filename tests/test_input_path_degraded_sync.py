"""Regression tests for the threading.Event-backed ``input_path_degraded`` flag
(review of main@7c548527 §3 — "Cross-thread HUD flag not synchronised").

The legacy code stored ``_input_path_degraded`` as a bare ``bool`` attribute on
``DispatchHealthMonitor``, written by the dispatch thread inside
``record_input_path_send_duration`` and read by the supervisor thread at publish time
via the ``input_path_degraded`` property. Documentation for CPython free-threading
recommends explicit synchronization primitives over relying on per-implementation
atomicity of bare attribute reads; we now back the flag with a ``threading.Event``
so cross-thread visibility is a documented invariant rather than an interpreter quirk.

Behaviour preserved:
  * Default state is False (event not set).
  * The flag trips only after the 1-second sustained-warn window completes.
  * Once tripped it stays True — the path is monotonic.

Cross-thread property:
  * A second thread reading ``input_path_degraded`` after the writer thread set it
    observes True (no torn read / no missing write on this architecture).
"""
from __future__ import annotations

import threading
import time
from unittest.mock import Mock

from sky_music.orchestration.core.loop import DispatchHealthMonitor


def _make(input_path_warn_us: int = 300) -> DispatchHealthMonitor:
    clock = Mock()
    clock.now_us.return_value = 0
    return DispatchHealthMonitor(
        backend=Mock(),
        clock=clock,
        focus_guard=Mock(),
        require_focus=False,
        input_path_warn_us=input_path_warn_us,
    )


def test_input_path_degraded_defaults_false() -> None:
    mon = _make()
    assert mon.input_path_degraded is False


def test_input_path_degraded_does_not_trip_before_one_second_window() -> None:
    mon = _make(input_path_warn_us=300)
    # Drive the 64-sample window fully over the warn threshold for ~1 second of "elapsed"
    # without ever crossing the 1s sustained band — the flag must NOT trip.
    for elapsed_us in range(0, 1_000_000, 1_000):
        # send_duration_us > warn so send_over_warn_count stays at the high-count branch,
        # but the trip only happens at the END of the warn window (1s sustained). We stop
        # just before the window closes so the flag must still be False.
        mon.record_input_path_send_duration(send_duration_us=400, elapsed_us=elapsed_us)
    assert mon.input_path_degraded is False


def test_input_path_degraded_trips_after_one_second_sustained_warn() -> None:
    mon = _make(input_path_warn_us=300)
    # Sustained >warn sends across ≥1.0 s of elapsed time, all within the warn-band of
    # the window's occupants (window is maxlen=64, default ≥95%-over-warn trips warn).
    for elapsed_us in range(0, 1_010_000, 1_000):
        mon.record_input_path_send_duration(send_duration_us=400, elapsed_us=elapsed_us)
    assert mon.input_path_degraded is True


def test_input_path_degraded_is_monotonic_once_set_never_clears() -> None:
    mon = _make(input_path_warn_us=300)
    for elapsed_us in range(0, 1_010_000, 1_000):
        mon.record_input_path_send_duration(send_duration_us=400, elapsed_us=elapsed_us)
    assert mon.input_path_degraded is True
    # Recover — push a long run of sub-warn samples. The flag must NOT clear.
    for elapsed_us in range(1_010_000, 5_010_000, 1_000):
        mon.record_input_path_send_duration(send_duration_us=10, elapsed_us=elapsed_us)
    assert mon.input_path_degraded is True


def test_input_path_degraded_value_is_visible_from_a_second_thread_after_trip() -> None:
    """The threading.Event backing the flag makes the trip observable cross-thread.

    We spin a worker thread that polls ``input_path_degraded`` until it sees True. If the
    backend Event failed to publish across threads, the worker would hang — the test
    bounds that with a deadline.
    """
    mon = _make(input_path_warn_us=300)
    observed = threading.Event()

    def reader() -> None:
        # Spin reading the property — the property delegates to Event.is_set(),
        # a documented synchronization primitive, so visibility is guaranteed.
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            if mon.input_path_degraded:
                observed.set()
                return
            time.sleep(0.001)

    reader_thread = threading.Thread(target=reader, name="degraded-reader", daemon=True)
    reader_thread.start()

    for elapsed_us in range(0, 1_010_000, 1_000):
        mon.record_input_path_send_duration(send_duration_us=400, elapsed_us=elapsed_us)

    # The reader must observe the flag without an explicit memory barrier — the Event IS
    # the barrier. Without the Event backing this would still pass on CPython today, but
    # the contract is the Event's documented cross-thread visibility guarantee.
    assert observed.wait(timeout=1.0), (
        "reader thread did not observe input_path_degraded=True within the timeout"
    )
    reader_thread.join(timeout=1.0)
