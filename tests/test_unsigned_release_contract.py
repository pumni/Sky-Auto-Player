from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from types import ModuleType

import pytest

ROOT = Path(__file__).parents[1]


def _load_manifest_verifier() -> ModuleType:
    path = ROOT / "scripts" / "verify_release_manifest.py"
    spec = importlib.util.spec_from_file_location("verify_release_manifest", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load release manifest verifier")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _common():
    scripts = str(ROOT / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    import release_common

    return release_common


def test_release_workflow_keeps_native_integrity_gates() -> None:
    workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    for forbidden in (
        "AUTHENTICODE_PFX_BASE64",
        "signtool",
        "verify_release_signatures.ps1",
        "build_rust_wheel.py",
        "maturin",
        "PyInstaller",
    ):
        assert forbidden not in workflow
    for required in (
        "uv run python scripts/check.py static",
        "uv run python scripts/check.py rust",
        "scripts/verify_release_manifest.py",
        "actions/attest-build-provenance@",
        "MANIFEST.json",
        "scripts/build_portable_release.py",
    ):
        assert required in workflow


def test_native_package_paths_pin_rust_compiler_explicitly() -> None:
    ci_workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    release_workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    assert "Expected stable Rust 1.98.0" in ci_workflow
    assert "RUSTUP_TOOLCHAIN: 1.98.0" in ci_workflow
    assert "RUSTUP_TOOLCHAIN: 1.98.0" in release_workflow
    assert "scripts/build_portable_release.py" in ci_workflow
    assert "scripts/build_portable_release.py" in release_workflow


def test_release_tree_is_exact_and_hash_verified(tmp_path: Path) -> None:
    verifier = _load_manifest_verifier()
    release_dir = tmp_path / "release"
    release_dir.mkdir()
    for name, payload in (
        ("Sky-Auto-Player.exe", b"app"),
        ("native_calibration.exe", b"calibration"),
        ("Sky-Auto-Player-Updater.exe", b"updater"),
    ):
        (release_dir / name).write_bytes(payload)
    _common().write_release_manifest(release_dir, "3.2.1", "Sky-Auto-Player.exe", "a" * 40)
    verifier.verify(release_dir, "3.2.1")
    (release_dir / "Sky-Auto-Player-Updater.exe").write_bytes(b"tampered")
    with pytest.raises(RuntimeError, match="manifest hash/size mismatch"):
        verifier.verify(release_dir, "3.2.1")


@pytest.mark.parametrize(
    "legacy_name",
    ["Sky-Player.exe", "updater.bat", "updater.ps1", "sky_updater_e2e.exe"],
)
def test_release_tree_rejects_legacy_update_artifacts(tmp_path: Path, legacy_name: str) -> None:
    verifier = _load_manifest_verifier()
    release_dir = tmp_path / "release"
    release_dir.mkdir()
    for name in ("Sky-Auto-Player.exe", "native_calibration.exe", "Sky-Auto-Player-Updater.exe"):
        (release_dir / name).write_bytes(b"native")
    _common().write_release_manifest(release_dir, "3.2.1", "Sky-Auto-Player.exe", "a" * 40)
    (release_dir / legacy_name).write_bytes(b"legacy")
    with pytest.raises(RuntimeError, match="forbidden artifacts"):
        verifier.verify(release_dir, "3.2.1")


@pytest.mark.parametrize(
    "relative",
    ["Sky-Auto-Player-Core.exe", "_internal/python314.dll", "native.pyd", "python314.dll", "legacy.py"],
)
def test_release_tree_rejects_retired_python_runtime(tmp_path: Path, relative: str) -> None:
    verifier = _load_manifest_verifier()
    release_dir = tmp_path / "release"
    release_dir.mkdir()
    for name in ("Sky-Auto-Player.exe", "native_calibration.exe", "Sky-Auto-Player-Updater.exe"):
        (release_dir / name).write_bytes(b"native")
    _common().write_release_manifest(release_dir, "3.2.1", "Sky-Auto-Player.exe", "a" * 40)
    path = release_dir / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"retired runtime")
    with pytest.raises(RuntimeError, match="bundled Python runtime artifacts"):
        verifier.verify(release_dir, "3.2.1")
