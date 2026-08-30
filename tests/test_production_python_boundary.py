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
    (tmp_path / "docs").mkdir()
    (tmp_path / "node_modules").mkdir()
    (tmp_path / "src" / "runtime.py").write_text("import pyo3\n", encoding="utf-8")
    (tmp_path / "docs" / "history.md").write_text("pyo3\n", encoding="utf-8")
    (tmp_path / "node_modules" / "ignored.py").write_text("pyo3\n", encoding="utf-8")

    references = _load().collect(tmp_path)

    assert [(item.marker, item.path, item.line) for item in references] == [("pyo3", "src/runtime.py", 1)]


def test_current_report_captures_known_transitional_boundaries() -> None:
    module = _load()
    references = module.collect(ROOT)

    assert any(
        item.marker == "pyo3" and item.path == "rust/crates/sky_player_rs/Cargo.toml"
        for item in references
    )
    assert any(
        item.marker == "desktop_ipc"
        and item.path == "src/sky_music/infrastructure/desktop_ipc/server.py"
        for item in references
    )
