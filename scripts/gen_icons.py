#!/usr/bin/env python3
"""Generate the blobtype app icons (black blob with a tiny waveform) with the
standard library only. Usage: python3 scripts/gen_icons.py"""
import math
import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
SS = 4  # supersampling factor


def blob_radius(theta):
    return 0.42 * (1 + 0.07 * math.sin(3 * theta + 0.6) + 0.04 * math.sin(5 * theta + 1.7))


BARS = [(-0.20, 0.10), (-0.10, 0.20), (0.0, 0.30), (0.10, 0.17), (0.20, 0.09)]  # (x, half-height)
BAR_HALF_W = 0.032


def sample(x, y):
    """Colour (r,g,b,a 0..1) at normalised coords in [-0.5, 0.5]."""
    r = math.hypot(x, y)
    if r > blob_radius(math.atan2(y, x)):
        return (0.0, 0.0, 0.0, 0.0)
    for bx, hh in BARS:
        dx, dy = abs(x - bx), abs(y)
        # rounded bar
        if dx <= BAR_HALF_W and dy <= hh - BAR_HALF_W:
            return (0.96, 0.96, 0.97, 1.0)
        if dx * dx + max(0, dy - (hh - BAR_HALF_W)) ** 2 <= BAR_HALF_W**2:
            return (0.96, 0.96, 0.97, 1.0)
    return (0.0, 0.0, 0.0, 1.0)


def render(size):
    rows = []
    for py in range(size):
        row = bytearray([0])  # PNG filter: none
        for px in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(SS):
                for sx in range(SS):
                    x = (px + (sx + 0.5) / SS) / size - 0.5
                    y = (py + (sy + 0.5) / SS) / size - 0.5
                    r, g, b, a = sample(x, y)
                    acc[0] += r * a
                    acc[1] += g * a
                    acc[2] += b * a
                    acc[3] += a
            n = SS * SS
            a = acc[3] / n
            if acc[3] > 0:
                rgb = [acc[i] / acc[3] for i in range(3)]
            else:
                rgb = [0, 0, 0]
            row += bytes([round(c * 255) for c in rgb] + [round(a * 255)])
        rows.append(bytes(row))
    return rows


def png(size):
    raw = b"".join(render(size))

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def ico(pngs):
    header = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    entries, blobs = b"", b""
    for size, data in pngs:
        entries += struct.pack(
            "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset + len(blobs)
        )
        blobs += data
    return header + entries + blobs


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    cache = {s: png(s) for s in (16, 32, 48, 128, 256, 512)}
    (OUT / "32x32.png").write_bytes(cache[32])
    (OUT / "128x128.png").write_bytes(cache[128])
    (OUT / "128x128@2x.png").write_bytes(cache[256])
    (OUT / "icon.png").write_bytes(cache[512])
    (OUT / "icon.ico").write_bytes(ico([(s, cache[s]) for s in (16, 32, 48, 256)]))
    print("icons written to", OUT)


if __name__ == "__main__":
    main()
