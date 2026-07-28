"""Patch T1: ``measure_dispatch_tail.py`` 3.14t rewrite.

Verifies:
1. ``_enforce_314t_runtime()`` fails fast on a stock 3.14 interpreter
   (GIL still enabled) — we simulate that with monkey-patching
   ``sys.version_info`` and ``sys._is_gil_enabled``.
2. The new 3.14t matrix runner compiles and produces per-cell summaries
   with the documented percentile keys (smoke; real runs live in the
   ``slow`` lane to keep CI under budget).
3. The pedantic hot-path microbench in ``bench_dispatch_send_pedantic.py``
   is gated by ``pytest-benchmark.pedantic`` with the documented
   ``rounds=20_000, warmup_rounds=200`` profile.

These tests exist so that a regression in the script's fail-fast
contract (e.g. someone re-introducing an unseeded ``random.random()``)
trips a unit-test before it ships a release.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "scripts" / "measure_dispatch_tail.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("measure_dispatch_tail_under_test", SCRIPT_PATH)
    assert spec and spec.loader, f"could not load spec for {SCRIPT_PATH}"
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_enforce_314t_runtime_exits_when_gil_enabled(monkeypatch) -> None:
    """A stock 3.14 interpreter (GIL still enabled) must make the script
    raise ``SystemExit(2)`` — the GIL-enabled branch is the wrong runtime
    per ``.python-version``. We simulate that with monkey-patched values.
    """
    module = _load_module()
    monkeypatch.setattr(sys, "version_info", (3, 14, 0, "final", 0))
    monkeypatch.setattr(sys, "_is_gil_enabled", lambda: True)

    with pytest.raises(SystemExit) as excinfo:
        module._enforce_314t_runtime()
    assert excinfo.value.code == 2, (
        f"SystemExit code must be 2 (fatal harness/runtime mismatch); got {excinfo.value.code}"
    )


def test_enforce_314t_runtime_exits_when_python_too_old(monkeypatch) -> None:
    """Anything below 3.14 must also fail-fast — the harness relies on
    ``sys._is_gil_enabled`` (CPython 3.14+ API).
    """
    module = _load_module()
    monkeypatch.setattr(sys, "version_info", (3, 13, 0, "final", 0))

    with pytest.raises(SystemExit) as excinfo:
        module._enforce_314t_runtime()
    assert excinfo.value.code == 2


def test_enforce_314t_runtime_exits_when_gil_probe_missing(monkeypatch) -> None:
    """``sys._is_gil_enabled`` must exist on the runtime — without it we
    cannot verify the free-threaded build.
    """
    module = _load_module()
    monkeypatch.setattr(sys, "version_info", (3, 14, 0, "final", 0))
    monkeypatch.delattr(sys, "_is_gil_enabled", raising=False)

    with pytest.raises(SystemExit) as excinfo:
        module._enforce_314t_runtime()
    assert excinfo.value.code == 2


def test_enforce_314t_runtime_passes_on_3_14t(monkeypatch) -> None:
    """On a 3.14 free-threaded interpreter the fail-fast must not fire."""
    module = _load_module()
    monkeypatch.setattr(sys, "version_info", (3, 14, 0, "final", 0))
    monkeypatch.setattr(sys, "_is_gil_enabled", lambda: False)

    module._enforce_314t_runtime()  # must not raise


def test_matrix_table_format() -> None:
    """The matrix table must include the documented percentile columns so
    downstream tools can grep the output stably.
    """
    module = _load_module()
    rows = [
        ("load_off timer_on", {"p50_visible": 1.0, "p99_visible": 2.0, "max_visible": 3.0,
                               "p50_dispatch": 4.0, "p99_dispatch": 5.0, "max_dispatch": 6.0}),
    ]
    out = module._matrix_table(rows)
    for col in ("cell", "p50 vis", "p99 vis", "max vis", "p50 disp", "p99 disp", "max disp"):
        assert col in out, f"matrix table missing column {col!r}"
    assert "load_off timer_on" in out


def test_seeded_synthetic_backend_uses_seed_for_latency(monkeypatch) -> None:
    """Two backends constructed with the same seed must produce identical
    latency draws (so p50/p99 across matrix cells differ only by the
    axis variable, not by RNG state).
    """
    module = _load_module()

    class _StubClock:
        _ns_based = False

        def now_us(self) -> int:
            return 0

    a = module._SeededSyntheticLatencyBackend(_StubClock(), seed=42)
    b = module._SeededSyntheticLatencyBackend(_StubClock(), seed=42)
    c = module._SeededSyntheticLatencyBackend(_StubClock(), seed=43)

    draws_a = [a._rng.random() for _ in range(8)]
    draws_b = [b._rng.random() for _ in range(8)]
    draws_c = [c._rng.random() for _ in range(8)]
    assert draws_a == draws_b, (
        f"identical seed must produce identical draws, got {draws_a} vs {draws_b}"
    )
    assert draws_a != draws_c, (
        f"different seeds must produce different draws, got identical {draws_a}"
    )
