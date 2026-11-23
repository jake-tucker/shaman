#!/usr/bin/env python3
"""indicator.py <scale> <png>...: whether a screenshot draws a home indicator: a run of rows in the bottom 20 points
of the centre column that differs from the row 2 points up from the bottom edge, and how wide it is."""
import sys
from pixels import Picture

def lum(p):
    return 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]

scale = float(sys.argv[1])
for path in sys.argv[2:]:
    im = Picture(path)
    w, h = im.size
    x = w // 2
    base = lum(im.getpixel((x, h - 1 - int(2 * scale))))
    rows = [y for y in range(h - 1 - int(20 * scale), h - int(3 * scale)) if abs(lum(im.getpixel((x, y))) - base) > 25]
    name = path.split("/")[-1]
    if not rows:
        print(f"| {name} | {w/scale:.0f}x{h/scale:.0f} | none drawn | | |")
        continue
    top, bottom = min(rows), max(rows)
    y = (top + bottom) // 2
    left = x
    while left > 0 and abs(lum(im.getpixel((left - 1, y))) - base) > 25:
        left -= 1
    right = x
    while right < w - 1 and abs(lum(im.getpixel((right + 1, y))) - base) > 25:
        right += 1
    print(f"| {name} | {w/scale:.0f}x{h/scale:.0f} | drawn | {(h-1-bottom)/scale:.1f}..{(h-top)/scale:.1f} pt from the bottom | {(right-left+1)/scale:.0f} pt wide, x {left/scale:.0f}..{(right+1)/scale:.0f} |")
