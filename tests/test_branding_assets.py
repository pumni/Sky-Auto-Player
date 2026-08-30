"""Check the branding source contract and generated consumer exports."""

from __future__ import annotations

import struct
from math import cos, hypot, isclose, radians, sin
from pathlib import Path
from re import fullmatch
from xml.etree import ElementTree

ROOT = Path(__file__).resolve().parents[1]
SVG_NS = "http://www.w3.org/2000/svg"
SVG = f"{{{SVG_NS}}}"
NIGHT = "#07090D"
IVORY = "#F4EFE3"
GOLD = "#F7DDA2"
SKY = "#B8CCD6"
PLATE_STROKE = "#1A222A"
EPSILON = 0.02

CANONICAL = ROOT / "branding" / "sky-auto-player-app-icon.svg"
SMALL = ROOT / "branding" / "sky-auto-player-app-icon-small.svg"
TINY = ROOT / "branding" / "sky-auto-player-app-icon-16.svg"

POINT_ATTRS = ("x1", "y1", "x2", "y2")
LINE_IDS = ("edge-a-b", "edge-a-c", "dash-b-c-1", "dash-b-c-2", "dash-b-c-3")
NODE_IDS = ("node-b-gold", "node-c-sky")
EXPECTED_ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)


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


def _ico_layers(data: bytes) -> dict[int, bytes]:
    assert data[:4] == b"\x00\x00\x01\x00"
    count = struct.unpack_from("<H", data, 4)[0]
    assert count == len(EXPECTED_ICO_SIZES)
    layers: dict[int, bytes] = {}
    for index in range(count):
        width, height, _colors, _reserved, _planes, _bits, size, offset = (
            struct.unpack_from("<BBBBHHII", data, 6 + index * 16)
        )
        decoded_width = 256 if width == 0 else width
        decoded_height = 256 if height == 0 else height
        assert decoded_width == decoded_height
        assert decoded_width not in layers
        layers[decoded_width] = data[offset : offset + size]
    return layers


def _png_dimensions(data: bytes) -> tuple[int, int]:
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    return struct.unpack_from(">II", data, 16)


def _assert_flat(root: ElementTree.Element, *, view_box: str | None = None) -> None:
    if view_box is not None:
        assert root.attrib["viewBox"] == view_box
    assert not _elements(root, "filter")
    assert not _elements(root, "image")
    assert not _elements(root, "linearGradient")
    assert not _elements(root, "radialGradient")


def _point(element: ElementTree.Element, x: str, y: str) -> tuple[float, float]:
    return float(element.attrib[x]), float(element.attrib[y])


def _line(root: ElementTree.Element, line_id: str) -> ElementTree.Element:
    line = root.find(f".//{SVG}line[@id='{line_id}']")
    assert line is not None, f"missing line {line_id}"
    return line


def _cross(a: tuple[float, float], b: tuple[float, float]) -> float:
    return a[0] * b[1] - a[1] * b[0]


def _subtract(a: tuple[float, float], b: tuple[float, float]) -> tuple[float, float]:
    return a[0] - b[0], a[1] - b[1]


def _distance(a: tuple[float, float], b: tuple[float, float]) -> float:
    return hypot(a[0] - b[0], a[1] - b[1])


def _projection_distance(
    point: tuple[float, float], start: tuple[float, float], end: tuple[float, float]
) -> float:
    vector = _subtract(end, start)
    length = _distance(start, end)
    return (point[0] - start[0]) * vector[0] / length + (point[1] - start[1]) * vector[
        1
    ] / length


def _mark_container(root: ElementTree.Element) -> ElementTree.Element:
    group = root.find(f"{SVG}g")
    return group if group is not None else root


def _group_transform(
    root: ElementTree.Element,
) -> tuple[ElementTree.Element, float, float, float, float]:
    group = root.find(f"{SVG}g")
    assert group is not None
    transform = group.attrib.get("transform")
    assert transform is not None
    number = r"[-+]?(?:\d+(?:\.\d*)?|\.\d+)"
    match = fullmatch(
        rf"\s*translate\(\s*({number})\s+({number})\s*\)\s+"
        rf"scale\(\s*({number})(?:\s+({number}))?\s*\)\s*",
        transform,
    )
    assert match is not None, f"unsupported lockup transform: {transform}"
    sx = float(match.group(3))
    sy = float(match.group(4)) if match.group(4) is not None else sx
    return group, float(match.group(1)), float(match.group(2)), sx, sy


def _transform_point(
    point: tuple[float, float], tx: float, ty: float, sx: float, sy: float
) -> tuple[float, float]:
    return tx + point[0] * sx, ty + point[1] * sy


def _diamond_transform(
    diamond: ElementTree.Element,
) -> tuple[float, tuple[float, float]]:
    transform = diamond.attrib.get("transform")
    assert transform is not None
    match = fullmatch(
        r"\s*rotate\(\s*([-+]?\d+(?:\.\d*)?|[-+]?\.\d+)\s+"
        r"([-+]?\d+(?:\.\d*)?|[-+]?\.\d+)\s+"
        r"([-+]?\d+(?:\.\d*)?|[-+]?\.\d+)\s*\)\s*",
        transform,
    )
    assert match is not None, f"unsupported diamond transform: {transform}"
    return float(match.group(1)), (float(match.group(2)), float(match.group(3)))


def _transformed_diamond_center(diamond: ElementTree.Element) -> tuple[float, float]:
    raw_center = (
        float(diamond.attrib["x"]) + float(diamond.attrib["width"]) / 2,
        float(diamond.attrib["y"]) + float(diamond.attrib["height"]) / 2,
    )
    angle, pivot = _diamond_transform(diamond)
    theta = radians(angle)
    offset = _subtract(raw_center, pivot)
    return (
        pivot[0] + offset[0] * cos(theta) - offset[1] * sin(theta),
        pivot[1] + offset[0] * sin(theta) + offset[1] * cos(theta),
    )


def _mark_nodes(
    container: ElementTree.Element,
) -> tuple[tuple[float, float], tuple[float, float], tuple[float, float]]:
    diamond = container.find(f"{SVG}rect[@id='diamond-a']")
    assert diamond is not None
    a = _transformed_diamond_center(diamond)
    circles = {
        circle.attrib["id"]: circle for circle in container.findall(f"{SVG}circle")
    }
    b = _point(circles["node-b-gold"], "cx", "cy")
    c = _point(circles["node-c-sky"], "cx", "cy")
    return a, b, c


def _assert_line_on_edge(
    line: ElementTree.Element,
    start: tuple[float, float],
    end: tuple[float, float],
    *,
    inset: bool = True,
) -> None:
    edge = _subtract(end, start)
    first = _point(line, "x1", "y1")
    last = _point(line, "x2", "y2")
    assert abs(_cross(edge, _subtract(first, start))) < EPSILON
    assert abs(_cross(edge, _subtract(last, start))) < EPSILON
    edge_length = _distance(start, end)
    first_t = _projection_distance(first, start, end) / edge_length
    last_t = _projection_distance(last, start, end) / edge_length
    if inset:
        assert 0.1 < first_t < last_t < 0.9
    else:
        assert 0 < first_t < last_t < 1


def _assert_mark_geometry(
    container: ElementTree.Element,
    *,
    dash_count: int,
    expected_dash_distances: tuple[float, ...] | None = None,
) -> None:
    diamond = container.find(f"{SVG}rect[@id='diamond-a']")
    assert diamond is not None
    angle, pivot = _diamond_transform(diamond)
    raw_center = (
        float(diamond.attrib["x"]) + float(diamond.attrib["width"]) / 2,
        float(diamond.attrib["y"]) + float(diamond.attrib["height"]) / 2,
    )
    assert isclose(angle, 45, abs_tol=EPSILON)
    assert _distance(pivot, raw_center) < EPSILON

    a, b, c = _mark_nodes(container)
    ab = _distance(a, b)
    ac = _distance(a, c)
    bc = _distance(b, c)
    assert isclose(ab, ac, abs_tol=EPSILON)
    assert isclose(ab, bc, abs_tol=EPSILON)
    assert isclose(ab, 62, abs_tol=EPSILON)

    _assert_line_on_edge(_line(container, "edge-a-b"), a, b)
    _assert_line_on_edge(_line(container, "edge-a-c"), a, c)

    dashes = [
        _line(container, f"dash-b-c-{index}") for index in range(1, dash_count + 1)
    ]
    centers: list[tuple[float, float]] = []
    for dash in dashes:
        _assert_line_on_edge(dash, b, c, inset=False)
        first = _point(dash, "x1", "y1")
        last = _point(dash, "x2", "y2")
        centers.append(((first[0] + last[0]) / 2, (first[1] + last[1]) / 2))
        assert 3.0 <= _distance(first, last) <= 7.0

    dash_distances = tuple(_projection_distance(center, b, c) for center in centers)
    spacing = [
        dash_distances[index + 1] - dash_distances[index]
        for index in range(len(dash_distances) - 1)
    ]
    assert spacing and max(spacing) - min(spacing) < 0.05
    if expected_dash_distances is not None:
        assert len(dash_distances) == len(expected_dash_distances)
        for actual, expected in zip(
            dash_distances, expected_dash_distances, strict=True
        ):
            assert isclose(actual, expected, abs_tol=0.12)


def _assert_rendered_lockup_geometry(root: ElementTree.Element) -> None:
    group, tx, ty, sx, sy = _group_transform(root)
    assert isclose(sx, sy, abs_tol=EPSILON)
    a, b, c = _mark_nodes(group)
    rendered_a = _transform_point(a, tx, ty, sx, sy)
    rendered_b = _transform_point(b, tx, ty, sx, sy)
    rendered_c = _transform_point(c, tx, ty, sx, sy)
    rendered_lengths = (
        _distance(rendered_a, rendered_b),
        _distance(rendered_a, rendered_c),
        _distance(rendered_b, rendered_c),
    )
    assert isclose(rendered_lengths[0], rendered_lengths[1], rel_tol=1e-4)
    assert isclose(rendered_lengths[0], rendered_lengths[2], rel_tol=1e-4)

    for line_id, start, end in (
        ("edge-a-b", rendered_a, rendered_b),
        ("edge-a-c", rendered_a, rendered_c),
    ):
        line = _line(group, line_id)
        first = _transform_point(_point(line, "x1", "y1"), tx, ty, sx, sy)
        last = _transform_point(_point(line, "x2", "y2"), tx, ty, sx, sy)
        edge = _subtract(end, start)
        assert abs(_cross(edge, _subtract(first, start))) < EPSILON * sx
        assert abs(_cross(edge, _subtract(last, start))) < EPSILON * sx

    centers: list[tuple[float, float]] = []
    for index in range(1, 4):
        line = _line(group, f"dash-b-c-{index}")
        first = _transform_point(_point(line, "x1", "y1"), tx, ty, sx, sy)
        last = _transform_point(_point(line, "x2", "y2"), tx, ty, sx, sy)
        edge = _subtract(rendered_c, rendered_b)
        assert abs(_cross(edge, _subtract(first, rendered_b))) < EPSILON * sx
        assert abs(_cross(edge, _subtract(last, rendered_b))) < EPSILON * sx
        centers.append(((first[0] + last[0]) / 2, (first[1] + last[1]) / 2))
    rendered_dash_distances = tuple(
        _projection_distance(center, rendered_b, rendered_c) for center in centers
    )
    for actual, expected in zip(
        rendered_dash_distances, (20 * sx, 32 * sx, 44 * sx), strict=True
    ):
        assert isclose(actual, expected, abs_tol=0.2)


def _assert_geometry_parity(
    expected: ElementTree.Element, actual: ElementTree.Element
) -> None:
    expected_a, expected_b, expected_c = _mark_nodes(expected)
    actual_a, actual_b, actual_c = _mark_nodes(actual)
    for expected_point, actual_point in zip(
        (expected_a, expected_b, expected_c),
        (actual_a, actual_b, actual_c),
        strict=True,
    ):
        assert _distance(expected_point, actual_point) < EPSILON

    for line_id in LINE_IDS:
        expected_line = _line(expected, line_id)
        actual_line = _line(actual, line_id)
        for attr in POINT_ATTRS:
            assert isclose(
                float(expected_line.attrib[attr]),
                float(actual_line.attrib[attr]),
                abs_tol=EPSILON,
            )

    for node_id in NODE_IDS:
        expected_node = expected.find(f".//{SVG}circle[@id='{node_id}']")
        actual_node = actual.find(f".//{SVG}circle[@id='{node_id}']")
        assert expected_node is not None and actual_node is not None
        for attr in ("cx", "cy", "r"):
            assert isclose(
                float(expected_node.attrib[attr]),
                float(actual_node.attrib[attr]),
                abs_tol=EPSILON,
            )

    expected_diamond = expected.find(f".//{SVG}rect[@id='diamond-a']")
    actual_diamond = actual.find(f".//{SVG}rect[@id='diamond-a']")
    assert expected_diamond is not None and actual_diamond is not None
    for attr in ("x", "y", "width", "height", "transform"):
        assert expected_diamond.attrib[attr] == actual_diamond.attrib[attr]


def _assert_inset_plate(root: ElementTree.Element, width: float, height: float) -> None:
    plate = root.find(f"{SVG}rect[@id='plate']")
    assert plate is not None
    stroke = float(plate.attrib["stroke-width"])
    x = float(plate.attrib["x"])
    y = float(plate.attrib["y"])
    right = x + float(plate.attrib["width"])
    bottom = y + float(plate.attrib["height"])
    assert x >= stroke / 2
    assert y >= stroke / 2
    assert right <= width - stroke / 2
    assert bottom <= height - stroke / 2


def test_logo_masters_lock_the_canonical_geometry_and_plate() -> None:
    root = _parse_svg(CANONICAL)
    _assert_flat(root, view_box="0 0 128 128")
    _assert_mark_geometry(
        _mark_container(root), dash_count=3, expected_dash_distances=(20, 32, 44)
    )
    _assert_inset_plate(root, 128, 128)

    plate = root.find(f"{SVG}rect[@id='plate']")
    assert plate is not None
    assert plate.attrib["fill"] == NIGHT
    assert plate.attrib["stroke"] == PLATE_STROKE
    assert float(plate.attrib["stroke-width"]) == 0.5

    lines = _elements(root, "line")
    assert all(line.attrib["stroke"] == IVORY for line in lines)
    assert root.find(f"{SVG}rect[@id='diamond-a']").attrib["fill"] == GOLD  # type: ignore[union-attr]
    assert root.find(f"{SVG}circle[@id='node-b-gold']").attrib["stroke"] == GOLD  # type: ignore[union-attr]
    assert root.find(f"{SVG}circle[@id='node-c-sky']").attrib["stroke"] == SKY  # type: ignore[union-attr]


def test_optical_masters_preserve_invariants_with_intentional_tuning() -> None:
    for path, dash_count in ((SMALL, 2), (TINY, 2)):
        root = _parse_svg(path)
        _assert_flat(root, view_box="0 0 128 128")
        _assert_mark_geometry(
            _mark_container(root),
            dash_count=dash_count,
            expected_dash_distances=(21.5, 37.5),
        )
        _assert_inset_plate(root, 128, 128)

    root = _parse_svg(TINY)
    gold = root.find(f"{SVG}circle[@id='node-b-gold']")
    blue = root.find(f"{SVG}circle[@id='node-c-sky']")
    assert gold is not None and blue is not None
    assert float(gold.attrib["r"]) - float(gold.attrib["stroke-width"]) / 2 > 7
    assert float(blue.attrib["r"]) - float(blue.attrib["stroke-width"]) / 2 > 7


def test_no_bg_is_semantically_transparent() -> None:
    root = _parse_svg(ROOT / "branding" / "sky-auto-player-mark-no-bg.svg")
    _assert_flat(root, view_box="0 0 128 128")
    assert root.find(f"{SVG}rect[@id='plate']") is None
    for node_id in NODE_IDS:
        node = root.find(f"{SVG}circle[@id='{node_id}']")
        assert node is not None
        assert node.attrib["fill"] == "none"


def test_variants_and_lockups_match_the_canonical_geometry() -> None:
    canonical = _mark_container(_parse_svg(CANONICAL))
    for name in (
        "sky-auto-player-mark-no-bg.svg",
        "sky-auto-player-mark-mono.svg",
        "sky-auto-player-mark-mono-dark.svg",
        "sky-auto-player-mark-mono-solid.svg",
        "lockup-horizontal.svg",
        "lockup-stacked.svg",
    ):
        root = _parse_svg(ROOT / "branding" / name)
        _assert_geometry_parity(canonical, _mark_container(root))
        if name.startswith("lockup-"):
            _assert_rendered_lockup_geometry(root)


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
        _assert_flat(_parse_svg(path), view_box="0 0 128 128")
    for path in required[7:9]:
        root = _parse_svg(path)
        view_box = root.attrib["viewBox"].split()
        width, height = (float(view_box[index]) for index in (2, 3))
        ratio = width / height
        if path.name == "lockup-horizontal.svg":
            assert 2.8 <= ratio <= 3.2
        else:
            assert 1.2 <= ratio <= 1.6
        _assert_inset_plate(root, width, height)
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
        _assert_flat(root, view_box="0 0 128 128")
        _assert_mark_geometry(
            _mark_container(root),
            dash_count=3,
            expected_dash_distances=(20, 32, 44),
        )
        assert not any(element.attrib.get("fill") == GOLD for element in root.iter())
        assert not any(
            element.attrib.get("stroke") in {GOLD, SKY} for element in root.iter()
        )


def test_consumers_use_the_right_master_or_identical_generated_exports() -> None:
    assert (ROOT / "site" / "public" / "favicon.svg").read_bytes() == SMALL.read_bytes()
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
    assert _ico_sizes(ico) == set(EXPECTED_ICO_SIZES)
    ico_layers = _ico_layers(ico)
    assert (
        ico_layers[16]
        == (ROOT / "branding" / "exports" / "web" / "favicon-16x16.png").read_bytes()
    )
    assert (
        ico_layers[32]
        == (ROOT / "branding" / "exports" / "web" / "favicon-32x32.png").read_bytes()
    )
    assert (ROOT / "desktop" / "src-tauri" / "icons" / "icon.ico").read_bytes() == ico
    assert (ROOT / "site" / "public" / "favicon.ico").read_bytes() == ico

    for name, size in (("favicon-16x16.png", 16), ("favicon-32x32.png", 32)):
        export = ROOT / "branding" / "exports" / "web" / name
        public = ROOT / "site" / "public" / name
        assert _png_dimensions(export.read_bytes()) == (size, size)
        assert public.read_bytes() == export.read_bytes()

    touch_icon = (
        ROOT / "branding" / "exports" / "web" / "apple-touch-icon.png"
    ).read_bytes()
    assert _png_dimensions(touch_icon) == (180, 180)
    assert (
        ROOT / "site" / "public" / "apple-touch-icon.png"
    ).read_bytes() == touch_icon


def test_website_routes_small_favicon_deterministically() -> None:
    layout = (ROOT / "site" / "src" / "layouts" / "BaseLayout.astro").read_text(
        encoding="utf-8"
    )
    assert "/favicon-16x16.png" in layout
    assert 'sizes="16x16"' in layout
    assert "/favicon-32x32.png" in layout
    assert 'sizes="32x32"' in layout
    assert 'type="image/svg+xml"' not in layout

    footer = (
        ROOT / "site" / "src" / "components" / "layout" / "SiteFooter.astro"
    ).read_text(encoding="utf-8")
    assert "/favicon.svg" in footer
    assert 'width="24" height="24"' in footer
