from __future__ import annotations

import tomllib
from pathlib import Path

ROOT = Path(__file__).parents[1]


def test_pure_player_has_no_python_or_delivery_dependency() -> None:
    manifest = tomllib.loads(
        (ROOT / "rust" / "crates" / "sky_player" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
    )
    assert "pyo3" not in manifest.get("dependencies", {})
    source = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "rust" / "crates" / "sky_player" / "src").rglob("*.rs"))
    )
    assert "pyo3" not in source
    assert "tauri" not in source
    assert not (ROOT / "rust" / "crates" / "sky_player_rs" / "src" / "engine").exists()


def test_python_wheel_adapter_depends_on_pure_player() -> None:
    manifest = tomllib.loads(
        (ROOT / "rust" / "crates" / "sky_player_rs" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
    )
    dependencies = manifest["dependencies"]
    assert dependencies["sky_player"]["path"] == "../sky_player"
    assert "pyo3" in dependencies
    assert "sky_dispatch_core" not in dependencies
    assert "sky_dispatch_win32" not in dependencies

    adapter_source = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted(
            (ROOT / "rust" / "crates" / "sky_player_rs" / "src").rglob("*.rs")
        )
    )
    assert "sky_dispatch_core" not in adapter_source
    assert "sky_dispatch_win32" not in adapter_source
    assert "sky_player::adapter_support" in adapter_source


def test_player_adapter_boundary_and_assembly_audit_are_explicit() -> None:
    facade_source = (ROOT / "rust" / "crates" / "sky_player" / "src" / "lib.rs").read_text(
        encoding="utf-8"
    )
    assert "pub mod adapter_support" in facade_source
    assert "compile_runtime_intents" in facade_source
    assert "qpc_frequency_hz" in facade_source
    assert "build_host_fingerprint" in facade_source

    audit_source = (ROOT / "scripts" / "audit_dispatch_assembly.ps1").read_text(
        encoding="utf-8"
    )
    assert "target\\dist\\deps" in audit_source
    assert "sky_player*.s" in audit_source
    assert "sky_player_rs.s" not in audit_source
    assert "authoritative sky_player output" in audit_source
