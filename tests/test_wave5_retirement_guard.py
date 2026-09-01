from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).parents[1]


def _load():
    path = ROOT / "scripts" / "check_wave5_retirement.py"
    spec = importlib.util.spec_from_file_location("check_wave5_retirement", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load Wave 5 retirement guard")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_wave5_product_graph_is_retired() -> None:
    audit = _load()
    assert audit._missing_paths() == []
    assert audit._active_hits() == []


def test_wave5_guard_is_read_only_and_passes() -> None:
    assert _load().main() == 0


def test_wave5_ledger_covers_baseline_exactly_once() -> None:
    assert _load()._ledger_errors() == []
