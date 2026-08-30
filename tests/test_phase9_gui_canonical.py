"""Regression checks for the Phase 9 product-surface cutover."""

from __future__ import annotations

import hashlib
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _read(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def _png_dimensions(path: Path) -> tuple[int, int]:
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    offset = 8
    saw_idat = False
    saw_iend = False
    compressed = bytearray()
    dimensions: tuple[int, int] | None = None
    while offset < len(data):
        assert offset + 12 <= len(data)
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        chunk_start = offset + 8
        chunk_end = chunk_start + length
        assert chunk_end + 4 <= len(data)
        chunk_type = data[offset + 4 : offset + 8]
        chunk_data = data[chunk_start:chunk_end]
        checksum = struct.unpack(">I", data[chunk_end : chunk_end + 4])[0]
        assert checksum == zlib.crc32(chunk_type + chunk_data) & 0xFFFFFFFF
        if chunk_type == b"IHDR":
            assert dimensions is None and length == 13
            width, height, bit_depth, color_type, compression, filter_method, interlace = (
                struct.unpack(">IIBBBBB", chunk_data)
            )
            assert width > 0 and height > 0
            assert bit_depth in {1, 2, 4, 8, 16}
            assert color_type in {0, 2, 3, 4, 6}
            assert compression == filter_method == 0
            assert interlace in {0, 1}
            dimensions = (width, height)
        elif chunk_type == b"IDAT":
            saw_idat = True
            compressed.extend(chunk_data)
        elif chunk_type == b"IEND":
            assert length == 0
            saw_iend = True
            assert chunk_end + 4 == len(data)
            break
        offset = chunk_end + 4

    assert dimensions is not None and saw_idat and saw_iend
    # Exercise the actual PNG zlib stream; the browser-side test below checks
    # that WebView/browser decoding and layout also succeed.
    assert zlib.decompress(bytes(compressed))
    return dimensions


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


def test_phase9_evidence_records_real_screenshot_hashes_and_dimensions() -> None:
    evidence = _read("docs/evidence/desktop-phase9/README.md")
    expected = {
        "library-real-tauri.png": (1214, 798),
        "minimum-real-tauri.png": (920, 620),
        "detail-real-tauri.png": (1214, 798),
        "settings-real-tauri.png": (1214, 798),
    }

    for name, dimensions in expected.items():
        image = ROOT / "docs/evidence/desktop-nonphysical" / name
        public = ROOT / "site/public/assets/images" / name
        assert image.is_file()
        assert public.is_file()
        assert _png_dimensions(image) == dimensions
        assert _png_dimensions(public) == dimensions
        digest = hashlib.sha256(image.read_bytes()).hexdigest()
        assert hashlib.sha256(public.read_bytes()).hexdigest() == digest
        assert f"`{digest}`" in evidence


def test_phase9_evidence_records_exact_capture_provenance() -> None:
    evidence = _read("docs/evidence/desktop-phase9/README.md")
    assert "no browser mock or fake bridge" in evidence
    assert "capture_repo_head" in evidence
    assert "capture_command" in evidence
    assert "capture_context" in evidence
    assert "Sky-Auto-Player.exe" in evidence
    assert "Windows" in evidence


def test_packaged_and_source_fallback_contracts_remain_documented() -> None:
    play_bat = _read("play.bat")
    architecture = _read("docs/architecture.md")
    source_faq = _read("site/src/content/faq/en/source.md")

    assert '"%PACKAGED_CORE%" --tui %*' in play_bat
    assert "packaged `Sky-Auto-Player.exe` Tauri/React application is the canonical" in architecture
    assert "Sky-Auto-Player-Core.exe --tui" in architecture
    assert "supported source Textual/CLI fallback" in source_faq
