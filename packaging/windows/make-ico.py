#!/usr/bin/env python3
"""Put PNG images into one Windows .ico, as the PNG-compressed entries Windows Vista and later read.

    make-ico.py out.ico icon-16.png icon-32.png icon-48.png icon-256.png

Standard library only, so the Windows package job needs nothing installed for it. The PNGs are
drawn from packaging/icons/hicolor/scalable/apps/mailo.svg by `resvg` in that job.
"""

import struct
import sys


def png_size(data: bytes) -> tuple[int, int]:
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise ValueError("not a PNG")
    return struct.unpack(">II", data[16:24])


def main(out: str, pngs: list[str]) -> None:
    images = []
    for path in pngs:
        with open(path, "rb") as f:
            data = f.read()
        width, height = png_size(data)
        if width > 256 or height > 256:
            raise ValueError(f"{path}: an icon entry is at most 256 pixels a side")
        images.append((width, height, data))

    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries = b""
    for width, height, data in images:
        # 256 is written as 0: the field is one byte.
        entries += struct.pack(
            "<BBBBHHII", width % 256, height % 256, 0, 0, 1, 32, len(data), offset
        )
        offset += len(data)
    with open(out, "wb") as f:
        f.write(header + entries + b"".join(data for _, _, data in images))


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2:])
