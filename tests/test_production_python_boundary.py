from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).parents[1]


def _load():
    path = ROOT / "scripts" / "report_production_python_boundary.py"
    spec = importlib.util.spec_from_file_location("report_production_python_boundary", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load production boundary reporter")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_report_scans_only_production_surfaces(tmp_path: Path) -> None:
    (tmp_path / "src").mkdir()
    (tmp_path / "scripts").mkdir()
    (tmp_path / "docs").mkdir()
    (tmp_path / "node_modules").mkdir()
    (tmp_path / "rust" / "tests").mkdir(parents=True)
    (tmp_path / "rust" / "src").mkdir(parents=True)
    (tmp_path / ".github" / "workflows").mkdir(parents=True)
    (tmp_path / ".github" / "ISSUE_TEMPLATE").mkdir(parents=True)
    (tmp_path / ".github" / "PULL_REQUEST_TEMPLATE.md").write_text("pyo3\\n", encoding="utf-8")
    (tmp_path / "src" / "runtime.py").write_text("import pyo3\\n", encoding="utf-8")
    (tmp_path / "scripts" / "report_production_python_boundary.py").write_text("pyo3\\n", encoding="utf-8")
    (tmp_path / "docs" / "history.md").write_text("pyo3\\n", encoding="utf-8")
    (tmp_path / "node_modules" / "ignored.py").write_text("pyo3\\n", encoding="utf-8")
    (tmp_path / "rust" / "tests" / "ignored.rs").write_text("pyo3\\n", encoding="utf-8")
    (tmp_path / "rust" / "src" / "test_session.rs").write_text("pyo3\\n", encoding="utf-8")
    (tmp_path / ".github" / "workflows" / "ci.yml").write_text("pyo3\\n", encoding="utf-8")
    (tmp_path / ".github" / "ISSUE_TEMPLATE" / "config.yml").write_text("pyo3\\n", encoding="utf-8")

    references = _load().collect(tmp_path)
    assert [(item.marker, item.path, item.line) for item in references] == [
        ("pyo3", ".github/workflows/ci.yml", 1),
        ("pyo3", "src/runtime.py", 1),
    ]


def test_current_report_is_runtime_python_zero() -> None:
    module = _load()
    references = module.collect(ROOT)
    payload = module._payload(ROOT, references)
    accounting = payload["python_boundary"]
    ownership = accounting["command_ownership"]["after"]
    assert ownership["python_count"] == 0
    assert ownership["native_count"] == 21
    assert accounting["python_core_process_required"] is False
    assert accounting["python_runtime_shipped"] is False
    assert accounting["production_runtime_python_boundary"] == "zero"
    assert accounting["pyinstaller_required_for_portable_desktop"] is False
    assert accounting["pyo3_required_for_production_tauri_playback"] is False
    retained = accounting["retained_repository_material"]
    assert "removed" in retained["pyo3_maturin"]
    assert "retired" in retained["textual_source"]
