from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).parents[1]
METHOD_PATTERN = re.compile(
    r'"((?:app|catalog|settings|update|playback|diagnostics|calibration)'
    r'(?:\.[a-z_]+)+)"'
)


def _methods_used_by_tauri_commands() -> set[str]:
    source = "\n".join(
        path.read_text(encoding="utf-8")
        for path in (
            ROOT / "desktop" / "src-tauri" / "src" / "commands.rs",
            ROOT / "desktop" / "src-tauri" / "src" / "core" / "supervisor.rs",
        )
    )
    return set(METHOD_PATTERN.findall(source))


def _methods_in_ownership_matrix() -> set[str]:
    source = (
        ROOT / "desktop" / "src-tauri" / "src" / "command_ownership.rs"
    ).read_text(encoding="utf-8")
    return set(METHOD_PATTERN.findall(source))


def test_every_tauri_core_method_is_in_the_explicit_ownership_matrix() -> None:
    used = _methods_used_by_tauri_commands()
    owned = _methods_in_ownership_matrix()
    assert used == owned
    assert len(owned) == 21


def test_delivery_has_no_implicit_native_to_python_fallback() -> None:
    commands = (ROOT / "desktop" / "src-tauri" / "src" / "commands.rs").read_text(
        encoding="utf-8"
    )
    ownership = (
        ROOT / "desktop" / "src-tauri" / "src" / "command_ownership.rs"
    ).read_text(encoding="utf-8")
    assert "native_then_python" not in commands
    assert "implicit native" in ownership
