from __future__ import annotations

import json
import tomllib
from pathlib import Path

ROOT = Path(__file__).parents[1]


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
        "lto": "thin",
        "codegen-units": 1,
        "opt-level": 3,
    }


def test_production_native_build_commands_select_dist() -> None:
    import sys

    sys.path.insert(0, str(ROOT / "src"))
    import build_app

    command = build_app.cargo_release_build_command(Path("native/Cargo.toml"), "native")
    assert command[command.index("--profile") + 1] == "dist"
    assert "--release" not in command

    build_app_source = (ROOT / "src" / "build_app.py").read_text(encoding="utf-8")
    portable_source = (ROOT / "scripts" / "build_portable_release.py").read_text(encoding="utf-8")
    release_workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    assert '"scripts/build_rust_wheel.py", "--profile", "dist"' in portable_source
    assert "scripts/build_rust_wheel.py --profile dist" in release_workflow
    assert '[sys.executable, str(build_script), "--profile", "dist"]' in build_app_source
    assert '"--profile",\n                "dist"' in portable_source

    desktop_package = json.loads((ROOT / "desktop" / "package.json").read_text(encoding="utf-8"))
    assert desktop_package["scripts"]["tauri:build"] == "tauri build --profile dist"


def test_wheel_builder_defaults_are_fail_safe() -> None:
    source = (ROOT / "scripts" / "build_rust_wheel.py").read_text(encoding="utf-8")
    tests_source = (ROOT / "tests" / "test_build_rust_wheel.py").read_text(encoding="utf-8")
    assert 'resolve_cargo_profile(None, test_support=False) == "dist"' in tests_source
    assert 'PRODUCTION_DEFAULT_PROFILE = "dist"' in source
    assert 'TEST_SUPPORT_DEFAULT_PROFILE = "release"' in source
    assert "default=None" in source
    assert "resolve_cargo_profile(args.profile, test_support=args.test_support)" in source


def test_shipping_binary_paths_match_dist_profile() -> None:
    build_app_source = (ROOT / "src" / "build_app.py").read_text(encoding="utf-8")
    portable_source = (ROOT / "scripts" / "build_portable_release.py").read_text(encoding="utf-8")
    updater_e2e_source = (ROOT / "scripts" / "test_windows_updater_e2e.ps1").read_text(encoding="utf-8")
    assembly_source = (ROOT / "scripts" / "audit_dispatch_assembly.ps1").read_text(encoding="utf-8")

    assert '"target" / "dist" / NATIVE_CALIBRATION_BINARY' in build_app_source
    assert '"target" / "dist" / "sky_updater.exe"' in build_app_source
    assert '"target" / "dist" / "sky_desktop_shell.exe"' in portable_source
    assert '"target" / "dist" / "sky_updater_e2e.exe"' in portable_source
    assert 'rust\\target\\dist\\sky_updater.exe' in updater_e2e_source
    assert "rust\\target\\dist\\deps\\sky_player_rs.s" in assembly_source
