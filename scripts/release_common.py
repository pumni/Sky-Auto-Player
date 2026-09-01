"""Small repository-only release helpers.

This module is intentionally tooling, not an installable product package.  The
portable builder uses it for version/provenance, locked Cargo commands, source
fingerprinting, hashing, and MANIFEST generation.  Product code must not import
this module.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import tomllib
from datetime import UTC, datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_NAME = "Sky-Auto-Player"
RUST_DISPATCH_SCHEMA_VERSION = 4
RUST_TOOLCHAIN_FILE = ROOT / "rust" / "rust-toolchain.toml"
COMMIT_PATTERN = re.compile(r"[0-9a-f]{40}")

NATIVE_PROVENANCE_PATHS = (
    Path("rust") / "Cargo.lock",
    Path("rust") / "crates" / "sky_app_core",
    Path("rust") / "crates" / "sky_dispatch_core",
    Path("rust") / "crates" / "sky_dispatch_win32",
    Path("rust") / "crates" / "sky_native_adapters",
    Path("rust") / "crates" / "sky_player",
    Path("rust") / "crates" / "sky_updater",
    Path("desktop") / "src-tauri",
)


def get_project_version() -> str:
    """Read the native desktop package version without importing Python product code."""
    path = ROOT / "desktop" / "src-tauri" / "Cargo.toml"
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise RuntimeError(f"Cannot read native package version from {path}") from exc
    version = data.get("package", {}).get("version")
    if not isinstance(version, str) or not version.strip():
        raise RuntimeError(f"Missing native package version in {path}")
    return version.strip()


def get_pinned_rust_toolchain() -> str:
    try:
        data = tomllib.loads(RUST_TOOLCHAIN_FILE.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise RuntimeError(f"Cannot read Rust toolchain file: {RUST_TOOLCHAIN_FILE}") from exc
    channel = data.get("toolchain", {}).get("channel")
    if not isinstance(channel, str) or re.fullmatch(r"\d+\.\d+\.\d+", channel) is None:
        raise RuntimeError(f"Rust toolchain file must pin an exact x.y.z channel: {RUST_TOOLCHAIN_FILE}")
    return channel


def native_build_environment() -> dict[str, str]:
    env = os.environ.copy()
    env["RUSTUP_TOOLCHAIN"] = get_pinned_rust_toolchain()
    return env


def cargo_release_build_command(manifest: Path, binary: str) -> list[str]:
    return [
        "cargo", "build", "--manifest-path", str(manifest), "--bin", binary,
        "--profile", "dist", "--locked",
    ]


def get_git_head(*, require_clean: bool = True) -> str:
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "HEAD"],
        capture_output=True, text=True, cwd=str(ROOT), check=False,
    )
    head = result.stdout.strip().removesuffix(r"\n").strip()
    if result.returncode != 0 or not head:
        raise RuntimeError("Cannot determine git HEAD for native build provenance")
    dirty_result = subprocess.run(
        ["git", "status", "--porcelain"],
        capture_output=True, text=True, cwd=str(ROOT), check=False,
    )
    if dirty_result.returncode != 0:
        raise RuntimeError("Cannot determine git worktree state for native build provenance")
    if dirty_result.stdout.strip() and require_clean:
        raise RuntimeError("release build requires a clean Git worktree")
    return f"{head}-dirty" if dirty_result.stdout.strip() else head


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(65_536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def native_source_fingerprint(repo_root: Path) -> str:
    digest = hashlib.sha256()
    digest.update(f"schema:{RUST_DISPATCH_SCHEMA_VERSION}\n".encode())
    files: list[Path] = []
    for relative in NATIVE_PROVENANCE_PATHS:
        path = repo_root / relative
        if path.is_file():
            files.append(path)
        elif path.is_dir():
            files.extend(candidate for candidate in path.rglob("*") if candidate.is_file())
    for path in sorted(files, key=lambda item: item.relative_to(repo_root).as_posix()):
        relative = path.relative_to(repo_root).as_posix()
        digest.update(relative.encode())
        digest.update(b"\0")
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(65_536), b""):
                digest.update(chunk)
        digest.update(b"\0")
    return digest.hexdigest()


def validate_native_build_provenance(repo_head: str, native_build_commit: str) -> None:
    """Reject a release whose native metadata does not identify this exact HEAD."""
    if COMMIT_PATTERN.fullmatch(repo_head) is None:
        raise RuntimeError(f"repository HEAD is not a full commit SHA: {repo_head!r}")
    if COMMIT_PATTERN.fullmatch(native_build_commit) is None:
        raise RuntimeError(
            f"native build metadata is not a full commit SHA: {native_build_commit!r}"
        )
    if native_build_commit != repo_head:
        raise RuntimeError(
            "native build provenance does not match repository HEAD: "
            f"native={native_build_commit}, repo={repo_head}"
        )


def write_release_manifest(
    release_dir: Path,
    version: str,
    exe_name: str,
    git_head: str,
    *,
    dirty_worktree: bool = False,
    native_build_commit: str | None = None,
) -> None:
    smoke_log = release_dir / "_smoke_test.log"
    try:
        smoke_log.unlink(missing_ok=True)
    except OSError as exc:
        raise RuntimeError(f"Failed to remove temporary smoke-test log: {smoke_log}") from exc
    executable = release_dir / exe_name
    if not executable.is_file():
        raise RuntimeError(f"Release executable is missing: {executable}")
    files: list[dict[str, str | int]] = []
    for path in sorted(release_dir.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(release_dir).as_posix()
        if relative == "MANIFEST.json":
            continue
        try:
            files.append({"path": relative, "size": path.stat().st_size, "sha256": hash_file(path)})
        except (OSError, ValueError) as exc:
            raise RuntimeError(f"Failed to hash release asset: {path}") from exc
    manifest = {
        "schema_version": 2,
        "app": APP_NAME,
        "version": version,
        "executable": exe_name,
        "git_head": git_head,
        "dirty_worktree": dirty_worktree,
        "native_build_commit": git_head if native_build_commit is None else native_build_commit,
        "build_time_utc": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "files": files,
    }
    (release_dir / "MANIFEST.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
