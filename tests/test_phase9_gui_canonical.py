"""Regression checks for the Phase 9 product-surface cutover."""

from __future__ import annotations

import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _read(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def test_root_readme_declares_gui_as_canonical_and_tui_as_fallback() -> None:
    readme = _read("README.md")

    assert "canonical Tauri desktop GUI" in readme
    assert "Sky-Auto-Player.exe" in readme
    assert "play.bat" in readme
    assert "Sky-Auto-Player-Core.exe --tui" in readme
    assert "Modern Tauri desktop GUI" in readme
    assert "Textual TUI fallback" in readme
    assert "site/public/assets/images/picker.webp" not in readme
    assert "Sky Auto Player TUI picker" not in readme


def test_public_site_uses_real_desktop_evidence_assets() -> None:
    product = _read("site/src/components/home/ProductView.astro")
    site_contract = _read("site/tests/e2e/site-contracts.spec.ts")

    assert "/assets/images/library-real-tauri.png" in product
    assert "/assets/images/minimum-real-tauri.png" in product
    assert "REAL TAURI WINDOW" in product
    assert "picker.webp" not in product
    for name in (
        "library-real-tauri.png",
        "minimum-real-tauri.png",
        "detail-real-tauri.png",
        "settings-real-tauri.png",
    ):
        asset = ROOT / "site/public/assets/images" / name
        assert asset.is_file() and asset.stat().st_size > 0
        assert f"/assets/images/{name}" in site_contract


def test_phase9_evidence_records_public_screenshot_hashes() -> None:
    evidence = _read("docs/evidence/desktop-phase9/README.md")
    expected = {
        "library-real-tauri.png": "64f8a7c13fb5717d66e898a6ed2ca3bab6b24a68c91e8898f5f7a19f3446a8b6",
        "minimum-real-tauri.png": "c5bbf89402cadb92b900fba468a0cd594bf8aadaaf60cf36d4a4ec837d596db6",
        "detail-real-tauri.png": "454346d2db5eb490f7d0a8630073bb72a9f38a45fd26e58ddbbfd70029956e21",
        "settings-real-tauri.png": "39525818205db19a2a2622d52fa2bcb58405860ffb75895b4c2ae5753ab54915",
    }

    for name, digest in expected.items():
        image = ROOT / "docs/evidence/desktop-nonphysical" / name
        actual = hashlib.sha256(image.read_bytes()).hexdigest()
        assert actual == digest
        assert digest in evidence


def test_packaged_and_source_fallback_contracts_remain_documented() -> None:
    play_bat = _read("play.bat")
    architecture = _read("docs/architecture.md")
    source_faq = _read("site/src/content/faq/en/source.md")

    assert '"%PACKAGED_CORE%" --tui %*' in play_bat
    assert "packaged `Sky-Auto-Player.exe` Tauri/React application is the canonical" in architecture
    assert "Sky-Auto-Player-Core.exe --tui" in architecture
    assert "supported source Textual/CLI fallback" in source_faq
