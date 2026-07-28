"""Pedantic microbench for the counter aggregation hot path under 3.14t.

This module provides the baseline for the `_observe_exec_result` and `_get_progress_counters`
hot-path candidate. 

Run under the free-threaded interpreter:

    uv run --env-file .env pytest tests/bench_counter_aggregation.py -m slow \
        --benchmark-only --benchmark-warmup=on --benchmark-warmup-iterations=50
"""
from __future__ import annotations

import pytest

from sky_music.orchestration.core.loop import DispatchLoop, ExecutionResult

pytestmark = pytest.mark.slow

@pytest.mark.benchmark(group="counter_aggregation")
def test_bench_counter_aggregation_baseline(benchmark):
    """Measures the overhead of `_observe_exec_result`.
    """
    class DummyHealthMonitor:
        pass

    class DummyWaitStrategy:
        pass

    class DummyCoordinator:
        pass

    class DummySleeper:
        pass
        
    class DummySleepPolicy:
        pass

    # Create a minimal DispatchLoop instance bypassing __init__ if needed,
    # or just initialize what's needed for _observe_exec_result
    loop = DispatchLoop.__new__(DispatchLoop)
    loop._max_lateness_us = 0
    loop._late_2ms = 0
    loop._late_5ms = 0
    loop._late_10ms = 0
    loop._release_max_us = 0
    loop._release_late_2ms = 0
    import collections
    loop._latencies = collections.deque(maxlen=2000)

    exec_result_down = ExecutionResult(
        event_index=0,
        scheduled_us=0,
        actual_us=1000,
        lateness_us=1000,
        send_duration_us=500,
        is_late=True,
        is_critically_late=False,
        kind="down",
        dispatch_completed_us=1000,
        deferred_by_us=0,
        visible_lateness_us=1000,
        sent_scan_codes=(1,),
        skipped_scan_codes=(),
        runtime_outcome="sent",
        applied_lead_us=0,
        send_duration_pure_us=500,
        bookkeeping_us=0,
        dispatch_lateness_us=1000,
    )

    def work():
        loop._observe_exec_result(exec_result_down)
        return loop._late_2ms

    result = benchmark.pedantic(
        work,
        iterations=1,
        rounds=20_000,
        warmup_rounds=200,
    )
    assert result >= 0
