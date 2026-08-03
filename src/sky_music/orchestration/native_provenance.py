"""Shared fingerprint for the native Rust dispatch contract."""

from __future__ import annotations

import hashlib
from pathlib import Path

from sky_music.orchestration.native_models import RUST_DISPATCH_SCHEMA_VERSION

NATIVE_PROVENANCE_PATHS = (
    Path("rust") / "Cargo.lock",
    Path("rust") / "crates" / "sky_dispatch_core",
    Path("rust") / "crates" / "sky_dispatch_win32",
    Path("rust") / "crates" / "sky_player_rs",
)


def native_source_fingerprint(repo_root: Path, native_abi: str) -> str:
    """Hash only files that can change the native dispatch contract."""
    digest = hashlib.sha256()
    digest.update(f"schema:{RUST_DISPATCH_SCHEMA_VERSION}\n".encode())
    digest.update(f"abi:{native_abi}\n".encode())

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
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(65_536), b""):
                digest.update(chunk)
        digest.update(b"\0")
    return digest.hexdigest()
