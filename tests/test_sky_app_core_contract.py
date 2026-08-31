from __future__ import annotations

import tomllib
from pathlib import Path

ROOT = Path(__file__).parents[1]
FORBIDDEN = {"tauri", "pyo3", "windows-sys", "sky_desktop_shell", "sky_player"}


def test_sky_app_core_is_pure_and_does_not_depend_on_player_or_delivery() -> None:
    manifest = tomllib.loads(
        (ROOT / "rust" / "crates" / "sky_app_core" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
    )
    assert not FORBIDDEN.intersection(manifest.get("dependencies", {}))
    source = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "rust" / "crates" / "sky_app_core" / "src").rglob("*.rs"))
    )
    assert not any(marker in source for marker in FORBIDDEN)
    assert "#![forbid(unsafe_code)]" in source
    assert "ApplicationCore" not in source
    assert "PlaybackRequest" not in source
    assert "SongSource" not in source
    assert "UpdateGateway" not in source
    assert sorted(path.name for path in (ROOT / "rust" / "crates" / "sky_app_core" / "src").glob("*.rs")) == [
        "lib.rs"
    ]
    assert not (ROOT / "rust" / "crates" / "sky_app_core" / "tests" / "use_cases.rs").exists()
