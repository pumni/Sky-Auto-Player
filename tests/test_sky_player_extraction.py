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
