from __future__ import annotations

import importlib.util
import sys
from pathlib import Path


def _checker():
    path = Path(__file__).parents[1] / "scripts" / "check_rust_architecture.py"
    spec = importlib.util.spec_from_file_location("check_rust_architecture", path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_checker_enforces_new_unsafe_boundary(tmp_path):
    source = tmp_path / "rust" / "crates" / "sky_dispatch_core" / "src"
    source.mkdir(parents=True)
    (source / "bad.rs").write_text(
        "pub fn unsafe_entry() { unsafe { std::hint::spin_loop() } }\n",
        encoding="utf-8",
    )

    report = _checker().check_repository(tmp_path)

    assert any(item.rule == "unsafe_boundary" for item in report.errors)


def test_checker_accepts_explicit_temporary_allowlist(tmp_path):
    source = tmp_path / "rust" / "crates" / "sky_dispatch_core" / "src"
    config = tmp_path / ".config"
    source.mkdir(parents=True)
    config.mkdir()
    (source / "large.rs").write_text("\n" * 901, encoding="utf-8")
    (config / "rust_architecture_allowlist.json").write_text(
        '{"entries":[{"path":"rust/crates/sky_dispatch_core/src/large.rs",'
        '"rule":"regular_module_lines","reason":"test debt",'
        '"expires_phase":"Phase 2"}]}\n',
        encoding="utf-8",
    )

    report = _checker().check_repository(tmp_path)

    assert not report.errors
    assert any("temporary allowlist" in item.message for item in report.warnings)


def test_checker_treats_python_root_as_ffi_boundary(tmp_path):
    source = tmp_path / "rust" / "crates" / "sky_player_rs" / "src"
    source.mkdir(parents=True)
    (source / "python.rs").write_text(
        "use pyo3::prelude::*;\n",
        encoding="utf-8",
    )

    report = _checker().check_repository(tmp_path)

    assert not any(item.rule == "pyo3_boundary" for item in report.errors)
