#!/usr/bin/env python3
"""across.py <png> <scale> <points from bottom> [threshold]: along one row of a screenshot, the runs darker than the
threshold (edges of capsules and buttons), each with its distance in points from the leading and trailing edges."""
import sys
from pixels import Picture
path, scale, up = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
limit = int(sys.argv[4]) if len(sys.argv) > 4 else 246
im = Picture(path)
w, h = im.size
y = h - 1 - int(up * scale)
runs = []
for x in range(w):
    r, g, b = im.getpixel((x, y))
    lum = round(0.299 * r + 0.587 * g + 0.114 * b)
    if lum < limit:
        if runs and runs[-1][1] >= x - 2:
            runs[-1][1] = x
            runs[-1][2] = min(runs[-1][2], lum)
        else:
            runs.append([x, x, lum])
print(f"{path.split('/')[-1]}: row {up}pt from bottom, {w/scale:.1f}pt wide")
for a, b, lum in runs:
    print(f"  x {a/scale:7.2f}..{(b+1)/scale:7.2f}pt  (from trailing edge {(w-b-1)/scale:7.2f})  darkest {lum}")
