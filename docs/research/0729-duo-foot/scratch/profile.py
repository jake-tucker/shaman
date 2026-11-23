#!/usr/bin/env python3
"""profile.py <png> <scale> [x fraction] [points]: the rows of the bottom <points> points of a screenshot along one
column, as runs of like luminance, each with its distance in points from the bottom edge. For finding where a
capsule's edge, a ground's edge or the home indicator stands."""
import sys
from pixels import Picture

path, scale = sys.argv[1], float(sys.argv[2])
frac = float(sys.argv[3]) if len(sys.argv) > 3 else 0.3
points = float(sys.argv[4]) if len(sys.argv) > 4 else 90
im = Picture(path)
w, h = im.size
x = int(w * frac)
rows = int(points * scale)
runs = []
for y in range(h - 1, h - 1 - rows, -1):
    r, g, b = im.getpixel((x, y))
    lum = round(0.299 * r + 0.587 * g + 0.114 * b)
    if runs and abs(runs[-1][2] - lum) <= 1:
        runs[-1][1] = y
    else:
        runs.append([y, y, lum, (r, g, b)])
print(f"{path.split('/')[-1]}: {w}x{h}px, scale {scale}, {w/scale:.1f}x{h/scale:.1f}pt, column x={x/scale:.1f}pt")
for bottom, top, lum, rgb in runs:
    if bottom - top + 1 >= 2 or True:
        print(f"  {(h-1-bottom)/scale:6.2f}..{(h-top)/scale:6.2f}pt from bottom  ({bottom-top+1:3d}px) lum {lum} rgb {rgb}")
