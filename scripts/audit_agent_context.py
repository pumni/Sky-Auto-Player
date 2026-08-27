"""Enforce the repository's small, model-native agent context boundary.

This check protects the shape of the control plane, not model behavior. Durable constraints belong
in compact root guidance, security policy, executable checks, and focused current documentation.
Historical implementation choreography belongs in Git history, not the active context path.
"""
from __future__ import annotations

import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CONTEXT_BUDGETS: dict[str, int] = {
    "AGENTS.md": 7_000,
    "CLAUDE.md": 1_500,
    "CONTRIBUTING.md": 6_000,
    "docs/INDEX.md": 5_000,
    "docs/README.md": 3_000,
    ".github/PULL_REQUEST_TEMPLATE.md": 3_000,
    ".github/ISSUE_TEMPLATE/bug_report.md": 3_000,
    ".github/ISSUE_TEMPLATE/feature_request.md": 2_000,
    ".github/ISSUE_TEMPLATE/config.yml": 1_500,
}

RETIRED_PATHS: tuple[str, ...] = (
    ".agent",
    ".claude",
    ".codex",
    ".cursor",
    ".windsurf",
    ".cursorrules",
    ".windsurfrules",
    "GEMINI.md",
    "COPILOT.md",
    "site/AGENTS.md",
    "site/CLAUDE.md",
    "docs/archive",
    "docs/plan",
    "docs/rust-dispatch-migration",
    "docs/PORTING_GUIDE.md",
    "docs/rust-migration-plan.md",
    "docs/2026-08-01-rust-overhaul-plan.md",
    "docs/2026-08-rust-core-consolidation-plan.md",
    "docs/dispatch-chord-timing-residual-review-2026-07-23.md",
    ".github/ISSUE_TEMPLATE/security_p0.md",
    ".github/copilot-instructions.md",
)

CONTROL_SURFACES: tuple[str, ...] = tuple(CONTEXT_BUDGETS)
SECURITY_OWNED_SURFACES: tuple[str, ...] = (
    "scripts/audit_security_mandates.py",
    ".config/security_audit_baseline.json",
    ".github/workflows/release.yml",
)
SCAN_ROOTS: tuple[str, ...] = (
    "src",
    "rust",
    "tests",
    "scripts",
    "docs",
    ".github",
    "site",
)
GENERATED_DIR_NAMES: frozenset[str] = frozenset(
    {
        ".git",
        ".venv",
        ".astro",
        ".cache",
        ".pytest_cache",
        ".ruff_cache",
        "__pycache__",
        "node_modules",
        "target",
        "dist",
        "build",
        "coverage",
    }
)
SHADOW_GUIDE_NAMES: frozenset[str] = frozenset(
    {
        "AGENTS.md",
        "CLAUDE.md",
        "GEMINI.md",
        "COPILOT.md",
    }
)
FORBIDDEN_CHOREOGRAPHY: tuple[str, ...] = (
    "priority stack",
    "altitude table",
    "coding_agent_handoff",
    "coding agent handoff",
    "agents.md p0",
    "<security_mandates>",
    "porting_guide.md",
    "read every plan",
    "preload all",
)
FORBIDDEN_DOC_NAME_MARKERS: tuple[str, ...] = (
    "handoff",
    "coding-agent",
    "ai-coding",
)


def _read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def _walk_tracked_surface(base: Path):
    """Walk source-like directories while pruning generated dependency/build trees."""
    for current_root, dirs, files in os.walk(base):
        dirs[:] = [name for name in dirs if name not in GENERATED_DIR_NAMES]
        current = Path(current_root)
        for name in files:
            yield current / name


def _is_plan_name(path: Path) -> bool:
    stem = path.stem.lower().replace("_", "-")
    return (
        stem == "plan"
        or stem.startswith("plan-")
        or stem.endswith("-plan")
        or "-plan-" in stem
    )


def main() -> int:
    failures: list[str] = []

    for path, limit in CONTEXT_BUDGETS.items():
        target = ROOT / path
        if not target.is_file():
            failures.append(f"missing context surface: {path}")
            continue
        size = len(target.read_bytes())
        if size > limit:
            failures.append(f"{path} is {size} bytes; budget is {limit}")

    failures.extend(
        f"retired context surface returned: {path}"
        for path in RETIRED_PATHS
        if (ROOT / path).exists()
    )

    for root_name in SCAN_ROOTS:
        base = ROOT / root_name
        if not base.is_dir():
            continue
        failures.extend(
            f"nested agent authority is forbidden: {path.relative_to(ROOT)}"
            for path in _walk_tracked_surface(base)
            if path.name in SHADOW_GUIDE_NAMES
        )

    docs = ROOT / "docs"
    if docs.is_dir():
        failures.extend(
            f"plan/archive directory must live in Git history: {path.relative_to(ROOT)}"
            for path in docs.rglob("*")
            if path.is_dir() and path.name.lower() in {"plan", "plans", "archive", "archives"}
        )

        for path in docs.rglob("*.md"):
            normalized_name = path.name.lower().replace("_", "-")
            if _is_plan_name(path):
                failures.append(
                    f"implementation plan must live in Git history: {path.relative_to(ROOT)}"
                )
            if any(marker in normalized_name for marker in FORBIDDEN_DOC_NAME_MARKERS):
                failures.append(
                    f"agent handoff/runbook document is forbidden: {path.relative_to(ROOT)}"
                )

        for path in docs.glob("*.md"):
            lowered = path.read_text(encoding="utf-8").lower()
            failures.extend(
                f"{path.relative_to(ROOT)} contains retired instruction choreography: {phrase!r}"
                for phrase in FORBIDDEN_CHOREOGRAPHY
                if phrase in lowered
            )

    for path in CONTROL_SURFACES:
        target = ROOT / path
        if not target.is_file():
            continue
        lowered = _read(path).lower()
        failures.extend(
            f"{path} contains retired instruction choreography: {phrase!r}"
            for phrase in FORBIDDEN_CHOREOGRAPHY
            if phrase in lowered
        )

    agents = ROOT / "AGENTS.md"
    if agents.is_file():
        text = _read("AGENTS.md")
        if "SECURITY.md" not in text:
            failures.append("AGENTS.md must route security authority to SECURITY.md")
        if "vendor-neutral repository contract" not in text:
            failures.append("AGENTS.md must identify itself as the vendor-neutral repository contract")

    claude = ROOT / "CLAUDE.md"
    if claude.is_file() and "AGENTS.md" not in _read("CLAUDE.md"):
        failures.append("CLAUDE.md must remain a thin adapter to AGENTS.md")

    failures.extend(
        f"{path} still derives security authority from AGENTS.md P0 wording"
        for path in SECURITY_OWNED_SURFACES
        if (ROOT / path).is_file() and "agents.md p0" in _read(path).lower()
    )

    if failures:
        print("Agent-context audit failed:")
        for failure in failures:
            print(f"  - {failure}")
        return 1

    print("[OK] Agent context is compact, routed, and free of retired choreography.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
