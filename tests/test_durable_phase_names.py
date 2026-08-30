from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).parents[1]


def _load():
    path = ROOT / "scripts" / "audit_durable_phase_names.py"
    spec = importlib.util.spec_from_file_location("audit_durable_phase_names", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load durable phase-name audit")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_durable_runtime_and_release_surfaces_have_no_phase_names() -> None:
    assert _load().find_violations() == []


def test_historical_benchmark_names_remain_explicitly_allowed() -> None:
    audit = _load()
    assert audit._is_historical("scripts/bench_phase6_native_diagnostics.py")
    assert not audit._is_historical("scripts/build_portable_release.py")
