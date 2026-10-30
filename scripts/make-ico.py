#!/usr/bin/env python3
"""Writes assets/icon/dawsome.ico, the icon of the Windows programs, from the icon's PNG
renders at 16, 32, 48 and 256 px.

usage: python3 -I scripts/make-ico.py [--check]
       --check: write nothing; exit 1 when the committed .ico is not what this would write

Each PNG goes into the .ico byte for byte: an .ico may hold PNG images (Windows reads them in
every size since Vista), so the icon Windows shows is exactly the PNGs, with nothing resampled
or re-encoded, and the same PNGs always give the same .ico. Standard library only.
"""

import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICON = ROOT / "assets" / "icon" / "dawsome.ico"
SIZES = (16, 32, 48, 256)
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def png_image(size):
    """The bytes of dawsome-<size>.png, checked to be a size×size, 8-bit RGBA PNG: the bit
    count the .ico's directory gives each image (32) must be the image's own."""
    path = ROOT / "assets" / "icon" / f"dawsome-{size}.png"
    data = path.read_bytes()
    if data[:8] != PNG_SIGNATURE or data[12:16] != b"IHDR":
        sys.exit(f"make-ico: {path} is not a PNG")
    width, height, depth, color = struct.unpack(">IIBB", data[16:26])
    if (width, height) != (size, size) or (depth, color) != (8, 6):
        sys.exit(f"make-ico: {path} is {width}x{height}, depth {depth}, color type {color};"
                 f" a {size}x{size} 8-bit RGBA image is needed")
    return data


def ico():
    """The .ico: a header, one 16-byte directory entry per image, then the images in order."""
    images = [png_image(size) for size in SIZES]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries = []
    for size, image in zip(SIZES, images):
        # Width and height are one byte each, so 256 is written as 0. No palette, one plane,
        # 32 bits a pixel.
        side = size % 256
        entries.append(struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(image), offset))
        offset += len(image)
    return header + b"".join(entries) + b"".join(images)


def main():
    if sys.argv[1:] not in ([], ["--check"]):
        sys.exit(__doc__.split("\n\n")[1])
    data = ico()
    if sys.argv[1:] == ["--check"]:
        if not ICON.is_file() or ICON.read_bytes() != data:
            print(f"make-ico: {ICON.relative_to(ROOT)} is not made from the PNGs as they are;"
                  " run scripts/make-ico.py", file=sys.stderr)
            return 1
        return 0
    ICON.write_bytes(data)
    print(f"make-ico: {ICON.relative_to(ROOT)}, {len(data)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
