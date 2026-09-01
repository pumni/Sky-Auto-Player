from __future__ import annotations

import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).parents[1]


def test_native_package_is_the_version_source() -> None:
    package = tomllib.loads(
        (ROOT / "desktop" / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8")
    )
    version = package["package"]["version"]
    assert re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?", version)
    assert version == "3.5.0"


def test_repository_tooling_does_not_import_deleted_product_metadata() -> None:
    for relative in ("scripts/release_common.py", "scripts/build_portable_release.py"):
        source = (ROOT / relative).read_text(encoding="utf-8")
        assert "import sky_music" not in source
        assert "from sky_music" not in source
        assert "build_app" not in source


def test_website_version_uses_native_package_metadata() -> None:
    source = (ROOT / "site" / "scripts" / "sync-version.mjs").read_text(encoding="utf-8")
    assert "desktop/src-tauri/Cargo.toml" in source
    assert "pyproject.toml" not in source
