"""Static checks for the Rust dispatch architecture.

This dependency-free checker is intentionally conservative. Existing debt is
explicit in the temporary allowlist; new violations fail ``--enforce``.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

FACADE_HARD_LIMIT = 250
REGULAR_SOFT_LIMIT = 700
REGULAR_HARD_LIMIT = 900
WORKER_FUNCTION_HARD_LIMIT = 350
CONTEXT_FIELD_HARD_LIMIT = 12
WORKER_SCHEDULE_CLONE_PATTERNS = (
    "schedule.clone()",
    "Clone::clone(&schedule",
    "Clone::clone(&config.schedule",
)
WORKER_SCHEDULE_CLONE_MESSAGE = (
    "production worker must move RuntimeSchedule into the coordinator; "
    "cloning the schedule is forbidden"
)
FACADES = {"engine.rs", "input.rs", "wait.rs", "lib.rs"}
DISPATCH_FUNCTION_HARD_LIMIT = 180
LEGACY_DISPATCH_PATHS = {
    "rust/crates/sky_player/src/engine/worker/downs.rs",
    "rust/crates/sky_player/src/engine/worker/down_outcome.rs",
    "rust/crates/sky_player/src/engine/worker/releases.rs",
    "rust/crates/sky_player_rs/src/engine/worker/downs.rs",
    "rust/crates/sky_player_rs/src/engine/worker/down_outcome.rs",
    "rust/crates/sky_player_rs/src/engine/worker/releases.rs",
}
CANONICAL_DISPATCH_FILES = {
    "authored.rs",
    "mod.rs",
    "observation.rs",
    "observer.rs",
    "recovery.rs",
    "timing.rs",
    "hold_forensics.rs",
    "observer_wake.rs",
}
ALLOWLIST_PATH = Path(".config/rust_architecture_allowlist.json")
PLAYER_ADAPTER_FORBIDDEN_DIRECT_DEPENDENCIES = {
    "sky_dispatch_core",
    "sky_dispatch_win32",
}
APP_CORE_FORBIDDEN_DEPENDENCIES = {
    "tauri",
    "pyo3",
    "windows-sys",
    "sky_desktop_shell",
    "sky_player",
}

ALLOWED_UNSAFE_MODULES = {
    "rust/crates/sky_dispatch_win32/src/calibration.rs",
    "rust/crates/sky_dispatch_win32/src/clock.rs",
    "rust/crates/sky_dispatch_win32/src/cpu.rs",
    "rust/crates/sky_dispatch_win32/src/event.rs",
    "rust/crates/sky_dispatch_win32/src/focus.rs",
    "rust/crates/sky_dispatch_win32/src/input.rs",
    "rust/crates/sky_dispatch_win32/src/input/physical.rs",
    "rust/crates/sky_dispatch_win32/src/input/raw.rs",
    "rust/crates/sky_dispatch_win32/src/mmcss.rs",
    "rust/crates/sky_dispatch_win32/src/power.rs",
    "rust/crates/sky_dispatch_win32/src/timer.rs",
    "rust/crates/sky_dispatch_win32/src/wait.rs",
    "rust/crates/sky_dispatch_win32/src/wait/timer.rs",
}


@dataclass(frozen=True)
class Violation:
    rule: str
    path: str
    message: str


@dataclass
class CheckReport:
    errors: list[Violation] = field(default_factory=list)
    warnings: list[Violation] = field(default_factory=list)
    infos: list[str] = field(default_factory=list)


def _load_allowlist(repository_root: Path) -> dict[tuple[str, str], dict[str, str]]:
    path = repository_root / ALLOWLIST_PATH
    if not path.exists():
        return {}
    payload = json.loads(path.read_text(encoding="utf-8"))
    result: dict[tuple[str, str], dict[str, str]] = {}
    for entry in payload.get("entries", []):
        required = {"path", "rule", "reason", "expires_phase"}
        missing = required - set(entry)
        if missing:
            raise ValueError(f"allowlist entry missing {sorted(missing)}: {entry!r}")
        key = (str(entry["path"]), str(entry["rule"]))
        entry_path = repository_root / key[0]
        if not entry_path.is_file():
            raise ValueError(f"allowlist path does not exist: {key[0]}")
        result[key] = {name: str(entry[name]) for name in required}
    return result


def _without_comments(lines: list[str]) -> list[str]:
    result: list[str] = []
    in_block_comment = False
    for line in lines:
        current = line
        if in_block_comment:
            end = current.find("*/")
            if end < 0:
                result.append("")
                continue
            current = current[end + 2 :]
            in_block_comment = False
        while "/*" in current:
            start = current.find("/*")
            end = current.find("*/", start + 2)
            if end < 0:
                current = current[:start]
                in_block_comment = True
                break
            current = current[:start] + current[end + 2 :]
        if "//" in current:
            current = current.split("//", 1)[0]
        result.append(current)
    return result


def _brace_end(lines: list[str], start: int) -> int | None:
    depth = 0
    opened = False
    for index in range(start, len(lines)):
        depth += lines[index].count("{") - lines[index].count("}")
        opened |= "{" in lines[index]
        if opened and depth <= 0:
            return index
    return None


def _context_violations(lines: list[str], path: str) -> list[Violation]:
    clean = _without_comments(lines)
    declaration = re.compile(
        r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_]\w*)[^\{]*\{"
    )
    field = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?[A-Za-z_]\w*\s*:")
    violations: list[Violation] = []
    for index, line in enumerate(clean):
        match = declaration.match(line)
        if not match or not re.search(r"(?:Context|Inputs|Config|Options|Shared)$", match.group(1)):
            continue
        end = _brace_end(clean, index)
        if end is None:
            continue
        fields = sum(1 for candidate in clean[index + 1 : end] if field.match(candidate))
        if fields > CONTEXT_FIELD_HARD_LIMIT:
            violations.append(
                Violation(
                    "context_fields",
                    path,
                    f"{match.group(1)} has {fields} fields (> {CONTEXT_FIELD_HARD_LIMIT})",
                )
            )
    return violations


def _function_line_violations(
    lines: list[str], path: str, hard_limit: int, rule: str, include: str, prefix: bool = False
) -> list[Violation]:
    if not (path.startswith(include) if prefix else path.endswith(include)):
        return []
    clean = _without_comments(lines)
    source = "".join(clean)
    offsets: list[int] = []
    offset = 0
    for line in clean:
        offsets.append(offset)
        offset += len(line)
    violations: list[Violation] = []
    for match in re.finditer(r"\bfn\s+([A-Za-z_]\w*)\s*\(", source):
        start_line = max((index for index, value in enumerate(offsets) if value <= match.start()), default=0)
        open_brace = source.find("{", match.end())
        if open_brace < 0:
            continue
        depth = 0
        end_offset = None
        for index in range(open_brace, len(source)):
            if source[index] == "{":
                depth += 1
            elif source[index] == "}":
                depth -= 1
                if depth == 0:
                    end_offset = index
                    break
        if end_offset is None:
            continue
        end_line = max((index for index, value in enumerate(offsets) if value <= end_offset), default=start_line)
        line_count = end_line - start_line + 1
        if line_count > hard_limit:
            violations.append(
                Violation(
                    rule,
                    path,
                    f"{match.group(1)} has {line_count} lines (> {hard_limit})",
                )
            )
    return violations


def _worker_function_violations(lines: list[str], path: str) -> list[Violation]:
    return [
        violation
        for include in (
            "rust/crates/sky_player/src/engine/worker/orchestration.rs",
            "rust/crates/sky_player_rs/src/engine/worker/orchestration.rs",
        )
        for violation in _function_line_violations(
            lines,
            path,
            WORKER_FUNCTION_HARD_LIMIT,
            "worker_function_lines",
            include,
        )
    ]


def _dispatch_function_violations(lines: list[str], path: str) -> list[Violation]:
    return _function_line_violations(
        lines,
        path,
        DISPATCH_FUNCTION_HARD_LIMIT,
        "dispatch_function_lines",
        "rust/crates/sky_player/src/engine/worker/dispatch/",
        prefix=True,
    ) + _function_line_violations(
        lines,
        path,
        DISPATCH_FUNCTION_HARD_LIMIT,
        "dispatch_function_lines",
        "rust/crates/sky_player_rs/src/engine/worker/dispatch/",
        prefix=True,
    )


def _worker_schedule_clone_violation(joined: str, path: str) -> Violation | None:
    worker_files = {
        "rust/crates/sky_player/src/engine/worker.rs",
        "rust/crates/sky_player_rs/src/engine/worker.rs",
    }
    worker_roots = (
        "rust/crates/sky_player/src/engine/worker/",
        "rust/crates/sky_player_rs/src/engine/worker/",
    )
    if path not in worker_files and not path.startswith(worker_roots):
        return None
    if any(pattern in joined for pattern in WORKER_SCHEDULE_CLONE_PATTERNS):
        return Violation("runtime_schedule_clone", path, WORKER_SCHEDULE_CLONE_MESSAGE)
    return None


def _top_level_glob_import(lines: list[str]) -> bool:
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("//") or stripped.startswith("#!"):
            continue
        if stripped.startswith("#["):
            continue
        return stripped == "use super::*;"
    return False


def _test_support_is_gated(lines: list[str], path: str) -> bool:
    if "/test_support/" not in f"/{path}/" and not path.endswith("/test_support.rs"):
        return True
    joined = "".join(lines)
    return (
        '#[cfg(any(test, feature = "test-support"))]' in joined
        or '#![cfg(any(test, feature = "test-support"))]' in joined
    )


def _module_declaration_is_gated(lines: list[str], module_name: str) -> bool:
    for index, line in enumerate(lines):
        if re.match(rf"^\s*mod\s+{re.escape(module_name)}\s*;", line):
            previous = "\n".join(lines[max(0, index - 3) : index])
            return 'cfg(any(test, feature = "test-support"))' in previous
    return True


def _line_is_test_support_gated(lines: list[str], index: int) -> bool:
    if any("#![cfg(any(test, feature = \"test-support\"))]" in line for line in lines[:index + 1]):
        return True
    previous = "\n".join(lines[max(0, index - 3) : index])
    return '#[cfg(any(test, feature = "test-support"))]' in previous


def _record(report: CheckReport, violation: Violation, allowlist: dict[tuple[str, str], dict[str, str]]) -> None:
    debt = allowlist.get((violation.path, violation.rule))
    if debt:
        report.warnings.append(
            Violation(
                violation.rule,
                violation.path,
                f"{violation.message}; temporary allowlist: {debt['reason']} (expires {debt['expires_phase']})",
            )
        )
    else:
        report.errors.append(violation)


def _player_adapter_violations(repository_root: Path) -> list[Violation]:
    """Keep the temporary Python adapter behind sky_player's typed facade."""

    manifest = repository_root / "rust/crates/sky_player_rs/Cargo.toml"
    if not manifest.exists():
        return []
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    dependencies = data.get("dependencies", {})
    return [
        Violation(
            "player_adapter_dependency",
            "rust/crates/sky_player_rs/Cargo.toml",
            f"sky_player_rs must not depend directly on {name}; use sky_player::adapter_support",
        )
        for name in sorted(PLAYER_ADAPTER_FORBIDDEN_DIRECT_DEPENDENCIES.intersection(dependencies))
    ]


def _app_core_violations(repository_root: Path) -> list[Violation]:
    """Reject delivery/platform/player dependencies in the pure app crate."""

    crate_root = repository_root / "rust/crates/sky_app_core"
    manifest = crate_root / "Cargo.toml"
    if not manifest.exists():
        return []

    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    dependencies = data.get("dependencies", {})
    violations = [
        Violation(
            "app_core_dependency",
            "rust/crates/sky_app_core/Cargo.toml",
            f"sky_app_core must not depend directly on {name}",
        )
        for name in sorted(APP_CORE_FORBIDDEN_DEPENDENCIES.intersection(dependencies))
    ]
    source_markers = re.compile(
        r"\b(?:tauri|pyo3|windows-sys|windows_sys|sky_desktop_shell|sky_player)\b"
    )
    for filepath in sorted((crate_root / "src").rglob("*.rs")):
        relative = filepath.relative_to(repository_root).as_posix()
        joined = "".join(_without_comments(filepath.read_text(encoding="utf-8").splitlines(keepends=True)))
        if source_markers.search(joined):
            violations.append(
                Violation(
                    "app_core_dependency",
                    relative,
                    "sky_app_core source references a forbidden delivery/platform/player dependency",
                )
            )
    return violations


def check_repository(repository_root: Path) -> CheckReport:
    report = CheckReport()
    allowlist = _load_allowlist(repository_root)
    workspace_root = repository_root / "rust" / "crates"
    if not workspace_root.exists():
        report.errors.append(Violation("workspace", "rust/crates", "workspace not found"))
        return report

    for violation in _player_adapter_violations(repository_root):
        _record(report, violation, allowlist)

    for violation in _app_core_violations(repository_root):
        _record(report, violation, allowlist)

    dispatch_dir = repository_root / "rust/crates/sky_player/src/engine/worker/dispatch"
    if not dispatch_dir.exists():
        dispatch_dir = repository_root / "rust/crates/sky_player_rs/src/engine/worker/dispatch"
    if dispatch_dir.exists():
        actual_dispatch_files = {
            path.name
            for path in dispatch_dir.glob("*.rs")
            if not path.name.endswith("_tests.rs")
        }
        for unexpected in sorted(actual_dispatch_files - CANONICAL_DISPATCH_FILES):
            report.errors.append(
                Violation(
                    "unexpected_dispatch_module",
                    f"{dispatch_dir.relative_to(repository_root).as_posix()}/{unexpected}",
                    "dispatch directory contains a non-canonical module",
                )
            )
        for missing in sorted(CANONICAL_DISPATCH_FILES - actual_dispatch_files):
            report.errors.append(
                Violation(
                    "missing_dispatch_module",
                    f"{dispatch_dir.relative_to(repository_root).as_posix()}/{missing}",
                    "dispatch directory is missing a canonical module",
                )
            )

    for crate in ("sky_dispatch_core", "sky_dispatch_win32", "sky_app_core", "sky_player", "sky_player_rs"):
        crate_path = workspace_root / crate / "src"
        if not crate_path.exists():
            continue
        for filepath in sorted(crate_path.rglob("*.rs")):
            relative = filepath.relative_to(repository_root).as_posix()
            lines = filepath.read_text(encoding="utf-8").splitlines(keepends=True)
            clean = _without_comments(lines)
            joined = "".join(clean)
            report.infos.append(f"{relative:80} | {len(lines):4} lines")

            if relative in LEGACY_DISPATCH_PATHS:
                _record(
                    report,
                    Violation(
                        "legacy_dispatch_path",
                        relative,
                        "legacy dispatch path must be removed; dispatch code now lives under worker/dispatch/",
                    ),
                    allowlist,
                )
                continue

            limit = FACADE_HARD_LIMIT if filepath.name in FACADES else REGULAR_HARD_LIMIT
            if len(lines) > limit:
                rule = "facade_lines" if filepath.name in FACADES else "regular_module_lines"
                _record(report, Violation(rule, relative, f"{len(lines)} lines (> {limit})"), allowlist)
            elif filepath.name not in FACADES and len(lines) > REGULAR_SOFT_LIMIT:
                report.warnings.append(Violation("regular_module_soft_lines", relative, f"{len(lines)} lines (> {REGULAR_SOFT_LIMIT})"))

            if re.search(r"\bunsafe\b", joined) and relative not in ALLOWED_UNSAFE_MODULES:
                _record(report, Violation("unsafe_boundary", relative, "unsafe code outside allowlist"), allowlist)
            if ("pyo3::" in joined or "use pyo3" in joined) and not (
                relative.startswith("rust/crates/sky_player_rs/src/python/")
                or relative == "rust/crates/sky_player_rs/src/python.rs"
                or relative == "rust/crates/sky_player_rs/src/lib.rs"
            ):
                _record(report, Violation("pyo3_boundary", relative, "PyO3 import outside Python boundary"), allowlist)
            if crate == "sky_dispatch_core" and ("sky_dispatch_win32::" in joined or "use sky_dispatch_win32" in joined):
                _record(report, Violation("dependency_direction", relative, "core imports sky_dispatch_win32"), allowlist)
            if crate == "sky_player_rs" and (
                "sky_dispatch_core::" in joined
                or "use sky_dispatch_core" in joined
                or "sky_dispatch_win32::" in joined
                or "use sky_dispatch_win32" in joined
            ):
                _record(
                    report,
                    Violation(
                        "player_adapter_dependency",
                        relative,
                        "sky_player_rs source must use sky_player::adapter_support for dispatch access",
                    ),
                    allowlist,
                )
            if crate in {"sky_dispatch_core", "sky_dispatch_win32"} and (
                "sky_player_rs::" in joined
                or "use sky_player_rs" in joined
                or "sky_player::" in joined
                or "use sky_player" in joined
            ):
                _record(report, Violation("dependency_direction", relative, "lower crate imports sky_player_rs"), allowlist)
            if _top_level_glob_import(lines) and not (
                relative.endswith("/tests.rs") or "/tests/" in relative
            ):
                _record(report, Violation("production_glob_import", relative, "top-level use super::* in production module"), allowlist)
            for line_index, line in enumerate(lines):
                if "Box<dyn Fn" in line and not _line_is_test_support_gated(lines, line_index):
                    _record(report, Violation("production_dynamic_emitter", relative, "dynamic emitter in production source"), allowlist)
            if not _test_support_is_gated(lines, relative):
                _record(report, Violation("test_support_cfg", relative, "test-support source is not cfg-gated"), allowlist)
            for violation in _context_violations(lines, relative):
                _record(report, violation, allowlist)
            for violation in _worker_function_violations(lines, relative):
                _record(report, violation, allowlist)
            for violation in _dispatch_function_violations(lines, relative):
                _record(report, violation, allowlist)
            schedule_clone_violation = _worker_schedule_clone_violation(joined, relative)
            if schedule_clone_violation is not None:
                _record(report, schedule_clone_violation, allowlist)

    engine = repository_root / "rust/crates/sky_player/src/engine.rs"
    if engine.exists():
        lines = engine.read_text(encoding="utf-8").splitlines(keepends=True)
        if not _module_declaration_is_gated(lines, "test_support"):
            _record(report, Violation("test_support_cfg", "rust/crates/sky_player/src/engine.rs", "test_support module is not cfg-gated"), allowlist)
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--enforce", action="store_true", help="fail on ERROR violations")
    args = parser.parse_args(argv)
    try:
        report = check_repository(Path(__file__).resolve().parents[1])
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"ERROR: architecture checker configuration failed: {error}")
        return 1
    print("--- Rust Architecture Check ---")
    print("\n".join(report.infos))
    print("\n--- INFO ---")
    print(f"- scanned {len(report.infos)} Rust source files")
    for title, values in (("WARNING", report.warnings), ("ERROR", report.errors)):
        print(f"\n--- {title} ---")
        if values:
            for item in values:
                print(f"- [{item.rule}] {item.path}: {item.message}")
        else:
            print("- none")
    return 1 if args.enforce and report.errors else 0


if __name__ == "__main__":
    sys.exit(main())
