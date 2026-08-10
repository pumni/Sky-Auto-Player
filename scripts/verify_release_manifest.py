"""Fail-closed package assertion for the canonical portable release tree."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

APP_NAME = "Sky-Auto-Player"
PRIMARY_EXE = f"{APP_NAME}.exe"
REQUIRED = {
    PRIMARY_EXE,
    "native_calibration.exe",
    "Sky-Auto-Player-Updater.exe",
    "MANIFEST.json",
}
FORBIDDEN = {
    "Sky-Player.exe",
    "updater.bat",
    "installer",
    "updater.ps1",
}


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify(release_dir: Path, version: str) -> None:
    if not release_dir.is_dir():
        raise RuntimeError(f"release directory is missing: {release_dir}")
    all_paths = list(release_dir.rglob("*"))
    if any(path.is_symlink() for path in all_paths):
        raise RuntimeError("release contains a symlink")
    actual = {
        path.relative_to(release_dir).as_posix()
        for path in all_paths
        if path.is_file()
    }
    missing = REQUIRED - actual
    if missing:
        raise RuntimeError(f"release is missing required files: {sorted(missing)}")
    forbidden = sorted(
        path
        for path in actual
        if path in FORBIDDEN
        or path.startswith("installer/")
        or path.endswith(".bat")
        or path.endswith(".ps1")
        or path.endswith(".Tests.ps1")
        or path.endswith("/TestResults.xml")
        or path.startswith(".pytest_cache/")
        or path.startswith("__pycache__/")
    )
    if forbidden:
        raise RuntimeError(f"release contains forbidden artifacts: {forbidden}")

    manifest_path = release_dir / "MANIFEST.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RuntimeError("MANIFEST.json is not valid UTF-8 JSON") from exc
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 2:
        raise RuntimeError("MANIFEST.json schema_version must be 2")
    if manifest.get("app") != APP_NAME or manifest.get("version") != version:
        raise RuntimeError("manifest app/version does not match the release")
    if manifest.get("executable") != PRIMARY_EXE or manifest.get("dirty_worktree") is not False:
        raise RuntimeError("manifest executable or clean-worktree policy is invalid")
    files = manifest.get("files")
    if not isinstance(files, list):
        raise RuntimeError("manifest files must be a list")
    expected: dict[str, tuple[int, str]] = {}
    for entry in files:
        if not isinstance(entry, dict):
            raise RuntimeError("manifest contains a non-object file entry")
        path = entry.get("path")
        size = entry.get("size")
        digest = entry.get("sha256")
        if (
            not isinstance(path, str)
            or not isinstance(size, int)
            or size < 0
            or not isinstance(digest, str)
            or len(digest) != 64
            or path in expected
            or path == "MANIFEST.json"
            or "\\" in path
            or path.startswith("/")
            or (len(path) >= 2 and path[1] == ":")
            or "/../" in f"/{path}/"
        ):
            raise RuntimeError(f"invalid or duplicate manifest entry: {entry!r}")
        expected[path] = (size, digest.lower())
    if set(expected) != actual - {"MANIFEST.json"}:
        raise RuntimeError("manifest file set does not match the release tree")
    for relative, (size, digest) in expected.items():
        path = release_dir / relative
        if path.stat().st_size != size or _sha256(path) != digest:
            raise RuntimeError(f"manifest hash/size mismatch: {relative}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--release-dir", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    verify(args.release_dir.resolve(), args.version)
    print(f"Release manifest verified: {args.release_dir.resolve()}")


if __name__ == "__main__":
    main()
