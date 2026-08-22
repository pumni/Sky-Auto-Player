"""Audit Sky Auto Player's source tree for AGENTS.md P0 security-mandate violations.

Mirrors the static guard `scripts/audit_free_threaded_wheels.py` plays for the
build pre-check. Walks Python under ``src/`` with an ``ast`` pass and Rust
under ``rust/`` with a conservative lexical pass. It flags anything that,
even if dormant or in dead code, crosses an AGENTS.md security mandate:

    P0.1 NO GAME TAMPERING    — hooks, injection, process tampering
    P0.2 SENDINPUT ONLY       — only `user32.SendInput` family is legal
    P0.3 STRICT VALIDATION    — runtime import of foothold tooling is denied

Rust's ``windows-sys`` dependency is checked against an explicit module
allowlist as well.  This keeps a newly added Win32 surface from silently
expanding the SendInput-only boundary.

The audit is read-only. Findings print to stdout, and exit code is non-zero
when any unsuppressed violation is found, so it slots directly into CI.

Baseline allowlist
------------------
``.config/security_audit_baseline.json`` lists historical violations the
project has chosen to migrate incrementally (one entry per (path, line,
rule)). Each unsuppressed entry below the baseline fails CI; entries that
match the baseline are reported but not failed. Delete a baseline entry
once the underlying code is migrated — the audit will then enforce it.

Stdlib only — no third-party deps. Mirrors `audit_free_threaded_wheels.py`.
"""
from __future__ import annotations

import ast
import json
import re
from dataclasses import dataclass
from pathlib import Path

NAME: str = "Sky Auto Player"
PYTHON_SOURCE_ROOT: Path = Path("src")
RUST_SOURCE_ROOT: Path = Path("rust")
BASELINE_PATH: Path = Path(".config") / "security_audit_baseline.json"

FORBIDDEN_CALL_NAMES: frozenset[str] = frozenset(
    {
        "SetWindowsHookEx", "SetWindowsHookExA", "SetWindowsHookExW",
        "SetWinEventHook",
        "ReadProcessMemory", "WriteProcessMemory",
        "NtReadVirtualMemory", "NtWriteVirtualMemory",
        "VirtualAllocEx", "VirtualFreeEx", "VirtualProtectEx", "VirtualQueryEx",
        "CreateRemoteThread", "CreateRemoteThreadEx",
        "NtCreateThreadEx", "RtlCreateUserThread", "QueueUserAPC",
        "GetThreadContext", "SetThreadContext", "SuspendThread",
        "DebugActiveProcess", "DebugActiveProcessStop",
        "ContinueDebugEvent", "WaitForDebugEvent",
        "NtQueryInformationProcess",
        "keybd_event", "mouse_event",
    }
)
ALLOWED_CALL_NAMES: frozenset[str] = frozenset(
    {
        "SendInput", "SendInputA", "SendInputW",
    }
)
FORBIDDEN_IMPORT_NAMES: frozenset[str] = frozenset(
    {
        "keyboard", "mouse", "pynput", "pyautogui",
        "pymem", "pyinject", "memory_kernel",
        "win32api",  # imported under win32api.* — kept loose; detect per-name below
    }
)
FORBIDDEN_DLL_NAMES: frozenset[str] = frozenset(
    {
        "ntdll.dll",
    }
)
ALLOWED_WINDOWS_SYS_MODULES: frozenset[str] = frozenset(
    {
        "Win32::Foundation",
        "Win32::Media",
        # Calibration runs in a dedicated subprocess and uses Raw Input only
        # to collect instrument-key evidence; it does not install hooks or
        # inspect another process.
        "Win32::UI::Input",
        "Win32::System::Performance",
        # The updater's application-owned progress window needs its real
        # module instance for WNDCLASSW/CreateWindowExW registration.
        "Win32::System::LibraryLoader",
        # The native acceptance artifact records the local Windows build as a
        # host fingerprint; it does not inspect game state or process memory.
        "Win32::System::SystemInformation",
        "Win32::System::Threading",
        "Win32::UI::Input::KeyboardAndMouse",
        # The updater progress window uses the standard progress-bar class;
        # this module does not provide input simulation or process access.
        "Win32::UI::Controls",
        "Win32::UI::WindowsAndMessaging",
        # The out-of-process updater uses WinHTTP for the explicit GitHub
        # HTTPS allow-list and MoveFileExW for atomic JSON replacement. These
        # modules are not input or process-tampering surfaces.
        "Win32::Networking::WinHttp",
        "Win32::Storage::FileSystem",
    }
)


@dataclass(frozen=True, slots=True)
class Finding:
    path: Path
    line: int
    rule: str
    detail: str


def _callee_name(node: ast.Call) -> tuple[str | None, str | None]:
    func = node.func
    if isinstance(func, ast.Attribute):
        return None, func.attr
    if isinstance(func, ast.Name):
        return func.id, None
    return None, None


def _scan_calls(tree: ast.AST) -> list[tuple[int, str, str]]:
    findings: list[tuple[int, str, str]] = []

    class Visitor(ast.NodeVisitor):
        def visit_Call(self, node: ast.Call) -> None:
            _, attr = _callee_name(node)
            direct, _ = _callee_name(node)
            name = attr or direct or ""
            if name and name in ALLOWED_CALL_NAMES:
                self.generic_visit(node)
                return
            if name and name in FORBIDDEN_CALL_NAMES:
                findings.append(
                    (
                        node.lineno,
                        f"forbidden-call:{name}",
                        f"`{name}` violates AGENTS.md P0 (game tampering / hook / debug).",
                    )
                )
            self.generic_visit(node)

    Visitor().visit(tree)
    return findings


def _scan_imports(tree: ast.AST) -> list[tuple[int, str, str]]:
    findings: list[tuple[int, str, str]] = []

    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                top = alias.name.split(".")[0]
                if top in FORBIDDEN_IMPORT_NAMES:
                    findings.append(
                        (
                            node.lineno,
                            f"forbidden-import:{top}",
                            f"importing `{alias.name}` is a foothold tool; violates AGENTS.md P0.",
                        )
                    )
        elif isinstance(node, ast.ImportFrom):
            module = (node.module or "").split(".")[0]
            if module in FORBIDDEN_IMPORT_NAMES:
                findings.append(
                    (
                        node.lineno,
                        f"forbidden-import:{module}",
                        f"`from {node.module} import ...` is a foothold tool; violates AGENTS.md P0.",
                    )
                )
            for alias in node.names:
                full = f"{module}.{alias.name}".lstrip(".")
                if "." in full and full.split(".")[0] == "win32api":
                    findings.extend(
                        (
                            node.lineno,
                            f"forbidden-import:{full}",
                            f"`from win32api import {alias.name}` is a foothold entry.",
                        )
                        for forbidden in FORBIDDEN_CALL_NAMES
                        if alias.name == forbidden and forbidden not in ALLOWED_CALL_NAMES
                    )
    return findings


def _scan_dll_loads(tree: ast.AST) -> list[tuple[int, str, str]]:
    findings: list[tuple[int, str, str]] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        callee_direct, callee_attr = _callee_name(node)
        if callee_direct != "WinDLL" and callee_attr != "WinDLL":
            continue
        if not node.args:
            continue
        first = node.args[0]
        if isinstance(first, ast.Constant) and isinstance(first.value, str):
            target = first.value.lower()
            if any(target == d for d in FORBIDDEN_DLL_NAMES):
                findings.append(
                    (
                        node.lineno,
                        "forbidden-dll-load",
                        f"`ctypes.WinDLL({first.value!r})` is process-tampering adjacent.",
                    )
                )
    return findings


def scan_file(path: Path) -> list[Finding]:
    try:
        source = path.read_text(encoding="utf-8")
    except OSError as exc:
        print(f"  [error] cannot read {path}: {exc}")
        return []

    try:
        tree = ast.parse(source, filename=str(path))
    except SyntaxError as exc:
        print(f"  [error] {path}:{exc.lineno}: {exc.msg}")
        return []

    findings: list[Finding] = []
    for line, rule, detail in _scan_calls(tree):
        findings.append(Finding(path, line, rule, detail))
    for line, rule, detail in _scan_imports(tree):
        findings.append(Finding(path, line, rule, detail))
    for line, rule, detail in _scan_dll_loads(tree):
        findings.append(Finding(path, line, rule, detail))
    return findings


def load_baseline(path: Path) -> set[tuple[str, int, str]]:
    if not path.exists():
        return set()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"  [warn] baseline unreadable at {path}: {exc}")
        return set()
    allow = set()
    for entry in data.get("exceptions", []):
        try:
            allow.add(
                (
                    str(entry["path"]),
                    int(entry["line"]),
                    str(entry["rule"]),
                )
            )
        except (TypeError, KeyError, ValueError):
            continue
    return allow


def audit_source_tree(source_root: Path) -> list[Finding]:
    if not source_root.exists():
        return []
    findings: list[Finding] = []
    for path in sorted(source_root.rglob("*.py")):
        findings.extend(scan_file(path))
    return findings


def _strip_rust_comments(source: str) -> str:
    """Remove Rust comments while preserving line numbers for findings."""

    source = re.sub(
        r"/\*.*?\*/",
        lambda match: "\n" * match.group(0).count("\n"),
        source,
        flags=re.DOTALL,
    )
    return "\n".join(line.split("//", 1)[0] for line in source.splitlines())


def scan_rust_file(path: Path) -> list[Finding]:
    try:
        source = path.read_text(encoding="utf-8")
    except OSError as exc:
        print(f"  [error] cannot read {path}: {exc}")
        return []

    code = _strip_rust_comments(source)
    forbidden_pattern = re.compile(
        rf"\b({'|'.join(re.escape(name) for name in sorted(FORBIDDEN_CALL_NAMES))})\b"
    )
    dll_pattern = re.compile(
        rf"(?i)\b({'|'.join(re.escape(name) for name in sorted(FORBIDDEN_DLL_NAMES))})\b"
    )
    windows_sys_pattern = re.compile(
        r"\bwindows_sys\s*::\s*"
        r"([A-Za-z0-9_]+(?:\s*::\s*[A-Za-z0-9_]+)*)?"
    )

    findings: list[Finding] = []
    for line_number, line in enumerate(code.splitlines(), start=1):
        for match in forbidden_pattern.finditer(line):
            name = match.group(1)
            findings.append(
                Finding(
                    path,
                    line_number,
                    f"forbidden-call:{name}",
                    f"`{name}` violates AGENTS.md P0 (SendInput-only / no tampering).",
                )
            )
        if match := dll_pattern.search(line):
            findings.append(
                Finding(
                    path,
                    line_number,
                    "forbidden-dll-load",
                    f"Rust reference to `{match.group(1)}` is process-tampering adjacent.",
                )
            )
        for match in windows_sys_pattern.finditer(line):
            module_path = re.sub(r"\s*::\s*", "::", match.group(1) or "<root>")
            if not any(
                module_path == allowed
                or module_path.startswith(f"{allowed}::")
                for allowed in ALLOWED_WINDOWS_SYS_MODULES
            ):
                findings.append(
                    Finding(
                        path,
                        line_number,
                        "disallowed-windows-sys-module",
                        f"`windows_sys::{module_path}` is outside the approved Win32 module allowlist.",
                    )
                )
    return findings


def audit_rust_tree(source_root: Path) -> list[Finding]:
    if not source_root.exists():
        return []
    findings: list[Finding] = []
    for path in sorted(source_root.rglob("*.rs")):
        findings.extend(scan_rust_file(path))
    return findings


def main() -> int:
    print(f"=== {NAME}: AGENTS.md P0 security-mandate audit ===\n")
    print(f"Python source:   {PYTHON_SOURCE_ROOT.resolve()}")
    print(f"Rust source:     {RUST_SOURCE_ROOT.resolve()}")
    print(f"Baseline file:   {BASELINE_PATH.resolve() or '(missing)'}\n")

    findings = audit_source_tree(PYTHON_SOURCE_ROOT)
    findings.extend(audit_rust_tree(RUST_SOURCE_ROOT))
    baseline = load_baseline(BASELINE_PATH)

    if not findings:
        print("[OK] No forbidden Windows API references in src/ or rust/.")
        return 0

    print("Findings:")
    suppressed = 0
    fresh = 0
    for f in findings:
        rel = f.path.resolve().relative_to(Path.cwd().resolve())
        norm_path = str(rel).replace("\\", "/")
        key = (norm_path, f.line, f.rule)
        marker = " (baseline)" if key in baseline else ""
        print(f"  {norm_path}:{f.line}  {f.rule}{marker}")
        print(f"      {f.detail}")
        if marker:
            suppressed += 1
        else:
            fresh += 1

    print(
        f"\nSummary: {len(findings)} finding(s) "
        f"({suppressed} suppressed by baseline, {fresh} fresh)"
    )
    if fresh:
        print("\n[FAIL] Unsuppressed P0 violations. Move code off the forbidden API")
        print("       or add a justified entry to .config/security_audit_baseline.json.")
        return 1

    print("\n[OK] All findings are covered by the baseline allowlist.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
