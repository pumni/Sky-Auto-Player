"""Build script and gate for native Rust extension wheel (sky_player_rs).

Builds the extension wheel using Maturin under CPython 3.14t, verifies the
exact wheel tag, installs only that artifact into both a clean uv environment
and the active test/build interpreter, and asserts that importing
`sky_player_rs` does not re-enable the GIL.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

from packaging.tags import Tag
from packaging.utils import canonicalize_name, parse_wheel_filename

EXPECTED_NATIVE_TAG = Tag("cp314", "cp314t", "win_amd64")
EXPECTED_NATIVE_ABI = "cp314t-win_amd64"


def git_head(repo_root: Path) -> str:
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "HEAD"],
        cwd=str(repo_root),
        capture_output=True,
        text=True,
        check=False,
    )
    commit = result.stdout.strip()
    if result.returncode != 0 or not commit:
        raise RuntimeError("cannot determine checkout HEAD")
    return commit


def git_status(repo_root: Path) -> str:
    result = subprocess.run(
        ["git", "status", "--porcelain=v1", "--untracked-files=all"],
        cwd=str(repo_root),
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError("cannot determine checkout cleanliness")
    return result.stdout.strip()


def expected_build_commit(repo_root: Path, *, allow_dirty: bool = False) -> str:
    head = git_head(repo_root)
    reported = os.environ.get("GITHUB_SHA", "").strip()
    dirty = bool(git_status(repo_root))

    if reported and reported != head:
        message = f"GITHUB_SHA {reported} does not match checkout HEAD {head}"
        if not allow_dirty:
            raise RuntimeError(message)
        print(f"[build_rust_wheel] WARNING: {message}", file=sys.stderr)
    if dirty and not allow_dirty:
        raise RuntimeError(
            "native release build requires a clean working tree; "
            "use --allow-dirty-development-build only for local development"
        )
    return f"{head}-dirty" if dirty else head


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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--allow-dirty-development-build",
        action="store_true",
        help="allow a local dirty build and mark its native commit metadata with -dirty",
    )
    parser.add_argument(
        "--test-support",
        action="store_true",
        help="build the non-production wheel with native benchmark test support",
    )
    args = parser.parse_args()
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
    try:
        expected_commit = expected_build_commit(
            repo_root,
            allow_dirty=args.allow_dirty_development_build,
        )
    except RuntimeError as exc:
        print(f"[build_rust_wheel] ERROR: {exc}", file=sys.stderr)
        return 1
    print(f"[build_rust_wheel] Expected build commit: {expected_commit}")
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
    if args.test_support:
        cmd.extend(["--features", "test-support"])

    build_env = os.environ.copy()
    build_env["GITHUB_SHA"] = expected_commit
    build_env["SKY_NATIVE_BUILD_COMMIT"] = expected_commit
    build_env["SKY_NATIVE_DIRTY_WORKTREE"] = str(expected_commit.endswith("-dirty")).lower()
    res = subprocess.run(cmd, cwd=str(repo_root), env=build_env, check=False)
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

    # The clean venv proves the artifact itself is valid, but pytest and
    # PyInstaller run from this interpreter. Reinstall the exact wheel there
    # so neither step can accidentally use a stale extension or no extension.
    print("[build_rust_wheel] Installing exact wheel into the active environment...")
    active_install = subprocess.run(
        [
            "uv",
            "pip",
            "install",
            "--python",
            sys.executable,
            "--reinstall",
            "--no-deps",
            str(latest_wheel),
        ],
        cwd=str(repo_root),
        check=False,
    )
    if active_install.returncode != 0:
        print(
            f"[build_rust_wheel] ERROR: active environment install failed with code {active_install.returncode}",
            file=sys.stderr,
        )
        return active_install.returncode

    active_env = os.environ.copy()
    active_env["SKY_EXPECTED_BUILD_COMMIT"] = expected_commit
    active_env.pop("PYTHONPATH", None)
    print("[build_rust_wheel] Verifying the active environment wheel...")
    active_test = subprocess.run(
        [
            sys.executable,
            "-c",
            "import os, sky_player_rs; info = sky_player_rs.build_info(); "
            "print('active_build_info:', info); "
            "assert info.get('native_abi') == 'cp314t-win_amd64', info; "
            "assert info.get('native_build_commit') == os.environ['SKY_EXPECTED_BUILD_COMMIT'], info",
        ],
        cwd=str(repo_root),
        env=active_env,
        check=False,
    )
    if active_test.returncode != 0:
        print("[build_rust_wheel] ERROR: active environment verification failed!", file=sys.stderr)
        return active_test.returncode

    print("[build_rust_wheel] Native Rust wheel build & verification PASSED cleanly.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
