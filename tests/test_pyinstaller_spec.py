from __future__ import annotations

from pathlib import Path

SPEC_PATH = Path(__file__).resolve().parents[1] / "Sky-Auto-Player.spec"


def test_pyinstaller_spec_uses_explicit_runtime_collection() -> None:
    spec = SPEC_PATH.read_text(encoding="utf-8")

    assert "collect_all" not in spec
    assert "base.tcss" in spec
    assert "sky_player_rs.sky_player_rs" in spec
    assert "native_module.origin" in spec
