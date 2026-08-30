from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

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
    assert audit._is_historical("tests/test_phase9_gui_canonical.py")
    assert not audit._is_historical("scripts/build_portable_release.py")


@pytest.mark.parametrize(
    "value",
    (
        "SKY_PHASE8_RESTART_SELFTEST",
        "__SKY_PHASE8_GUI_SMOKE__",
        "some_phase9_runtime_flag",
        "PHASE8_ARTIFACT_SUMMARY",
        "scripts/build_phase8.py",
    ),
)
def test_embedded_phase_names_are_rejected(value: str) -> None:
    audit = _load()
    assert audit._contains_phase_name(value)
    assert audit._path_has_durable_phase_name(value)


def test_historical_phase_paths_are_not_rejected() -> None:
    audit = _load()
    assert not audit._path_has_durable_phase_name(
        "docs/evidence/desktop-phase8/README.md"
    )
    assert not audit._path_has_durable_phase_name(
        "tests/test_phase9_gui_canonical.py"
    )
