"""Reject migration-phase names from durable runtime and release surfaces."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# The separators around a phase token must not use ``\b``: ``_`` is a word
# character, so ``SKY_PHASE8_RESTART_SELFTEST`` would otherwise be missed.
PHASE_NAME = re.compile(
    r"(?<![A-Za-z0-9])phase[_-]?\d+(?![A-Za-z0-9])",
    re.IGNORECASE,
)
_DURABLE_ROOTS = (
    "src",
    "desktop",
    "rust",
    "scripts",
    ".github/workflows",
    ".github/actions",
)
_DURABLE_FILES = (
    "pyproject.toml",
    "uv.lock",
)
_HISTORICAL_PREFIXES = (
    "scripts/bench_phase",
    "docs/evidence/desktop-phase",
    "tests/test_phase",
)
_IGNORED_DIRS = frozenset(
    {".git", "node_modules", "target", "dist", ".pytest_cache", "__pycache__"}
)


def _is_historical(path: str) -> bool:
    return path.startswith(_HISTORICAL_PREFIXES)


def _contains_phase_name(value: str) -> bool:
    return PHASE_NAME.search(value) is not None


def _path_has_durable_phase_name(relative: str) -> bool:
    return not _is_historical(relative) and _contains_phase_name(relative)


def _text_violations(text: str) -> list[tuple[int, str]]:
    return [
        (line_number, line.strip())
        for line_number, line in enumerate(text.splitlines(), 1)
        if _contains_phase_name(line)
    ]


def _durable_files() -> list[Path]:
    files: list[Path] = [ROOT / relative for relative in _DURABLE_FILES]
    for relative_root in _DURABLE_ROOTS:
        root = ROOT / relative_root
        if root.is_dir():
            files.extend(
                path
                for path in root.rglob("*")
                if path.is_file()
                and not any(
                    part in _IGNORED_DIRS or part.endswith(".egg-info")
                    for part in path.relative_to(ROOT).parts
                )
            )
    return sorted(set(files))


def find_violations() -> list[tuple[str, int, str]]:
    violations: list[tuple[str, int, str]] = []
    audit_path = Path(__file__).resolve()
    for path in _durable_files():
        if path.resolve() == audit_path:
            continue
        relative = path.relative_to(ROOT).as_posix()
        if _is_historical(relative):
            continue
        if _path_has_durable_phase_name(relative):
            violations.append((relative, 0, relative))
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for line_number, line in _text_violations(text):
            violations.append((relative, line_number, line))
    return violations


def main() -> int:
    violations = find_violations()
    if violations:
        for path, line_number, line in violations:
            print(f"{path}:{line_number}: durable phase identifier: {line}", file=sys.stderr)
        return 1
    print("Durable runtime/release phase-name audit: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
