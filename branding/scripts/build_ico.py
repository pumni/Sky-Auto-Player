"""Assemble PNG layers from the large and small branding masters into a Windows ICO."""

from __future__ import annotations

import argparse
import struct
from pathlib import Path

EXPECTED_SIZES = (16, 24, 32, 48, 64, 128, 256)


def png_dimensions(data: bytes) -> tuple[int, int]:
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise ValueError("expected a PNG file")
    return struct.unpack_from(">II", data, 16)


def find_layers(directory: Path, sizes: tuple[int, ...]) -> dict[int, bytes]:
    layers: dict[int, bytes] = {}
    for path in sorted(directory.rglob("*.png")):
        data = path.read_bytes()
        width, height = png_dimensions(data)
        if width != height or width not in sizes:
            continue
        if width in layers:
            raise ValueError(f"duplicate {width}x{height} PNG in {directory}")
        layers[width] = data
    return layers


def build_ico(layers: dict[int, bytes]) -> bytes:
    missing = set(EXPECTED_SIZES) - set(layers)
    if missing:
        raise ValueError(
            f"missing ICO layers: {', '.join(f'{size}x{size}' for size in sorted(missing))}"
        )

    count = len(EXPECTED_SIZES)
    header_size = 6 + count * 16
    entries: list[bytes] = []
    payload = bytearray()
    offset = header_size
    for size in EXPECTED_SIZES:
        image = layers[size]
        entries.append(
            struct.pack(
                "<BBBBHHII",
                0 if size == 256 else size,
                0 if size == 256 else size,
                0,
                0,
                1,
                32,
                len(image),
                offset,
            )
        )
        payload.extend(image)
        offset += len(image)

    return struct.pack("<HHH", 0, 1, count) + b"".join(entries) + payload


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--large-dir", type=Path, required=True)
    parser.add_argument("--small-dir", type=Path, required=True)
    parser.add_argument("--tiny-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    layers = find_layers(args.large_dir, (32, 48, 64, 128, 256))
    layers.update(find_layers(args.small_dir, (24,)))
    layers.update(find_layers(args.tiny_dir, (16,)))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(build_ico(layers))


if __name__ == "__main__":
    main()
