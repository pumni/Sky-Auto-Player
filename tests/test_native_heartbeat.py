"""Heartbeat-thread fault propagation tests."""

from __future__ import annotations

import time

from sky_music.orchestration.native_dispatch import NativeHeartbeatThread


class _FailingSession:
    def heartbeat(self) -> None:
        raise RuntimeError("binding failed")


def test_native_heartbeat_keeps_original_exception() -> None:
    thread = NativeHeartbeatThread(_FailingSession(), interval_s=0.001)
    thread.start()
    thread.join(timeout=1.0)

    assert not thread.is_alive()
    assert isinstance(thread.error, RuntimeError)
    assert str(thread.error) == "binding failed"


def test_native_heartbeat_stops_without_error() -> None:
    class Session:
        def heartbeat(self) -> None:
            return None

    thread = NativeHeartbeatThread(Session(), interval_s=0.001)
    thread.start()
    time.sleep(0.005)
    thread.stop()
    thread.join(timeout=1.0)

    assert not thread.is_alive()
    assert thread.error is None
