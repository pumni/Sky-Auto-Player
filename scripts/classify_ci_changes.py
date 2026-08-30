"""Classify changed paths into the repository's CI validation layers.

The classifier is intentionally small and deterministic.  It keeps ordinary
documentation/site changes out of the expensive portable-package job while
forcing that job for every input that can change a shipped executable or its
qualification workflow.
"""

from __future__ import annotations

import argparse
import sys
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import PurePosixPath


@dataclass(frozen=True, slots=True)
class ChangeClasses:
    static_required: bool
    code_required: bool
    package_required: bool
    reason: str


_PACKAGE_FILES = frozenset(
    {
        ".python-version",
        ".env.example",
        "pyproject.toml",
        "uv.lock",
        "windows_version_info.txt",
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust/rust-toolchain.toml",
        ".cargo/config.toml",
        "desktop/src-tauri/Cargo.toml",
        "desktop/src-tauri/build.rs",
        "desktop/bun.lock",
        "desktop/package.json",
        "Sky-Auto-Player.spec",
        "Sky-Auto-Player-Core.spec",
    }
)

_PACKAGE_WORKFLOW_FILES = frozenset(
    {
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        ".github/actions/python-environment/action.yml",
    }
)

_PACKAGE_PATH_PREFIXES = (
    "desktop/src-tauri/capabilities/",
    "desktop/src-tauri/icons/",
    "rust/crates/sky_updater/",
    "scripts/build_",
    "scripts/test_windows_updater_e2e.ps1",
    "scripts/verify_release_manifest.py",
)

_CODE_PREFIXES = (
    "src/",
    "desktop/",
    "rust/",
    "tests/",
    "scripts/",
    ".cargo/",
)


def _normalize(path: str) -> str:
    normalized = str(PurePosixPath(path.replace("\\", "/")))
    while normalized.startswith("./"):
        normalized = normalized[2:]
    return normalized


def _is_package_sensitive(path: str) -> bool:
    return (
        path in _PACKAGE_FILES
        or path in _PACKAGE_WORKFLOW_FILES
        or path in {"desktop/src-tauri/tauri.conf.json", "src/build_app.py"}
        or any(path.startswith(prefix) for prefix in _PACKAGE_PATH_PREFIXES)
    )


def classify(paths: Iterable[str], *, force_full: bool = False) -> ChangeClasses:
    normalized = tuple(path for path in (_normalize(item.strip()) for item in paths) if path)
    if force_full:
        return ChangeClasses(True, True, True, "full validation requested")
    if not normalized:
        return ChangeClasses(False, False, False, "no changed paths")

    package = tuple(path for path in normalized if _is_package_sensitive(path))
    code = tuple(
        path
        for path in normalized
        if path.startswith(_CODE_PREFIXES) or path in _PACKAGE_FILES
    )
    if package:
        reason = f"package-sensitive: {', '.join(package[:3])}"
    elif code:
        reason = f"code/windows: {', '.join(code[:3])}"
    else:
        reason = "static/site/docs only"
    return ChangeClasses(True, bool(code), bool(package), reason)


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--full",
        action="store_true",
        help="require every validation layer (main pushes and manual runs)",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    result = classify(sys.stdin, force_full=args.full)
    print(f"static_required={'true' if result.static_required else 'false'}")
    print(f"code_required={'true' if result.code_required else 'false'}")
    print(f"package_required={'true' if result.package_required else 'false'}")
    print(f"classification_reason={result.reason}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
