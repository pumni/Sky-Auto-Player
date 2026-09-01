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
    assert Path("scripts/build_portable_release.py") in audit.ACTIVE_FILES
    assert Path(".github/workflows/ci.yml") in audit.ACTIVE_FILES
    assert Path(".github/workflows/release.yml") in audit.ACTIVE_FILES
    assert "build_rust_wheel.py" in audit.FORBIDDEN_TOKENS


def test_wave5_guard_catches_retired_builder_tokens() -> None:
    audit = _load()
    assert "maturin" in audit._find_forbidden_tokens("cargo run build_rust_wheel.py maturin")
    assert "pyinstaller" in audit._find_forbidden_tokens("PyInstaller Core packaging")


def test_wave5_guard_catches_retired_workflow_tokens() -> None:
    audit = _load()
    assert "build_rust_wheel.py" in audit._find_forbidden_tokens(
        "python scripts/build_rust_wheel.py"
    )
    assert "sky_player_rs" in audit._find_forbidden_tokens(
        "install sky_player_rs wheel"
    )


def test_wave5_guard_is_read_only_and_passes() -> None:
    assert _load().main() == 0


def test_wave5_ledger_covers_baseline_exactly_once() -> None:
    assert _load()._ledger_errors() == []


def test_wave5_ledger_rejects_missing_concrete_evidence() -> None:
    audit = _load()
    errors = audit._evidence_errors(
        {
            "path": "tests/deleted.py",
            "classification": "MIGRATED",
            "invariants": ["an invariant"],
            "evidence": ["rust/missing.rs::missing_test"],
        }
    )
    assert any("evidence target is missing" in error for error in errors)


def test_wave5_ledger_rejects_placeholder_evidence() -> None:
    audit = _load()
    errors = audit._evidence_errors(
        {
            "path": "tests/deleted.py",
            "classification": "DUPLICATE",
            "invariants": ["an invariant"],
            "evidence": ["named native/frontend/updater tests cover the retired product invariant"],
        }
    )
    assert any("placeholder evidence" in error for error in errors)


def test_wave5_ledger_accepts_existing_symbol_evidence() -> None:
    audit = _load()
    assert audit._evidence_errors(
        {
            "path": "tests/deleted.py",
            "classification": "FIXTURE_FROZEN",
            "invariants": ["committed fixture semantics"],
            "evidence": [
                "rust/crates/sky_app_core/tests/wave2_fixtures.rs::catalog_fixture_preserves_ids_order_normalization_and_generation"
            ],
        }
    ) == []
