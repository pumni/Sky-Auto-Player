"""Stress tests for the watchdog subprocess stall / EOF panic contract.

Regression guard for review of main@7c548527 §2 (false-positive stall panic). The legacy
watchdog had only ~250 ms of scheduling slack between the parent's 0.5 s heartbeat write
and the 0.75 s stall threshold, so a single late heartbeat flush could fire a full-15 KEYUP
while the dispatch thread was still mid-note. The hardened watchdog requires multiple
consecutive missed heartbeats (``STALL_AFTER_S = 3 × heartbeat``, ``STALL_TICK_THRESHOLD``
polls of accumulated age) before panicking, while EOF / read-error still release immediately.

We exercise the watchdog's ``main()`` in-process with a controllable fake stdin so we can
schedule heartbeat writes, sustained silence, and EOF deterministically without spawning a
real subprocess. ``send_scan_code_batch`` is recorded (not called for real).
"""
from __future__ import annotations

import io
import sys
import threading
import time
from collections.abc import Callable
from typing import Any

import pytest

import sky_music.watchdog as watchdog


class _FakeStdin:
    """Minimal buffer.read(1)-compatible stdin: producers push bytes, reader pops."""

    def __init__(self) -> None:
        self._buf: bytearray = bytearray()
        self._lock = threading.Lock()
        self._cond = threading.Condition(self._lock)
        self._closed = False
        self._buffer = _FakeBuffer(self)

    def write_bytes(self, data: bytes) -> None:
        with self._cond:
            if self._closed:
                return
            self._buf.extend(data)
            self._cond.notify_all()

    def close(self) -> None:
        with self._cond:
            self._closed = True
            self._cond.notify_all()

    @property
    def buffer(self) -> _FakeBuffer:
        return self._buffer


class _FakeBuffer:
    def __init__(self, parent: _FakeStdin) -> None:
        self._parent = parent

    def read(self, n: int = -1) -> bytes:
        # mimic sys.stdin.buffer.read(1): block until ≥1 byte available or parent closed.
        with self._parent._cond:
            if n == 1:
                while not self._parent._buf and not self._parent._closed:
                    self._parent._cond.wait()
                if not self._parent._buf and self._parent._closed:
                    return b""
                byte = self._parent._buf[:1]
                del self._parent._buf[0:1]
                return bytes(byte)
            while not self._parent._buf and not self._parent._closed:
                self._parent._cond.wait()
            if not self._parent._buf and self._parent._closed:
                return b""
            take = n if n > 0 else len(self._parent._buf)
            data = bytes(self._parent._buf[:take])
            del self._parent._buf[: len(data)]
            return data


def _patch_watchdog(
    monkeypatch: pytest.MonkeyPatch,
    *,
    stall_after_s: float,
    stall_ticks: int,
    poll_s: float,
) -> tuple[list[tuple[Any, Any]], io.StringIO]:
    """Stub send_scan_code_batch + stderr and shorten watchdog timing constants."""
    calls: list[tuple[Any, Any]] = []
    monkeypatch.setattr(
        watchdog,
        "send_scan_code_batch",
        lambda codes, key_up=False: calls.append((tuple(codes), key_up)),
    )
    monkeypatch.setattr(watchdog, "STALL_AFTER_S", stall_after_s)
    monkeypatch.setattr(watchdog, "STALL_TICK_THRESHOLD", stall_ticks)
    monkeypatch.setattr(watchdog, "POLL_INTERVAL_S", poll_s)
    err = io.StringIO()
    monkeypatch.setattr(sys, "stderr", err)
    monkeypatch.setattr(watchdog, "sys", sys)  # ensure watchdog's stderr lookup hits our swap
    return calls, err


def _reader_thread(target: Callable[[], None]) -> threading.Thread:
    return threading.Thread(target=target, name="watchdog-main", daemon=True)


def _wait_until(cond: Callable[[], bool], *, timeout_s: float, interval_s: float = 0.005) -> bool:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if cond():
            return True
        time.sleep(interval_s)
    return cond()


def test_no_short_hiccup_heartbeat_pause_does_not_panic(monkeypatch: pytest.MonkeyPatch) -> None:
    """A single late heartbeat (~ half the old slack, ~250ms) must NOT trigger stall panic.

    Reproduces the legacy false-positive scenario at a much smaller delay than the old 0.75s
    threshold held: the parent sends one byte, then a 0.3s silence, then resumes. The
    hardened watchdog must keep ``panic_release_all`` (i.e., ``send_scan_code_batch``)
    un-touched throughout.
    """
    calls, _err = _patch_watchdog(
        monkeypatch,
        stall_after_s=1.5,
        stall_ticks=4,
        poll_s=0.01,
    )
    fake_stdin = _FakeStdin()
    monkeypatch.setattr(sys, "stdin", fake_stdin)

    main_done = threading.Event()

    def runner() -> None:
        try:
            watchdog.main()
        finally:
            main_done.set()

    t = _reader_thread(runner)
    t.start()

    # Heartbeat 1: write a byte, wait a beat, silence for 0.3s (under any reasonable threshold),
    # then resume. We must NOT see a panic during the silence window or immediately after.
    fake_stdin.write_bytes(b"\x00")
    time.sleep(0.05)
    time.sleep(0.3)  # one late heartbeat — under the old 0.75s threshold too, but legacy hit 250ms slack
    fake_stdin.write_bytes(b"\x00")
    # Give the watchdog a few poll cycles to confirm it did not bank a panic streak.
    time.sleep(0.10)

    # No panic expected yet — we still have a live parent and never reached STALL_AFTER_S.
    assert calls == [], f"watchdog MUST NOT panic on a 0.3s heartbeat pause; got {calls}"

    # Cleanly tear down: close stdin so main() returns.
    fake_stdin.close()
    assert _wait_until(main_done.is_set, timeout_s=2.0), "watchdog main() did not exit after EOF"


def test_sustained_silence_eventually_panics_with_reason(monkeypatch: pytest.MonkeyPatch) -> None:
    """Sustained heartbeat silence exceeding STALL_AFTER_S + tick threshold DOES panic,
    and forensic telemetry records ``panic_reason=stall`` plus heartbeat_age.
    """
    # Shorten constants so the test runs in ~0.2s wall clock.
    calls, err = _patch_watchdog(
        monkeypatch,
        stall_after_s=0.05,
        stall_ticks=2,
        poll_s=0.01,
    )
    fake_stdin = _FakeStdin()
    monkeypatch.setattr(sys, "stdin", fake_stdin)

    main_done = threading.Event()

    def runner() -> None:
        try:
            watchdog.main()
        finally:
            main_done.set()

    t = _reader_thread(runner)
    t.start()

    # Write one heartbeat to seed last_heartbeat, then go silent.
    fake_stdin.write_bytes(b"\x00")
    time.sleep(0.01)

    # Wait for the stall to qualify and panic. STALL_AFTER_S=0.05s + 2 ticks × 0.01s ≥ ~0.07s
    # plus read-loop latency. Poll up to 1.0s for a single panic call.
    panicked = _wait_until(lambda: len(calls) >= 1, timeout_s=1.0)
    assert panicked, "watchdog SHOULD panic after sustained silence"
    codes, key_up = calls[0]
    assert key_up is True
    assert tuple(codes) == tuple(watchdog.SKY_15_SCAN_CODES)

    # Telemetry line must have hit the patched stderr.
    err_text = err.getvalue()
    assert "panic_reason=stall" in err_text, err_text
    assert "heartbeat_age=" in err_text, err_text

    # main() must terminate on its own after the panic.
    assert _wait_until(main_done.is_set, timeout_s=1.0), "watchdog main() did not exit after stall panic"


def test_eof_releases_immediately_with_reason(monkeypatch: pytest.MonkeyPatch) -> None:
    """EOF (parent pipe closed) releases the 15-key chord AND records ``panic_reason=eof``.

    Belt-and-braces duplicate of the parent's own atexit release; idempotent if the parent
    already cleaned up. The legacy watchdog exited without releasing on clean EOF.
    """
    calls, err = _patch_watchdog(
        monkeypatch,
        # Set thresholds so a stall can never fire during this short test.
        stall_after_s=10.0,
        stall_ticks=10_000,
        poll_s=0.01,
    )
    fake_stdin = _FakeStdin()
    monkeypatch.setattr(sys, "stdin", fake_stdin)

    main_done = threading.Event()

    def runner() -> None:
        try:
            watchdog.main()
        finally:
            main_done.set()

    t = _reader_thread(runner)
    t.start()

    # Close the pipe immediately — the read_loop must observe EOF and trigger panic.
    fake_stdin.close()

    panicked = _wait_until(lambda: len(calls) >= 1, timeout_s=1.0)
    assert panicked, "watchdog SHOULD release keys immediately on EOF"
    codes, key_up = calls[0]
    assert key_up is True
    assert tuple(codes) == tuple(watchdog.SKY_15_SCAN_CODES)

    err_text = err.getvalue()
    assert "panic_reason=eof" in err_text, err_text

    assert _wait_until(main_done.is_set, timeout_s=1.0), "watchdog main() did not exit after EOF panic"
