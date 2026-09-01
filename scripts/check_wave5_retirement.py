"""Enforce the Wave 5 product/runtime retirement boundary.

This is a small, read-only guard for active manifests, product source, active
workflows, and canonical build scripts. Historical documentation, updater
migration fixtures, and this guard's own retired-token inventory are
intentionally outside the active product scan.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN_TOKENS = (
    "pyo3",
    "maturin",
    "pyinstaller",
    "sky_player_rs",
    "textual",
    "desktop_ipc",
    "sky-auto-player-core",
    "python.exe",
    "build_rust_wheel.py",
    "rapidfuzz",
    "soundcard",
)

ACTIVE_FILES = (
    Path("pyproject.toml"),
    Path("rust/Cargo.toml"),
    Path("rust/Cargo.lock"),
    Path("desktop/src-tauri/Cargo.toml"),
    Path("scripts/build_portable_release.py"),
    Path("scripts/release_common.py"),
    Path("scripts/verify_release_manifest.py"),
    Path(".github/workflows/ci.yml"),
    Path(".github/workflows/release.yml"),
)
ACTIVE_DIRS = (Path("desktop/src-tauri/src"),)
EXCLUDED_FILENAMES = {"check_wave5_retirement.py"}
LEDGER_PATH = ROOT / "docs" / "migration" / "wave5-python-retirement-ledger.json"
BASELINE = "1634729acbdc236e0e0964a3fc9f74283a68c1c6"
CLASSIFICATIONS = {
    "MIGRATED",
    "OBSOLETE",
    "TRANSPORT_ONLY",
    "DUPLICATE",
    "FIXTURE_FROZEN",
    "TOOLING_RETAINED",
}
EVIDENCE_REQUIRED = {"MIGRATED", "DUPLICATE", "FIXTURE_FROZEN"}
EVIDENCE_PLACEHOLDERS = (
    "named native/frontend/updater tests cover",
    "direct rust/native build evidence is stronger",
    "native rust/tauri services now own",
    "direct native qualification or the native release builder supersedes",
)


def _files() -> list[Path]:
    paths = [ROOT / relative for relative in ACTIVE_FILES]
    for relative in ACTIVE_DIRS:
        directory = ROOT / relative
        if directory.is_dir():
            paths.extend(path for path in directory.rglob("*") if path.is_file())
    rust_crates = ROOT / "rust" / "crates"
    if rust_crates.is_dir():
        paths.extend(
            path
            for crate in rust_crates.iterdir()
            if (crate / "src").is_dir()
            for path in (crate / "src").rglob("*")
            if path.is_file()
        )
    return sorted(set(paths))


def _active_hits() -> list[str]:
    hits: list[str] = []
    for path in _files():
        if path.name in EXCLUDED_FILENAMES:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for line_number, line in enumerate(text.splitlines(), 1):
            if _find_forbidden_tokens(line):
                hits.append(f"{path.relative_to(ROOT).as_posix()}:{line_number}: {line.strip()}")
    return hits


def _find_forbidden_tokens(text: str) -> list[str]:
    """Return retired product tokens found in active source text."""
    pattern = re.compile("|".join(re.escape(token) for token in FORBIDDEN_TOKENS), re.IGNORECASE)
    return [match.casefold() for match in pattern.findall(text)]


def _missing_paths() -> list[str]:
    forbidden = (
        Path("Sky-Auto-Player.spec"),
        Path("Sky-Auto-Player-Core.spec"),
        Path("src"),
        Path("rust/pyproject.toml"),
        Path("rust/crates/sky_player_rs"),
        Path("scripts/build_rust_wheel.py"),
        Path("scripts/build_pyinstaller_bootloader.ps1"),
    )
    return [str(path) for path in forbidden if (ROOT / path).exists()]


def _evidence_errors(entry: dict[str, object]) -> list[str]:
    """Validate concrete invariant-transfer evidence for one ledger entry."""
    classification = entry.get("classification")
    if classification not in EVIDENCE_REQUIRED:
        return []
    path = entry.get("path", "<unknown>")
    errors: list[str] = []
    invariants = entry.get("invariants")
    evidence = entry.get("evidence")
    if not isinstance(invariants, list) or not invariants or not all(
        isinstance(item, str) and item.strip() for item in invariants
    ):
        errors.append(f"{path}: {classification} requires non-empty invariants")
    if not isinstance(evidence, list) or not evidence or not all(
        isinstance(item, str) and item.strip() for item in evidence
    ):
        errors.append(f"{path}: {classification} requires a concrete evidence list")
        return errors

    for item in evidence:
        folded = item.casefold()
        if any(placeholder in folded for placeholder in EVIDENCE_PLACEHOLDERS):
            errors.append(f"{path}: placeholder evidence is not accepted: {item}")
            continue
        reference, separator, symbol = item.partition("::")
        normalized = reference.replace("\\", "/")
        candidate = Path(normalized)
        if not separator or candidate.is_absolute() or ".." in candidate.parts:
            errors.append(f"{path}: evidence must be a repository path::symbol: {item}")
            continue
        target = ROOT / candidate
        if not target.is_file():
            errors.append(f"{path}: evidence target is missing: {reference}")
            continue
        if not symbol.strip():
            errors.append(f"{path}: evidence symbol is empty: {item}")
            continue
        try:
            source = target.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as exc:
            errors.append(f"{path}: cannot read evidence target {reference}: {exc}")
            continue
        if symbol not in source:
            errors.append(f"{path}: evidence symbol is missing from {reference}: {symbol}")
    return errors


def _ledger_errors() -> list[str]:
    """Ensure every baseline/new Python surface has exactly one ledger entry."""
    try:
        ledger = json.loads(LEDGER_PATH.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        return [f"cannot read retirement ledger: {exc}"]
    entries = ledger.get("entries")
    additions = ledger.get("wave5_additions")
    if not isinstance(entries, list) or not isinstance(additions, list):
        return ["retirement ledger must contain entries and wave5_additions lists"]

    errors: list[str] = []
    paths: list[str] = []
    for entry in entries:
        if not isinstance(entry, dict):
            errors.append("retirement ledger contains a non-object entry")
            continue
        path = entry.get("path")
        classification = entry.get("classification")
        if not isinstance(path, str) or not isinstance(classification, str):
            errors.append(f"invalid retirement ledger entry: {entry!r}")
            continue
        paths.append(path.replace("\\", "/"))
        if classification not in CLASSIFICATIONS:
            errors.append(f"unknown retirement classification for {path}: {classification}")
        errors.extend(_evidence_errors(entry))
    if len(paths) != len(set(paths)):
        errors.append("retirement ledger contains duplicate baseline paths")

    result = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", BASELINE, "--", "scripts", "src", "tests"],
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        errors.append("cannot enumerate baseline Python surfaces")
        baseline_paths: set[str] = set()
    else:
        baseline_paths = {
            line.replace("\\", "/")
            for line in result.stdout.splitlines()
            if line.lower().endswith(".py")
        }
    ledger_paths = set(paths)
    errors.extend(
        f"baseline Python path missing from ledger: {missing}"
        for missing in sorted(baseline_paths - ledger_paths)
    )
    errors.extend(
        f"non-baseline path in baseline ledger entries: {extra}"
        for extra in sorted(ledger_paths - baseline_paths)
    )

    addition_paths: list[str] = []
    for entry in additions:
        if not isinstance(entry, dict):
            errors.append("retirement ledger contains a non-object Wave 5 addition")
            continue
        path = entry.get("path")
        classification = entry.get("classification")
        if not isinstance(path, str) or classification != "TOOLING_RETAINED":
            errors.append(f"invalid Wave 5 tooling addition: {entry!r}")
            continue
        normalized = path.replace("\\", "/")
        addition_paths.append(normalized)
        if not (ROOT / normalized).is_file():
            errors.append(f"Wave 5 tooling addition is missing: {normalized}")
    if len(addition_paths) != len(set(addition_paths)):
        errors.append("retirement ledger contains duplicate Wave 5 additions")
    if set(addition_paths) & ledger_paths:
        errors.append("Wave 5 additions overlap baseline ledger entries")
    return errors


def main() -> int:
    failures: list[str] = []
    if _missing_paths():
        failures.extend(f"retired path still exists: {path}" for path in _missing_paths())
    failures.extend(f"active product retirement token: {hit}" for hit in _active_hits())
    failures.extend(f"retirement ledger: {error}" for error in _ledger_errors())

    cargo = (ROOT / "rust/Cargo.lock").read_text(encoding="utf-8")
    if re.search(r"(?m)^name = \"(?:pyo3|pyo3-build-config|pyo3-ffi|sky_player_rs)\"$", cargo):
        failures.append("Cargo.lock still contains a retired binding package")

    project = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
    if "[project.scripts]" in project or 'package = true' in project:
        failures.append("pyproject still exposes an installable product package")

    if failures:
        print("Wave 5 retirement guard: FAIL", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        return 1
    print("Wave 5 retirement guard: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
