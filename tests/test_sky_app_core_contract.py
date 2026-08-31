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
    assert "sky_native_adapters" not in manifest.get("dependencies", {})
    source = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "rust" / "crates" / "sky_app_core" / "src").rglob("*.rs"))
    )
    assert not any(marker in source for marker in FORBIDDEN)
    assert "#![forbid(unsafe_code)]" in source
    assert "ApplicationCore" not in source
    assert "PlaybackRequest" not in source
    assert "UpdateGateway" not in source
    assert "SongSource" in source
    assert "SettingsStore" in source
    assert "FuzzyRanker" in source
    assert "EventSink" not in source
    assert "std::fs" not in source
    assert "std::net" not in source
    assert sorted(path.name for path in (ROOT / "rust" / "crates" / "sky_app_core" / "src").glob("*.rs")) == [
        "catalog.rs",
        "lib.rs",
        "settings.rs",
        "song.rs",
        "timing.rs",
        "update.rs",
    ]


def test_native_services_are_deferred_until_a_real_native_owner_exists() -> None:
    shell = ROOT / "desktop" / "src-tauri" / "src"
    assert not (shell / "native_services.rs").exists()
    assert not (shell / "event_mux.rs").exists()
    source = (shell / "lib.rs").read_text(encoding="utf-8")
    assert "NativeServices" not in source
    assert "current_dir()" not in source
