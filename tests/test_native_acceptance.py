from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType

import pytest


def _load_acceptance_module() -> ModuleType:
    path = Path(__file__).parents[1] / "scripts" / "bench_native_acceptance.py"
    spec = importlib.util.spec_from_file_location("native_acceptance_under_test", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ACCEPTANCE = _load_acceptance_module()


def test_real_backend_without_mock_options_uses_zero_mock_latency() -> None:
    assert ACCEPTANCE._resolve_mock_latency_values(
        backend="sendinput",
        mock_base_latency_us=None,
        mock_per_key_latency_us=None,
    ) == (0, 0)


def test_mock_backend_defaults_preserve_latency_model() -> None:
    assert ACCEPTANCE._resolve_mock_latency_values(
        backend="mock",
        mock_base_latency_us=None,
        mock_per_key_latency_us=None,
    ) == (80, 40)


def test_mock_latency_overrides_are_preserved_for_mock_backend() -> None:
    assert ACCEPTANCE._resolve_mock_latency_values(
        backend="mock",
        mock_base_latency_us=100,
        mock_per_key_latency_us=25,
    ) == (100, 25)


@pytest.mark.parametrize(
    ("base_latency_us", "per_key_latency_us"),
    [(0, 1), (1, 0), (80, 40)],
)
def test_explicit_mock_options_are_rejected_for_real_backend(
    base_latency_us: int, per_key_latency_us: int
) -> None:
    with pytest.raises(SystemExit, match="only valid with --backend mock"):
        ACCEPTANCE._resolve_mock_latency_values(
            backend="sendinput",
            mock_base_latency_us=base_latency_us,
            mock_per_key_latency_us=per_key_latency_us,
        )


def test_negative_mock_latency_is_rejected() -> None:
    with pytest.raises(SystemExit, match="non-negative"):
        ACCEPTANCE._resolve_mock_latency_values(
            backend="mock",
            mock_base_latency_us=-1,
            mock_per_key_latency_us=None,
        )
