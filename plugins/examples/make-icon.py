#!/usr/bin/env python3
"""Draws the example plugins' 128x128 icons (no third-party art).

    python3 -I plugins/examples/make-icon.py

Standard library only (zlib, struct). The output is deterministic: the same
script always writes the same bytes, so the committed PNGs can be checked
with `git diff --exit-code` after running it.
"""

import os
import struct
import zlib

SIZE = 128
HERE = os.path.dirname(os.path.abspath(__file__))


def png(pixels):
    """RGBA rows -> PNG bytes (colour type 6, no interlace, level 9)."""
    raw = b"".join(b"\x00" + bytes(row) for row in pixels)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def canvas(background, radius=24):
    """A rounded square in `background` on a transparent canvas."""
    rows = []
    for y in range(SIZE):
        row = []
        for x in range(SIZE):
            cx = min(max(x, radius), SIZE - 1 - radius)
            cy = min(max(y, radius), SIZE - 1 - radius)
            inside = (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius
            row.extend(background + (255,) if inside else (0, 0, 0, 0))
        rows.append(row)
    return rows


def put(rows, x, y, colour):
    if 0 <= x < SIZE and 0 <= y < SIZE:
        rows[y][4 * x : 4 * x + 4] = list(colour) + [255]


def rect(rows, x0, y0, x1, y1, colour):
    for y in range(y0, y1):
        for x in range(x0, x1):
            put(rows, x, y, colour)


def hello_world():
    """A speech bubble with three dots."""
    rows = canvas((46, 125, 50))
    white = (255, 255, 255)
    rect(rows, 26, 34, 102, 82, white)
    for i in range(14):  # the bubble's tail
        rect(rows, 40, 82 + i, 54 - i, 83 + i, white)
    for cx in (44, 64, 84):
        for y in range(52, 64):
            for x in range(cx - 6, cx + 6):
                if (x - cx + 0.5) ** 2 + (y - 57.5) ** 2 <= 30:
                    put(rows, x, y, (46, 125, 50))
    return rows


def developer_tools():
    """Angle brackets around a slash: </>."""
    rows = canvas((38, 50, 56))
    accent = (129, 212, 250)
    for t in range(-26, 27):  # "<" and ">"
        dx = abs(t)
        for w in range(-4, 5):
            put(rows, 24 + dx + w, 64 + t, accent)
            put(rows, 103 - dx + w, 64 + t, accent)
    for t in range(-30, 31):  # "/"
        for w in range(-4, 5):
            put(rows, 64 - t // 2 + w, 64 + t, (255, 255, 255))
    return rows


def main():
    for name, draw in (("hello-world", hello_world), ("developer-tools", developer_tools)):
        path = os.path.join(HERE, name, "assets", "icon-128.png")
        with open(path, "wb") as out:
            out.write(png(draw()))
        print(path)


if __name__ == "__main__":
    main()
