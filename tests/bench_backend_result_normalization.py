"""Pedantic microbench for the sender-side hot path under 3.14t.

This module provides the baseline for the `WinSendInputBackend._emit`
result normalization hot-path candidate. The candidate proposes dropping
the `tuple/None/int` compatibility shim (which uses `hasattr` or `isinstance`)
and strictly returning `PlatformSendResult`.

Run under the free-threaded interpreter:

    uv run --env-file .env pytest tests/bench_backend_result_normalization.py -m slow \
        --benchmark-only --benchmark-warmup=on --benchmark-warmup-iterations=50
"""
from __future__ import annotations

import pytest

from sky_music.infrastructure.backend import WinSendInputBackend
from sky_music.infrastructure.timing import PerfCounterClock

pytestmark = pytest.mark.slow

@pytest.mark.benchmark(group="backend_emit")
def test_bench_backend_emit_shim_baseline(benchmark, monkeypatch):
    """Measures the overhead of the current _emit shim.
    
    We monkeypatch `send_scan_code_batch_trusted` to return a mock `PlatformSendResult`
    without making actual OS calls, so we isolate the Python overhead of `_emit`.
    """
    class MockPlatformSendResult:
        __slots__ = ("completed_us", "inserted")
        def __init__(self, inserted: int, completed_us: int):
            self.inserted = inserted
            self.completed_us = completed_us

    clock = PerfCounterClock()
    backend = WinSendInputBackend()
    backend.set_clock(clock)
    
    scan_codes = (0x1E,)
    
    # Mock the actual OS call to be an instant return
    def mock_send_trusted(*args, **kwargs):
        return MockPlatformSendResult(inserted=1, completed_us=clock.now_us())
        
    monkeypatch.setattr(backend.inputs_module, "send_scan_code_batch_trusted", mock_send_trusted)

    def work():
        return backend._emit(scan_codes, key_up=False)

    result = benchmark.pedantic(
        work,
        iterations=1,
        rounds=20_000,
        warmup_rounds=200,
    )
    assert result is not None
