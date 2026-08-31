from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest


def _checker():
    path = Path(__file__).parents[1] / "scripts" / "check_rust_architecture.py"
    spec = importlib.util.spec_from_file_location("check_rust_architecture", path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _dispatch_fixture(tmp_path: Path, files: set[str]) -> Path:
    dispatch = (
        tmp_path
        / "rust"
        / "crates"
        / "sky_player_rs"
        / "src"
        / "engine"
        / "worker"
        / "dispatch"
    )
    dispatch.mkdir(parents=True)
    for name in files:
        (dispatch / name).write_text("\n", encoding="utf-8")
    return dispatch


CANONICAL_DISPATCH_FILES = {
    "authored.rs",
    "mod.rs",
    "observation.rs",
    "observer.rs",
    "recovery.rs",
    "timing.rs",
    "hold_forensics.rs",
    "observer_wake.rs",
}


def test_checker_accepts_exact_canonical_dispatch_set(tmp_path):
    _dispatch_fixture(tmp_path, CANONICAL_DISPATCH_FILES)
    report = _checker().check_repository(tmp_path)
    assert not report.errors


def test_checker_ignores_dispatch_test_sidecars(tmp_path):
    _dispatch_fixture(
        tmp_path,
        CANONICAL_DISPATCH_FILES | {"authored_tests.rs", "cutoff_tests.rs"},
    )
    report = _checker().check_repository(tmp_path)
    assert not any(
        item.rule in {"unexpected_dispatch_module", "missing_dispatch_module"}
        for item in report.errors
    )


def test_checker_rejects_unexpected_observer_drain_module(tmp_path):
    _dispatch_fixture(tmp_path, CANONICAL_DISPATCH_FILES | {"observer_" + "drain.rs"})
    report = _checker().check_repository(tmp_path)
    assert any(item.rule == "unexpected_dispatch_module" for item in report.errors)


def test_checker_rejects_unexpected_harness_module(tmp_path):
    _dispatch_fixture(tmp_path, CANONICAL_DISPATCH_FILES | {"harness.rs"})
    report = _checker().check_repository(tmp_path)
    assert any(item.rule == "unexpected_dispatch_module" for item in report.errors)


def test_checker_rejects_missing_observer_module(tmp_path):
    _dispatch_fixture(tmp_path, CANONICAL_DISPATCH_FILES - {"observer.rs"})
    report = _checker().check_repository(tmp_path)
    assert any(item.rule == "missing_dispatch_module" for item in report.errors)


def test_checker_rejects_legacy_downs_path(tmp_path):
    _dispatch_fixture(tmp_path, CANONICAL_DISPATCH_FILES)
    legacy = tmp_path / "rust" / "crates" / "sky_player_rs" / "src" / "engine" / "worker"
    (legacy / "downs.rs").write_text("\n", encoding="utf-8")
    report = _checker().check_repository(tmp_path)
    assert any(item.rule == "legacy_dispatch_path" for item in report.errors)


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


def test_checker_rejects_stale_allowlist_path(tmp_path):
    source = tmp_path / "rust" / "crates" / "sky_dispatch_core" / "src"
    config = tmp_path / ".config"
    source.mkdir(parents=True)
    config.mkdir()
    (config / "rust_architecture_allowlist.json").write_text(
        '{"entries":[{"path":"rust/crates/sky_dispatch_core/src/removed.rs",'
        '"rule":"regular_module_lines","reason":"stale",'
        '"expires_phase":"Phase 2"}]}\n',
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="allowlist path does not exist"):
        _checker().check_repository(tmp_path)


def test_checker_treats_python_root_as_ffi_boundary(tmp_path):
    source = tmp_path / "rust" / "crates" / "sky_player_rs" / "src"
    source.mkdir(parents=True)
    (source / "python.rs").write_text(
        "use pyo3::prelude::*;\n",
        encoding="utf-8",
    )

    report = _checker().check_repository(tmp_path)

    assert not any(item.rule == "pyo3_boundary" for item in report.errors)


def test_checker_rejects_runtime_schedule_clone_in_worker(tmp_path):
    worker = tmp_path / "rust" / "crates" / "sky_player_rs" / "src" / "engine"
    worker.mkdir(parents=True)
    (worker / "worker.rs").write_text(
        "let coordinator = RuntimeDispatchCoordinator::try_new_ticks(\n"
        "    schedule.clone(),\n"
        ");\n",
        encoding="utf-8",
    )

    report = _checker().check_repository(tmp_path)

    violations = [item for item in report.errors if item.rule == "runtime_schedule_clone"]
    assert len(violations) == 1
    assert violations[0].message == (
        "production worker must move RuntimeSchedule into the coordinator; "
        "cloning the schedule is forbidden"
    )


def test_checker_accepts_worker_schedule_move(tmp_path):
    worker = tmp_path / "rust" / "crates" / "sky_player_rs" / "src" / "engine"
    worker.mkdir(parents=True)
    (worker / "worker.rs").write_text(
        "let coordinator = RuntimeDispatchCoordinator::try_new_ticks(schedule);\n",
        encoding="utf-8",
    )

    report = _checker().check_repository(tmp_path)

    assert not any(item.rule == "runtime_schedule_clone" for item in report.errors)


def test_checker_rejects_player_adapter_direct_dispatch_dependency(tmp_path):
    crate = tmp_path / "rust" / "crates" / "sky_player_rs"
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        "[package]\nname = 'sky_player_rs'\nversion = '0.1.0'\n"
        "[dependencies]\nsky_dispatch_core = { path = '../sky_dispatch_core' }\n",
        encoding="utf-8",
    )
    (crate / "src").mkdir()
    (crate / "src" / "lib.rs").write_text("pub fn adapter() {}\n", encoding="utf-8")

    report = _checker().check_repository(tmp_path)

    assert any(item.rule == "player_adapter_dependency" for item in report.errors)


def test_checker_rejects_player_adapter_source_dispatch_import(tmp_path):
    crate = tmp_path / "rust" / "crates" / "sky_player_rs"
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        "[package]\nname = 'sky_player_rs'\nversion = '0.1.0'\n",
        encoding="utf-8",
    )
    (crate / "src").mkdir()
    (crate / "src" / "lib.rs").write_text(
        "use sky_dispatch_win32::win32_available;\n", encoding="utf-8"
    )

    report = _checker().check_repository(tmp_path)

    assert any(item.rule == "player_adapter_dependency" for item in report.errors)


def test_checker_rejects_forbidden_sky_app_core_dependency(tmp_path):
    crate = tmp_path / "rust" / "crates" / "sky_app_core"
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        "[package]\nname = 'sky_app_core'\nversion = '0.1.0'\n"
        "[dependencies]\ntauri = '2'\n",
        encoding="utf-8",
    )
    (crate / "src").mkdir()
    (crate / "src" / "lib.rs").write_text("pub fn core() {}\n", encoding="utf-8")

    report = _checker().check_repository(tmp_path)

    assert any(item.rule == "app_core_dependency" for item in report.errors)
