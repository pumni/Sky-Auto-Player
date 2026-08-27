"""Enforce the repository's small, model-native agent context boundary.

This check protects the shape of the control plane, not model behavior. Durable constraints belong
in compact root guidance, security policy, executable checks, and focused current documentation.
Historical implementation choreography belongs in Git history, not the active context path.
"""
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CONTEXT_BUDGETS: dict[str, int] = {
    "AGENTS.md": 10_000,
    "CLAUDE.md": 2_500,
    "CONTRIBUTING.md": 10_000,
    "docs/INDEX.md": 8_000,
    "docs/README.md": 5_000,
    ".github/PULL_REQUEST_TEMPLATE.md": 5_000,
}

RETIRED_PATHS: tuple[str, ...] = (
    ".claude",
    "docs/plan",
    "docs/rust-dispatch-migration",
    "docs/PORTING_GUIDE.md",
    "docs/2026-08-01-rust-overhaul-plan.md",
    "docs/2026-08-rust-core-consolidation-plan.md",
    "docs/dispatch-chord-timing-residual-review-2026-07-23.md",
)

CONTROL_SURFACES: tuple[str, ...] = tuple(CONTEXT_BUDGETS)
FORBIDDEN_CHOREOGRAPHY: tuple[str, ...] = (
    "priority stack",
    "altitude table",
    "coding_agent_handoff",
    "coding agent handoff",
    "<security_mandates>",
    "porting_guide.md",
    "read every plan",
    "preload all",
)


def _read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


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

    docs = ROOT / "docs"
    if docs.is_dir():
        failures.extend(
            f"top-level implementation plan must live in Git history: {path.relative_to(ROOT)}"
            for path in docs.glob("*plan*.md")
        )
        failures.extend(
            f"active agent handoff document is forbidden: {path.relative_to(ROOT)}"
            for path in docs.rglob("*HANDOFF*.md")
            if "archive" not in path.parts
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

    security_audit = ROOT / "scripts" / "audit_security_mandates.py"
    if security_audit.is_file() and "AGENTS.md P0" in security_audit.read_text(encoding="utf-8"):
        failures.append("security audit must be owned by SECURITY.md, not AGENTS.md P0 wording")

    if failures:
        print("Agent-context audit failed:")
        for failure in failures:
            print(f"  - {failure}")
        return 1

    print("[OK] Agent context is compact, routed, and free of retired choreography.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
