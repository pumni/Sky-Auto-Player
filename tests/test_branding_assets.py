"""Keep the approved logo geometry and generated consumer exports aligned."""

from __future__ import annotations

import struct
from pathlib import Path
from xml.etree import ElementTree

ROOT = Path(__file__).resolve().parents[1]
SVG_NS = 'http://www.w3.org/2000/svg'
SVG = f'{{{SVG_NS}}}'

CANONICAL = ROOT / 'branding' / 'sky-auto-player-app-icon.svg'
MONO = ROOT / 'branding' / 'sky-auto-player-mark-mono.svg'


def _parse_svg(path: Path) -> ElementTree.Element:
    return ElementTree.parse(path).getroot()


def _line_signature(line: ElementTree.Element) -> tuple[str, ...]:
    return tuple(line.attrib[key] for key in ('x1', 'y1', 'x2', 'y2'))


def _ico_sizes(data: bytes) -> set[int]:
    assert data[:4] == b'\x00\x00\x01\x00'
    count = struct.unpack_from('<H', data, 4)[0]
    return {
        256 if width == 0 else width
        for width, _height in (
            struct.unpack_from('<BB', data, 6 + index * 16)
            for index in range(count)
        )
    }


def _png_dimensions(data: bytes) -> tuple[int, int]:
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    return struct.unpack_from('>II', data, 16)


def test_canonical_logo_preserves_approved_flat_geometry() -> None:
    root = _parse_svg(CANONICAL)
    assert root.attrib['viewBox'] == '0 0 128 128'
    assert not root.findall(f'.//{SVG}filter')
    assert not root.findall(f'.//{SVG}image')
    assert not root.findall(f'.//{SVG}linearGradient')
    assert not root.findall(f'.//{SVG}radialGradient')

    lines = root.findall(f'{SVG}line')
    assert [_line_signature(line) for line in lines] == [
        ('40', '33', '40', '95'),
        ('40', '33', '93.6936', '64'),
        ('53.8564', '87', '59.0526', '84'),
        ('64.2487', '81', '69.4449', '78'),
        ('74.6410', '75', '79.8372', '72'),
    ]
    assert all(line.attrib['stroke'] == '#F4EFE3' for line in lines)
    assert all(line.attrib['stroke-linecap'] == 'round' for line in lines)

    plate, diamond = root.findall(f'{SVG}rect')
    assert plate.attrib == {'width': '128', 'height': '128', 'rx': '28', 'fill': '#07090D'}
    assert diamond.attrib['transform'] == 'rotate(45 40 33)'
    assert diamond.attrib['fill'] == '#F7DDA2'
    assert diamond.attrib['stroke'] == '#F4EFE3'

    circles = root.findall(f'{SVG}circle')
    assert [(circle.attrib['cx'], circle.attrib['cy']) for circle in circles] == [
        ('40', '95'),
        ('93.6936', '64'),
    ]
    assert [circle.attrib['stroke'] for circle in circles] == ['#F7DDA2', '#B8CCD6']


def test_monochrome_derivative_keeps_the_same_connection_skeleton() -> None:
    root = _parse_svg(MONO)
    assert root.attrib['viewBox'] == '0 0 128 128'
    assert [_line_signature(line) for line in root.findall(f'{SVG}line')] == [
        ('40', '33', '40', '95'),
        ('40', '33', '93.6936', '64'),
        ('53.8564', '87', '59.0526', '84'),
        ('64.2487', '81', '69.4449', '78'),
        ('74.6410', '75', '79.8372', '72'),
    ]
    assert not root.findall(f'.//{SVG}filter')
    assert not root.findall(f'.//{SVG}image')
    assert all(element.attrib.get('fill') != '#F7DDA2' for element in root.iter())


def test_consumers_use_identical_generated_exports() -> None:
    assert (ROOT / 'site' / 'public' / 'assets' / 'sky-auto-player-mark.svg').read_bytes() == CANONICAL.read_bytes()
    assert (ROOT / 'site' / 'public' / 'favicon.svg').read_bytes() == CANONICAL.read_bytes()

    ico = (ROOT / 'branding' / 'exports' / 'windows' / 'sky-auto-player.ico').read_bytes()
    assert _ico_sizes(ico) == {16, 24, 32, 48, 64, 256}
    assert (ROOT / 'desktop' / 'src-tauri' / 'icons' / 'icon.ico').read_bytes() == ico
    assert (ROOT / 'site' / 'public' / 'favicon.ico').read_bytes() == ico

    touch_icon = (ROOT / 'branding' / 'exports' / 'web' / 'apple-touch-icon.png').read_bytes()
    assert _png_dimensions(touch_icon) == (180, 180)
    assert (ROOT / 'site' / 'public' / 'apple-touch-icon.png').read_bytes() == touch_icon
