from __future__ import annotations

import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).parents[1]


def _release_common():
    scripts = str(ROOT / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    import release_common

    return release_common


def test_dist_profile_preserves_shipping_optimization() -> None:
    cargo = tomllib.loads((ROOT / "rust" / "Cargo.toml").read_text(encoding="utf-8"))
    assert cargo["profile"]["release"] == {
        "lto": False,
        "codegen-units": 16,
        "opt-level": 2,
        "panic": "unwind",
        "strip": "symbols",
        "overflow-checks": True,
    }
    assert cargo["profile"]["dist"] == {
        "inherits": "release",
        "debug": False,
        "debug-assertions": False,
        "lto": "thin",
        "codegen-units": 1,
        "opt-level": 3,
    }


def test_native_release_helpers_are_tooling_only() -> None:
    common = _release_common()
    command = common.cargo_release_build_command(Path("native/Cargo.toml"), "native")
    assert command[command.index("--profile") + 1] == "dist"
    assert "--release" not in command
    source = (ROOT / "scripts" / "build_portable_release.py").read_text(encoding="utf-8")
    assert "build_rust_wheel" not in source
    assert "maturin" not in source
    assert "PyInstaller" not in source
    assert "sky_player_rs" not in source
    assert "build_app" not in source
    assert "from sky_music" not in source

    desktop_package = json.loads((ROOT / "desktop" / "package.json").read_text(encoding="utf-8"))
    assert desktop_package["scripts"]["tauri:build"] == "tauri build --profile dist"
    tauri_package = tomllib.loads(
        (ROOT / "desktop" / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8")
    )
    assert "tauri/custom-protocol" in tauri_package["features"]["desktop-runtime"]


def test_workspace_has_no_python_player_bridge() -> None:
    workspace = (ROOT / "rust" / "Cargo.toml").read_text(encoding="utf-8")
    lock = (ROOT / "rust" / "Cargo.lock").read_text(encoding="utf-8")
    assert "sky_player_rs" not in workspace
    assert "sky_player_rs" not in lock
    assert 'name = "pyo3"' not in lock
    assert not (ROOT / "rust" / "crates" / "sky_player_rs").exists()


def test_shipping_binary_paths_use_dist_profile() -> None:
    source = (ROOT / "scripts" / "build_portable_release.py").read_text(encoding="utf-8")
    assert '"target" / "dist" / "sky_desktop_shell.exe"' in source
    assert '"target" / "dist" / "sky_updater_e2e.exe"' in source
    assert '"target" / "dist" / CALIBRATION_EXE' in source
    assert '"--profile",\n            "dist"' in source
