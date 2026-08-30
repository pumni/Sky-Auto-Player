"""Check the branding source contract and generated consumer exports."""

from __future__ import annotations

import struct
from math import hypot
from pathlib import Path
from xml.etree import ElementTree

ROOT = Path(__file__).resolve().parents[1]
SVG_NS = "http://www.w3.org/2000/svg"
SVG = f"{{{SVG_NS}}}"
NIGHT = "#07090D"
IVORY = "#F4EFE3"
GOLD = "#F7DDA2"
SKY = "#B8CCD6"

CANONICAL = ROOT / "branding" / "sky-auto-player-app-icon.svg"
SMALL = ROOT / "branding" / "sky-auto-player-app-icon-small.svg"
TINY = ROOT / "branding" / "sky-auto-player-app-icon-16.svg"


def _parse_svg(path: Path) -> ElementTree.Element:
    return ElementTree.parse(path).getroot()


def _elements(root: ElementTree.Element, name: str) -> list[ElementTree.Element]:
    return root.findall(f".//{SVG}{name}")


def _ico_sizes(data: bytes) -> set[int]:
    assert data[:4] == b"\x00\x00\x01\x00"
    count = struct.unpack_from("<H", data, 4)[0]
    return {
        256 if width == 0 else width
        for width, _height in (
            struct.unpack_from("<BB", data, 6 + index * 16) for index in range(count)
        )
    }


def _png_dimensions(data: bytes) -> tuple[int, int]:
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    return struct.unpack_from(">II", data, 16)


def _assert_flat(root: ElementTree.Element) -> None:
    assert root.attrib["viewBox"] == "0 0 128 128"
    assert not _elements(root, "filter")
    assert not _elements(root, "image")
    assert not _elements(root, "linearGradient")
    assert not _elements(root, "radialGradient")


def _assert_mark_skeleton(root: ElementTree.Element, dash_count: int) -> None:
    lines = _elements(root, "line")
    assert len(lines) == 2 + dash_count
    solid = [line for line in lines if line.attrib["id"].startswith("edge-")]
    dashes = [line for line in lines if line.attrib["id"].startswith("dash-")]
    assert len(solid) == 2
    assert len(dashes) == dash_count
    assert all(line.attrib["stroke-linecap"] == "round" for line in lines)
    assert all("stroke-dasharray" not in line.attrib for line in lines)
    assert all(2.0 <= float(line.attrib["stroke-width"]) <= 3.5 for line in lines)
    assert len(_elements(root, "circle")) == 2
    diamond = root.find(f"{SVG}rect[@id='diamond-a']")
    assert diamond is not None
    assert 14 <= float(diamond.attrib["width"]) <= 16
    assert diamond.attrib["transform"].startswith("rotate(45")

    circles = {circle.attrib["id"]: circle for circle in _elements(root, "circle")}
    gold = circles["node-b-gold"]
    blue = circles["node-c-sky"]
    gold_outer = float(gold.attrib["r"]) + float(gold.attrib["stroke-width"]) / 2
    blue_outer = float(blue.attrib["r"]) + float(blue.attrib["stroke-width"]) / 2
    assert gold_outer > blue_outer + 1

    # The node skeleton remains equilateral without coupling the test to old coordinates.
    distance = hypot(
        float(blue.attrib["cx"]) - float(gold.attrib["cx"]),
        float(blue.attrib["cy"]) - float(gold.attrib["cy"]),
    )
    assert 60 < distance < 64


def test_logo_masters_are_flat_and_visually_hierarchical() -> None:
    root = _parse_svg(CANONICAL)
    _assert_flat(root)
    _assert_mark_skeleton(root, dash_count=3)

    plate = root.find(f"{SVG}rect[@id='plate']")
    assert plate is not None
    assert plate.attrib["fill"] == NIGHT
    assert plate.attrib["stroke"] == "#1A222A"
    assert plate.attrib["width"] == plate.attrib["height"] == "128"

    lines = _elements(root, "line")
    assert all(line.attrib["stroke"] == IVORY for line in lines)
    assert root.find(f"{SVG}rect[@id='diamond-a']").attrib["fill"] == GOLD  # type: ignore[union-attr]
    assert root.find(f"{SVG}circle[@id='node-b-gold']").attrib["stroke"] == GOLD  # type: ignore[union-attr]
    assert root.find(f"{SVG}circle[@id='node-c-sky']").attrib["stroke"] == SKY  # type: ignore[union-attr]


def test_small_master_has_optimized_two_dash_rhythm() -> None:
    root = _parse_svg(SMALL)
    _assert_flat(root)
    _assert_mark_skeleton(root, dash_count=2)


def test_16px_master_protects_ring_holes() -> None:
    root = _parse_svg(TINY)
    _assert_flat(root)
    _assert_mark_skeleton(root, dash_count=2)
    gold = root.find(f"{SVG}circle[@id='node-b-gold']")
    blue = root.find(f"{SVG}circle[@id='node-c-sky']")
    assert gold is not None and blue is not None
    assert float(gold.attrib["r"]) - float(gold.attrib["stroke-width"]) / 2 > 7
    assert float(blue.attrib["r"]) - float(blue.attrib["stroke-width"]) / 2 > 7


def test_required_branding_sources_exist_and_lockups_are_flat() -> None:
    required = [
        CANONICAL,
        SMALL,
        TINY,
        ROOT / "branding" / "sky-auto-player-mark-mono.svg",
        ROOT / "branding" / "sky-auto-player-mark-mono-dark.svg",
        ROOT / "branding" / "sky-auto-player-mark-mono-solid.svg",
        ROOT / "branding" / "sky-auto-player-mark-no-bg.svg",
        ROOT / "branding" / "lockup-horizontal.svg",
        ROOT / "branding" / "lockup-stacked.svg",
        ROOT / "branding" / "README.md",
        ROOT / "branding" / "scripts" / "build_ico.py",
    ]
    assert all(path.exists() for path in required)

    for path in required[2:7]:
        _assert_flat(_parse_svg(path))
    for path in required[7:9]:
        text = path.read_text(encoding="utf-8")
        assert "Sky Auto Player" in text
        assert "Play the sheet." in text
        assert "Not the keyboard." in text
        assert "<filter" not in text
        assert "<image" not in text


def test_monochrome_variants_keep_the_dashed_edge() -> None:
    for name in (
        "sky-auto-player-mark-mono.svg",
        "sky-auto-player-mark-mono-dark.svg",
        "sky-auto-player-mark-mono-solid.svg",
    ):
        root = _parse_svg(ROOT / "branding" / name)
        _assert_flat(root)
        _assert_mark_skeleton(root, dash_count=3)
        assert not any(element.attrib.get("fill") == GOLD for element in root.iter())
        assert not any(
            element.attrib.get("stroke") in {GOLD, SKY} for element in root.iter()
        )


def test_consumers_use_the_right_master_or_identical_generated_exports() -> None:
    small_copy = ROOT / "site" / "public" / "favicon.svg"
    assert small_copy.read_bytes() == SMALL.read_bytes()
    assert (
        ROOT / "site" / "public" / "assets" / "sky-auto-player-mark.svg"
    ).read_bytes() == CANONICAL.read_bytes()
    assert (
        ROOT / "site" / "public" / "assets" / "sky-auto-player-mark-mono.svg"
    ).read_bytes() == (ROOT / "branding" / "sky-auto-player-mark-mono.svg").read_bytes()
    assert (
        ROOT / "site" / "public" / "assets" / "sky-auto-player-mark-no-bg.svg"
    ).read_bytes() == (
        ROOT / "branding" / "sky-auto-player-mark-no-bg.svg"
    ).read_bytes()

    ico = (
        ROOT / "branding" / "exports" / "windows" / "sky-auto-player.ico"
    ).read_bytes()
    assert _ico_sizes(ico) == {16, 24, 32, 48, 64, 128, 256}
    assert (ROOT / "desktop" / "src-tauri" / "icons" / "icon.ico").read_bytes() == ico
    assert (ROOT / "site" / "public" / "favicon.ico").read_bytes() == ico

    touch_icon = (
        ROOT / "branding" / "exports" / "web" / "apple-touch-icon.png"
    ).read_bytes()
    assert _png_dimensions(touch_icon) == (180, 180)
    assert (
        ROOT / "site" / "public" / "apple-touch-icon.png"
    ).read_bytes() == touch_icon
