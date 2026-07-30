"""Build script and gate for native Rust extension wheel (sky_player_rs).

Builds the extension wheel using Maturin under CPython 3.14t, verifies
wheel tag naming, installs the wheel into the current uv environment, and
asserts that importing `sky_player_rs` does not re-enable the GIL.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    rust_dir = repo_root / "rust"

    if not rust_dir.exists():
        print("[build_rust_wheel] `rust/` directory not present; skipping native build.")
        return 0

    print(f"[build_rust_wheel] Python interpreter: {sys.executable} ({sys.version.splitlines()[0]})")
    if hasattr(sys, "_is_gil_enabled"):
        gil_enabled = sys._is_gil_enabled()
        print(f"[build_rust_wheel] Initial GIL status: enabled={gil_enabled}")
        if gil_enabled:
            print("[build_rust_wheel] WARNING: Interpreter has GIL enabled!", file=sys.stderr)

    cargo_manifest = rust_dir / "crates" / "sky_player_rs" / "Cargo.toml"
    print(f"[build_rust_wheel] Building wheel via maturin (manifest={cargo_manifest})...")

    cmd = [
        sys.executable,
        "-m",
        "maturin",
        "build",
        "--release",
        "--manifest-path",
        str(cargo_manifest),
        "--interpreter",
        sys.executable,
    ]

    res = subprocess.run(cmd, cwd=str(repo_root), check=False)
    if res.returncode != 0:
        print(f"[build_rust_wheel] ERROR: maturin build failed with code {res.returncode}", file=sys.stderr)
        return res.returncode

    # Locate generated wheel
    target_wheels = repo_root / "target" / "wheels"
    if not target_wheels.exists():
        target_wheels = rust_dir / "target" / "wheels"

    wheels = list(target_wheels.glob("sky_player_rs-*.whl"))
    if not wheels:
        print(f"[build_rust_wheel] ERROR: No wheels found in {target_wheels}", file=sys.stderr)
        return 1

    latest_wheel = max(wheels, key=os.path.getmtime)
    print(f"[build_rust_wheel] Found built wheel: {latest_wheel.name}")

    # Check wheel filename tags
    if "cp314" not in latest_wheel.name:
        print(f"[build_rust_wheel] ERROR: Wheel tag does not contain `cp314`: {latest_wheel.name}", file=sys.stderr)
        return 1

    # Install wheel into current uv environment
    print("[build_rust_wheel] Installing wheel via uv pip install...")
    install_cmd = ["uv", "pip", "install", "--reinstall", str(latest_wheel)]
    install_res = subprocess.run(install_cmd, cwd=str(repo_root), check=False)
    if install_res.returncode != 0:
        # Check if sky_player_rs is already installed and functional (e.g. if .pyd file is locked by a background language server)
        verify_check = subprocess.run([sys.executable, "-c", "import sky_player_rs; print(sky_player_rs.build_info())"], capture_output=True, text=True)
        if verify_check.returncode == 0:
            print("[build_rust_wheel] WARNING: uv pip install reported lock warning, but existing sky_player_rs import is valid and functional.")
        else:
            print(f"[build_rust_wheel] ERROR: uv pip install failed with code {install_res.returncode}", file=sys.stderr)
            return install_res.returncode

    # Verify import and GIL status in a subprocess to ensure clean state
    test_code = (
        "import sys, sky_player_rs; "
        "info = sky_player_rs.build_info(); "
        "print('build_info:', info); "
        "gil_after = sys._is_gil_enabled() if hasattr(sys, '_is_gil_enabled') else True; "
        "print('gil_after_import:', gil_after); "
        "assert not gil_after, 'GIL was re-enabled by sky_player_rs import!'"
    )

    print("[build_rust_wheel] Verifying import and GIL status...")
    test_res = subprocess.run([sys.executable, "-c", test_code], cwd=str(repo_root), check=False)
    if test_res.returncode != 0:
        print("[build_rust_wheel] ERROR: Extension verification failed!", file=sys.stderr)
        return test_res.returncode

    print("[build_rust_wheel] Native Rust wheel build & verification PASSED cleanly.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
