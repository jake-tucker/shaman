#!/usr/bin/env python3
"""Writes the pictures the 0807 probe has tools return, into the directory named.

    python3 make_fixtures.py <dir>

A 64x64 PNG is drawn here; `sips` (macOS) turns it into JPEG, GIF, TIFF, BMP and HEIC; WebP and
SVG are written from bytes held below. `noise-<n>mib.png` are PNGs of noise that do not compress,
sized to stand for a large screenshot. Everything is deterministic.
"""
import base64, random, struct, subprocess, sys, zlib
from pathlib import Path


def png(width, height, rows):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    raw = b"".join(b"\x00" + row for row in rows)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def small():
    rows = []
    for y in range(64):
        rows.append(b"".join(bytes(((x * 4) % 256, (y * 4) % 256, 160 if (x // 8 + y // 8) % 2 else 40)) for x in range(64)))
    return png(64, 64, rows)


def noise(target_bytes):
    side = int((target_bytes / 3) ** 0.5)
    rng = random.Random(807)
    return png(side, side, [rng.randbytes(side * 3) for _ in range(side)])


# A 1x1 lossless WebP.
WEBP = base64.b64decode("UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==")
SVG = b'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#2a6"/></svg>\n'


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    (out / "small.png").write_bytes(small())
    for kind in ("jpeg", "gif", "tiff", "bmp", "heic"):
        subprocess.run(["sips", "-s", "format", kind, str(out / "small.png"), "--out", str(out / f"small.{kind}")],
                       check=True, capture_output=True)
    (out / "small.webp").write_bytes(WEBP)
    (out / "small.svg").write_bytes(SVG)
    for mib in (1, 3, 6):
        (out / f"noise-{mib}mib.png").write_bytes(noise(mib * 1024 * 1024))
    for path in sorted(out.iterdir()):
        print(f"{path.stat().st_size:>9}  {path.name}")


if __name__ == "__main__":
    main()
