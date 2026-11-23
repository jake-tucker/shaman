"""Reads a screenshot's pixels through sips: no imaging library is installed here."""
import os, struct, subprocess, tempfile


class Picture:
    """A screenshot's pixels, read through sips as an uncompressed BMP: no imaging library is installed here."""

    def __init__(self, path):
        out = os.path.join(tempfile.mkdtemp(), "p.bmp")
        subprocess.run(["sips", "-s", "format", "bmp", path, "--out", out], check=True, capture_output=True)
        self.data = open(out, "rb").read()
        self.offset = struct.unpack_from("<I", self.data, 10)[0]
        self.w, height = struct.unpack_from("<ii", self.data, 18)
        self.h, self.up = abs(height), height > 0
        self.bytes = struct.unpack_from("<H", self.data, 28)[0] // 8
        self.stride = (self.w * self.bytes + 3) & ~3
        self.size = (self.w, self.h)

    def getpixel(self, at):
        x, y = at
        row = self.h - 1 - y if self.up else y
        i = self.offset + row * self.stride + x * self.bytes
        b, g, r = self.data[i], self.data[i + 1], self.data[i + 2]
        return r, g, b
