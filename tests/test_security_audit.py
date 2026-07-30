from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from types import ModuleType


def _load_audit_module() -> ModuleType:
    path = Path(__file__).parents[1] / "scripts" / "audit_security_mandates.py"
    spec = importlib.util.spec_from_file_location("audit_security_mandates", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


AUDIT = _load_audit_module()


def test_python_legacy_input_apis_are_forbidden(tmp_path: Path) -> None:
    source = tmp_path / "legacy_input.py"
    source.write_text(
        "user32.keybd_event(1, 2, 3, 4)\nuser32.mouse_event(1, 2, 3, 4, 5)\n",
        encoding="utf-8",
    )

    rules = {finding.rule for finding in AUDIT.scan_file(source)}

    assert rules == {
        "forbidden-call:keybd_event",
        "forbidden-call:mouse_event",
    }


def test_python_third_party_input_modules_are_forbidden(tmp_path: Path) -> None:
    source = tmp_path / "third_party_input.py"
    source.write_text("import keyboard\nfrom pynput import keyboard as keys\n", encoding="utf-8")

    rules = {finding.rule for finding in AUDIT.scan_file(source)}

    assert rules == {
        "forbidden-import:keyboard",
        "forbidden-import:pynput",
    }


def test_rust_scanner_allows_sendinput_and_ignores_comments(tmp_path: Path) -> None:
    source = tmp_path / "allowed.rs"
    source.write_text(
        "// keybd_event must remain forbidden\n"
        "/* SetWindowsHookExW is forbidden too. */\n"
        "unsafe { SendInput(1, inputs, size); }\n",
        encoding="utf-8",
    )

    assert AUDIT.scan_rust_file(source) == []


def test_rust_scanner_flags_forbidden_api_and_dll(tmp_path: Path) -> None:
    source = tmp_path / "forbidden.rs"
    source.write_text(
        'let dll = "ntdll.dll";\nunsafe { SetWindowsHookExW(0, hook, 0, 0); }\n',
        encoding="utf-8",
    )

    rules = {finding.rule for finding in AUDIT.scan_rust_file(source)}

    assert rules == {
        "forbidden-call:SetWindowsHookExW",
        "forbidden-dll-load",
    }


def test_rust_scanner_allows_approved_windows_sys_modules(tmp_path: Path) -> None:
    source = tmp_path / "approved_windows_sys.rs"
    source.write_text(
        "use windows_sys::Win32::Foundation::HANDLE;\n"
        "use windows_sys::Win32::System::Threading::{CloseHandle, WaitForSingleObject};\n"
        "use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;\n",
        encoding="utf-8",
    )

    assert AUDIT.scan_rust_file(source) == []


def test_rust_scanner_flags_unapproved_windows_sys_module(tmp_path: Path) -> None:
    source = tmp_path / "unapproved_windows_sys.rs"
    source.write_text(
        "use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;\n",
        encoding="utf-8",
    )

    rules = {finding.rule for finding in AUDIT.scan_rust_file(source)}

    assert rules == {
        "forbidden-call:ReadProcessMemory",
        "disallowed-windows-sys-module",
    }
