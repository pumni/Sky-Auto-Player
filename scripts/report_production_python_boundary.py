"""Report Python-related production boundary references during the Rust migration.

The report intentionally scans only production/build surfaces. Tests, docs, generated
artifacts, and dependency trees are excluded so the output can be used as migration
evidence instead of a repository-wide text search.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Final

ROOT = Path(__file__).resolve().parents[1]

MARKERS: Final[tuple[str, ...]] = (
    "pyo3",
    "maturin",
    "PyInstaller",
    "Sky-Auto-Player-Core",
    "--desktop-worker",
    "desktop_ipc",
    "python-environment",
)
SCAN_DIRECTORIES: Final[tuple[str, ...]] = (
    ".github/actions",
    ".github/workflows",
    "desktop/src-tauri",
    "rust",
    "src",
)
SCAN_FILES: Final[tuple[str, ...]] = (
    ".env.example",
    ".python-version",
    "Sky-Auto-Player-Core.spec",
    "Sky-Auto-Player.spec",
    "pyproject.toml",
    "scripts/build_portable_release.py",
    "scripts/build_pyinstaller_bootloader.ps1",
    "scripts/build_rust_wheel.py",
    "scripts/test_windows_updater_e2e.ps1",
    "scripts/verify_release_manifest.py",
)
TEXT_SUFFIXES: Final[frozenset[str]] = frozenset(
    {".json", ".mjs", ".ps1", ".py", ".rs", ".spec", ".toml", ".ts", ".tsx", ".yml", ".yaml"}
)
EXCLUDED_PARTS: Final[frozenset[str]] = frozenset(
    {
        ".git",
        ".pytest_tmp",
        ".venv",
        "__pycache__",
        "build",
        "dist",
        "generated",
        "gen",
        "node_modules",
        "target",
        "tests",
    }
)
REPORTER_PATH = "scripts/report_production_python_boundary.py"
COMMAND_OWNERSHIP_PATH = "desktop/src-tauri/src/command_ownership.rs"
COMMAND_OWNER_PATTERN = re.compile(
    r'\("(?P<method>[a-z_]+(?:\.[a-z_]+)+)",\s*CommandOwner::(?P<owner>Python|Native)\)'
)


@dataclass(frozen=True, slots=True)
class Reference:
    marker: str
    path: str
    line: int
    text: str


def _candidate_files(repository_root: Path) -> tuple[Path, ...]:
    candidates: set[Path] = set()
    for relative in SCAN_FILES:
        path = repository_root / relative
        if path.is_file():
            candidates.add(path)
    for relative in SCAN_DIRECTORIES:
        directory = repository_root / relative
        if not directory.is_dir():
            continue
        for path in directory.rglob("*"):
            relative_path = path.relative_to(repository_root)
            if (
                path.is_file()
                and path.suffix.lower() in TEXT_SUFFIXES
                and not EXCLUDED_PARTS.intersection(relative_path.parts)
                and not _is_test_file(path)
            ):
                candidates.add(path)
    return tuple(sorted(candidates))


def _is_test_file(path: Path) -> bool:
    name = path.name.casefold()
    return (
        name.startswith("test_")
        or name.endswith("_test.py")
        or name.endswith("_tests.rs")
        or "selftest" in name
    )


def collect(repository_root: Path = ROOT) -> tuple[Reference, ...]:
    """Collect sorted marker references from the production/build scan scope."""

    references: list[Reference] = []
    lowered_markers = tuple((marker, marker.casefold()) for marker in MARKERS)
    for path in _candidate_files(repository_root):
        relative = path.relative_to(repository_root).as_posix()
        if relative == REPORTER_PATH:
            continue
        for line_number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            folded_line = raw_line.casefold()
            for marker, folded_marker in lowered_markers:
                if folded_marker in folded_line:
                    references.append(Reference(marker, relative, line_number, raw_line.strip()))
    return tuple(references)


def _git_head(repository_root: Path) -> str | None:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=repository_root,
            capture_output=True,
            check=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return result.stdout.strip() or None


def _command_ownership(repository_root: Path) -> dict[str, object]:
    """Read the delivery matrix without importing the Tauri crate."""
    path = repository_root / COMMAND_OWNERSHIP_PATH
    if not path.is_file():
        return {
            "status": "unavailable",
            "source": COMMAND_OWNERSHIP_PATH,
            "before": {"commands": [], "native_count": 0, "python_count": 0},
            "after": {"commands": [], "native_count": 0, "python_count": 0},
        }
    current = [
        {"method": match.group("method"), "owner": match.group("owner")}
        for match in COMMAND_OWNER_PATTERN.finditer(path.read_text(encoding="utf-8"))
    ]
    before = [{"method": item["method"], "owner": "Python"} for item in current]

    def summary(commands: list[dict[str, str]]) -> dict[str, object]:
        return {
            "commands": commands,
            "native_count": sum(item["owner"] == "Native" for item in commands),
            "python_count": sum(item["owner"] == "Python" for item in commands),
        }

    return {
        "status": "ok",
        "source": COMMAND_OWNERSHIP_PATH,
        "before": summary(before),
        "after": summary(current),
    }


def _python_boundary_accounting(
    repository_root: Path,
    references: tuple[Reference, ...],
) -> dict[str, object]:
    ownership = _command_ownership(repository_root)
    python_modules = sorted(
        {
            item.path
            for item in references
            if item.path.startswith("src/") and item.path.endswith(".py")
        }
    )
    return {
        "command_ownership": ownership,
        "production_python_modules_still_required": python_modules,
        # These modules still serve the Textual/CLI surfaces or the eight
        # remaining Python-owned desktop routes. Do not label an entire module
        # non-authoritative until its last production consumer is gone.
        "python_modules_made_non_authoritative": [],
        "native_command_count": ownership["after"]["native_count"],
        "python_command_count": ownership["after"]["python_count"],
        "python_core_process_required": True,
        "python_runtime_shipped": True,
        "pyinstaller_required_for_portable_desktop": True,
        "pyo3_required_for_production_tauri_playback": False,
        "coresupervisor_use": "yes: settings.get, settings.patch, update.check, update.preferences.get, update.preferences.patch, update.begin_handoff, calibration.start, calibration.cancel",
        "desktop_ipc_use": "yes: the eight remaining Python-owned commands and their event stream",
        "remaining_blockers": {
            "settings.*": "Python Core still owns persisted settings routes because its process-local AppConfig cache remains live; native services read the atomically persisted shadow but do not write through a second authority.",
            "update.check": "Python still owns release metadata orchestration; preferences remain with Core until the settings/update family can move coherently through one authority.",
            "update.preferences.*": "Python Core owns the cached update preferences; moving only these routes to Native would create stale policy reads in update.check.",
            "update.begin_handoff": "Python still correlates the checked update to handoff; the native gateway has not yet taken over this transaction boundary.",
            "calibration.*": "Python still owns process isolation, evidence validation, cancellation, and cache publication.",
        },
    }


def _payload(repository_root: Path, references: tuple[Reference, ...]) -> dict[str, object]:
    grouped: dict[str, list[Reference]] = {marker: [] for marker in MARKERS}
    for reference in references:
        grouped[reference.marker].append(reference)
    return {
        "schema_version": 2,
        "repository_head": _git_head(repository_root),
        "scan_directories": list(SCAN_DIRECTORIES),
        "scan_files": list(SCAN_FILES),
        "markers": {
            marker: {
                "count": len(items),
                "files": sorted({item.path for item in items}),
                "references": [asdict(item) for item in items],
            }
            for marker, items in grouped.items()
        },
        "python_boundary": _python_boundary_accounting(repository_root, references),
    }


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    references = collect()
    payload = _payload(ROOT, references)
    if args.json:
        print(json.dumps(payload, indent=2, sort_keys=True))
        return 0

    print("Production Python boundary report")
    print(f"repository head: {payload['repository_head'] or 'unavailable'}")
    print("scope: .github/actions, .github/workflows, desktop/src-tauri, rust, src, and explicit packaging files")
    for marker in MARKERS:
        data = payload["markers"][marker]
        assert isinstance(data, dict)
        print(f"- {marker}: {data['count']} references in {len(data['files'])} files")
    accounting = payload["python_boundary"]
    assert isinstance(accounting, dict)
    ownership = accounting["command_ownership"]
    assert isinstance(ownership, dict)
    after = ownership["after"]
    assert isinstance(after, dict)
    print(
        "command ownership: "
        f"{after['native_count']} native / {after['python_count']} Python"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
