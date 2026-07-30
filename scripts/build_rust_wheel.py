"""Build script and gate for native Rust extension wheel (sky_player_rs).

Builds the extension wheel using Maturin under CPython 3.14t, verifies the
exact wheel tag, installs only that artifact into a clean uv environment, and
asserts that importing `sky_player_rs` does not re-enable the GIL.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

from packaging.tags import Tag
from packaging.utils import canonicalize_name, parse_wheel_filename

EXPECTED_NATIVE_TAG = Tag("cp314", "cp314t", "win_amd64")


def expected_build_commit(repo_root: Path) -> str:
    for name in ("SKY_NATIVE_BUILD_COMMIT", "GITHUB_SHA"):
        value = os.environ.get(name, "").strip()
        if value:
            return value
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "HEAD"],
        cwd=str(repo_root),
        capture_output=True,
        text=True,
        check=False,
    )
    commit = result.stdout.strip()
    if result.returncode != 0 or not commit:
        raise RuntimeError("cannot determine expected native build commit")
    return commit


def verify_wheel_name(wheel: Path) -> None:
    distribution, _version, _build, tags = parse_wheel_filename(wheel.name)
    if distribution != canonicalize_name("sky_player_rs"):
        raise RuntimeError(f"unexpected wheel distribution: {distribution}")
    if tags != {EXPECTED_NATIVE_TAG}:
        rendered = ", ".join(sorted(str(tag) for tag in tags))
        raise RuntimeError(
            "wheel must have exactly the CPython 3.14t Windows x64 tag "
            f"{EXPECTED_NATIVE_TAG}; found {rendered}"
        )


def clean_venv_python(venv: Path) -> Path:
    if os.name == "nt":
        return venv / "Scripts" / "python.exe"
    return venv / "bin" / "python"


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

    try:
        verify_wheel_name(latest_wheel)
    except (ValueError, RuntimeError) as exc:
        print(f"[build_rust_wheel] ERROR: invalid wheel ABI: {exc}", file=sys.stderr)
        return 1

    expected_commit = expected_build_commit(repo_root)
    print(f"[build_rust_wheel] Expected build commit: {expected_commit}")

    # Install only the wheel under test into a fresh environment. A failed
    # reinstall must never be satisfied by a previously installed extension.
    with tempfile.TemporaryDirectory(prefix="sky-rust-wheel-") as temp_dir:
        venv = Path(temp_dir) / "venv"
        create_res = subprocess.run(
            ["uv", "venv", "--python", sys.executable, str(venv)],
            cwd=str(repo_root),
            check=False,
        )
        if create_res.returncode != 0:
            print("[build_rust_wheel] ERROR: failed to create clean verification environment", file=sys.stderr)
            return create_res.returncode
        clean_python = clean_venv_python(venv)
        print("[build_rust_wheel] Installing exact wheel into clean environment...")
        install_res = subprocess.run(
            ["uv", "pip", "install", "--python", str(clean_python), "--no-deps", str(latest_wheel)],
            cwd=str(repo_root),
            check=False,
        )
        if install_res.returncode != 0:
            print(
                f"[build_rust_wheel] ERROR: exact wheel install failed with code {install_res.returncode}",
                file=sys.stderr,
            )
            return install_res.returncode

        test_code = (
            "import os, sys, sky_player_rs; "
            "info = sky_player_rs.build_info(); "
            "print('build_info:', info); "
            "assert info.get('native_abi') == 'cp314t-win_amd64', info; "
            "assert info.get('native_build_commit') == os.environ['SKY_EXPECTED_BUILD_COMMIT'], info; "
            "gil_after = sys._is_gil_enabled() if hasattr(sys, '_is_gil_enabled') else True; "
            "print('gil_after_import:', gil_after); "
            "assert not gil_after, 'GIL was re-enabled by sky_player_rs import!'"
        )
        clean_env = os.environ.copy()
        clean_env.pop("PYTHONPATH", None)
        clean_env.pop("VIRTUAL_ENV", None)
        clean_env["SKY_EXPECTED_BUILD_COMMIT"] = expected_commit
        print("[build_rust_wheel] Verifying exact wheel import, ABI, commit and GIL...")
        test_res = subprocess.run(
            [str(clean_python), "-c", test_code],
            cwd=str(repo_root),
            env=clean_env,
            check=False,
        )
        if test_res.returncode != 0:
            print("[build_rust_wheel] ERROR: clean extension verification failed!", file=sys.stderr)
            return test_res.returncode

    print("[build_rust_wheel] Native Rust wheel build & verification PASSED cleanly.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
