#!/usr/bin/env python3
"""Writes the few more files the Claude third of the 0807 probe reads, into the directory named.

    python3 claude_extra_fixtures.py <dir>

Beside `make_fixtures.py`, whose `png` and `noise` it uses: PNGs of noise of about 100, 250, 500 and
750 KiB, to find where the CLI starts re-encoding; `wide-2600.png`, 2600 pixels square and small in
bytes, to see a picture scaled down; `small.pdf`, one page of text; `small.ipynb`, a notebook whose one
cell's output is the 64x64 PNG; and `tiny.ipynb`, the same around an 8x8 PNG, since the CLI left the
first one's output out as too large. Everything is deterministic.
"""
import base64, json, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from make_fixtures import noise, png, small  # noqa: E402


def wide(side):
    return png(side, side, [b"".join(bytes(((x // 20) % 256, (y // 20) % 256, 90)) for x in range(side))
                            for y in range(side)])


def pdf():
    stream = b"BT /F1 24 Tf 72 720 Td (probe 807) Tj ET"
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
               b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R "
               b"/Resources << /Font << /F1 5 0 R >> >> >>",
               b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"\nendstream",
               b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    out, offsets = b"%PDF-1.4\n", []
    for number, body in enumerate(objects, 1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % number + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    out += b"".join(b"%010d 00000 n \n" % offset for offset in offsets)
    return out + b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)


def tiny():
    return png(8, 8, [bytes((200, 40, 40)) * 8 for _ in range(8)])


def notebook(picture):
    cell = {"cell_type": "code", "execution_count": 1, "metadata": {}, "source": ["show()"],
            "outputs": [{"output_type": "display_data", "metadata": {},
                         "data": {"image/png": base64.b64encode(picture).decode(), "text/plain": ["<picture>"]}}]}
    return json.dumps({"cells": [cell], "metadata": {}, "nbformat": 4, "nbformat_minor": 5}).encode()


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    for kib in (100, 250, 500, 750):
        (out / f"noise-{kib}kib.png").write_bytes(noise(kib * 1024))
    (out / "wide-2600.png").write_bytes(wide(2600))
    (out / "small.pdf").write_bytes(pdf())
    (out / "small.ipynb").write_bytes(notebook(small()))
    (out / "tiny.ipynb").write_bytes(notebook(tiny()))
    for path in sorted(out.iterdir()):
        print(f"{path.stat().st_size:>9}  {path.name}")


if __name__ == "__main__":
    main()
