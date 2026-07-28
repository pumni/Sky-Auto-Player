"""Pedantic microbench for the sender-side hot path under 3.14t.

The ChatGPT-web review of commit ``8a47656`` flagged two perf candidates
that remain explicitly **unscheduled** until a 3.14t p50 >= 10% / p99 <= 5%
gate exists:

1. ``WinSendInputBackend._emit`` accepting only ``PlatformSendResult``
   (drop tuple/None/int compatibility shim).
2. Outer tuple in ``_ARRAY_CACHE`` (split down/up, scan-tuple direct key).

This file provides that gate. The hot-path microbench targets
``_lookup_or_build_input_array`` after a prewarm so every iteration is a
cache hit — the dominant sender path once ``prewarm_input_arrays`` has
been called at the top of ``PlaybackEngine.play()``. A future candidate
that claims a p50 >= 10% win MUST run alongside ``bench_dispatch_fidelity.py``
(structural fidelity, 5-iter) and the synthetic ``measure_dispatch_tail.py``
matrix (3.14t) as the third gate; only then does the candidate land.

Verify gate:

    uv run --env-file .env pytest tests/bench_dispatch_send_pedantic.py -m slow \\
        --benchmark-only --benchmark-warmup=on --benchmark-warmup-iterations=50

Skipped on the fast lane (``pytest -m "not slow"``); the ``slow`` marker
is declared in ``pyproject.toml``. The bench runs 20 000 rounds with 200
warm-up rounds to expose the per-iteration cost above warm-up jitter.
"""
from __future__ import annotations

import pytest

from sky_music.platform.win32 import inputs as win32_inputs

pytestmark = pytest.mark.slow


# Fixed input sequence — Sky melodic single-key dominant path. Using a
# single-element tuple forces the cache hit on the down path that
# production emits ~80 % of the time (single-key down dispatch in the
# prewarmed cache). Pedantic microbench keeps the input shape stable
# across rounds so the measurement isolates the lookup work, not
# shape-driven cache build.
_FIXED_SINGLE_KEY_SHAPE: tuple[int, ...] = (0x1E,)  # keyboard 'A'
_FIXED_FLAG_DOWN: int = 0x0008  # KEYEVENTF_SCANCODE (without KEYEVENTF_KEYUP)


def _reset_array_cache_for_bench() -> None:
    """Drop the process-global array cache so ``_lookup_or_build_input_array``
    returns a clean measurement on each bench run. Caller MUST guarantee no
    dispatch thread is sending (matches the production contract enforced by
    ``clear_array_cache`` docstring).
    """
    win32_inputs.clear_array_cache()


def _prewarm_single_key_down() -> None:
    """Prewarm exactly one shape (single-key down) so every iteration of
    ``work()`` is a cache hit. Mirrors what ``PlaybackEngine.play()`` does
    at startup; we keep the bench focused on the lookup hot path, not
    the build hot path.
    """
    win32_inputs.prewarm_input_arrays([(_FIXED_SINGLE_KEY_SHAPE, False)])


@pytest.mark.benchmark(group="array_cache_lookup")
def test_bench_array_cache_lookup_single_key_pedantic(benchmark):
    """Cache-hit path on a prewarmed single-key-down shape.

    This is the dominant sender path once ``prewarm_input_arrays`` runs at
    the top of ``PlaybackEngine.play()``. The pedantic gate runs 20 000
    rounds with 200 warm-up rounds; p50/p99 deltas from this baseline
    are the only signal accepted for hot-path candidates targeting
    ``_ARRAY_CACHE`` or ``_emit`` argument normalisation.
    """
    _reset_array_cache_for_bench()
    _prewarm_single_key_down()

    def work() -> int:
        # ``n`` is the ctypes array length; under prewarm this is always
        # the prebuilt single-key INPUT array (length 1).
        arr = win32_inputs._lookup_or_build_input_array(
            _FIXED_SINGLE_KEY_SHAPE, _FIXED_FLAG_DOWN
        )
        return len(arr)

    result = benchmark.pedantic(
        work,
        iterations=1,
        rounds=20_000,
        warmup_rounds=200,
    )
    assert result == 1, (
        f"single-key prewarm must yield a 1-element INPUT array, got {result}"
    )


@pytest.mark.benchmark(group="array_cache_lookup")
def test_bench_array_cache_lookup_cold_pedantic(benchmark):
    """Cache-miss path: every iteration rebuilds the INPUT array.

    Used as the noise ceiling for the cache-hit bench: a candidate that
    claims a win on the hot path must show a p50 improvement >= 10 %
    relative to this baseline, not just relative to its own warm-noise.
    """
    _reset_array_cache_for_bench()

    def work() -> int:
        arr = win32_inputs._lookup_or_build_input_array(
            _FIXED_SINGLE_KEY_SHAPE, _FIXED_FLAG_DOWN
        )
        # Drop the cache entry immediately so the next iteration is a miss.
        cache_key = (_FIXED_SINGLE_KEY_SHAPE, _FIXED_FLAG_DOWN)
        with win32_inputs._CACHE_LOCK:
            win32_inputs._ARRAY_CACHE.pop(cache_key, None)
        return len(arr)

    result = benchmark.pedantic(
        work,
        iterations=1,
        rounds=20_000,
        warmup_rounds=200,
    )
    assert result == 1
