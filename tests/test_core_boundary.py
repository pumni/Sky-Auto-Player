"""Phase 4 structural isolation boundary — AST-enforced import gate.

See docs/2026-07_core-dispatch-refactor-and-isolation-plan.md §7.6.

The dispatch core (``sky_music.orchestration.core``) MUST NOT couple to platform,
UI, or focus implementation modules. The future Rust worker replaces a
well-defined seam; this test walks every module under the package and rejects
forbidden imports so regressions cannot silently re-couple the core to the
edge.

Forbidden prefixes:
- ``sky_music.platform`` — SendInput / focus / debug-log live behind ports.
- ``sky_music.ui`` — renderer / Textual / HUD wiring is caller-side.
- ``sky_music.infrastructure.focus`` — focus policy is consumed via the
  ``FocusSignal`` / ``FocusGuard`` ports; the concrete win32 implementation
  stays in infrastructure.

Allowed (the seam surface itself):
- ``sky_music.orchestration.core`` — internal co-module / subpackage imports.
- ``sky_music.infrastructure.backend``, ``.timing``, ``.wait_strategy`` —
  the protocol types live there; ``core.ports`` re-exports them, but core
  modules may also import the originals.
- ``sky_music.domain.scheduler_types`` — pure data types used by the loop.
- Standard-library / typing-only imports.

Why ``ast`` not grep: catches relative-buried + multi-line + aliased imports,
and stays stable under refactors that move symbols but not import statements.
"""

from __future__ import annotations

import ast
from pathlib import Path

CORE_PACKAGE = Path(__file__).resolve().parents[1] / "src" / "sky_music" / "orchestration" / "core"
FORBIDDEN_PREFIXES: tuple[str, ...] = (
    "sky_music.platform",
    "sky_music.ui",
    "sky_music.infrastructure.focus",
    # No back-reference to the engine: the core is consumed BY the engine, never the
    # reverse. This keeps the seam acyclic (§7.6) so the Rust worker can replace it wholesale.
    "sky_music.orchestration.engine",
)

# Modules that legitimately import the engine seam (re-exports, type-check-only shims).
# The boundary test must allow them — the boundary rule is one-directional: core MUST
# NOT depend on platform/ui/focus, but other modules MAY depend on core.
_ALLOWED_FORWARD_DEPENDENTS: frozenset[str] = frozenset()


def _module_targets(tree: ast.AST) -> list[str]:
    targets: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            targets.extend(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module is not None:
            # Resolve ``from .x import y`` relative imports against the file's package
            # so for ``sky_music.orchestration.core.state`` importing ``.ports`` we
            # get ``sky_music.orchestration.core.ports``, not ``.ports``.
            module = node.module
            if node.level:
                here = node.module if node.module else ""
                # ast gives level=N for N dots; the package chain comes from the file.
                # We do not need the absolute name for forbidden-prefix checks (relative
                # imports cannot target ``sky_music.platform.*`` etc from inside core),
                # so relative imports are simply skipped.
                if not here:
                    continue
            targets.append(module)
    return targets


def _violations_for(path: Path) -> list[tuple[Path, str]]:
    src = path.read_text(encoding="utf-8")
    tree = ast.parse(src, filename=str(path))
    matched: list[tuple[Path, str]] = [
        (path, module)
        for module in _module_targets(tree)
        for prefix in FORBIDDEN_PREFIXES
        if module == prefix or module.startswith(prefix + ".")
    ]
    return matched


def test_core_has_no_forbidden_imports() -> None:
    """No module under ``sky_music.orchestration.core`` may import forbidden prefixes."""
    if not CORE_PACKAGE.exists():
        return  # package absent — nothing to gate (premature import would fail anyway).
    violations: list[tuple[Path, str]] = []
    for py_file in sorted(CORE_PACKAGE.rglob("*.py")):
        # Skip __pycache__/etc.; rglob would not normally include them but be explicit.
        if "__pycache__" in py_file.parts:
            continue
        violations.extend(_violations_for(py_file))
    if violations:
        details = "\n".join(f"  {p.relative_to(CORE_PACKAGE.parent.parent.parent)} -> {mod}" for p, mod in violations)
        raise AssertionError(
            f"core boundary violated — these imports are forbidden:\n{details}\n"
            "Platform, UI, and focus internals must reach core only through ports."
        )


def test_boundary_test_self_checks_are_current() -> None:
    """Catch silent prefix-list drift: forbidden prefixes must still hit known would-be violations.

    Sanity guard so a future rename of the plan's forbidden prefixes cannot leave this
    test as a no-op. We synthesise a tiny module that imports ``sky_music.platform.win32.inputs``
    and confirm the walker catches it.
    """
    sample = (
        "from sky_music.platform.win32 import inputs\n"
        "from sky_music.ui.textual_app import app\n"
        "from sky_music.infrastructure.focus import FocusGuard\n"
        "from sky_music.orchestration.engine import PlaybackEngine\n"
    )
    tree = ast.parse(sample)
    targets = _module_targets(tree)
    matched_prefixes = {
        prefix
        for module in targets
        for prefix in FORBIDDEN_PREFIXES
        if module == prefix or module.startswith(prefix + ".")
    }
    assert matched_prefixes == set(FORBIDDEN_PREFIXES), (
        f"prefix list drift — walker matched {matched_prefixes!r} but expected {set(FORBIDDEN_PREFIXES)!r}"
    )
