#!/usr/bin/env python3
"""box.py <scale> <png>...: where the composer's field stands in a screenshot of the app: its rounded box is the
lowest run of its own grey (242, 242, 247) at least 20 points tall in the column 100 points from the leading edge;
its sides are the first and last pixels of that grey along the row through its middle. Also the ground's top: the
first row above the box that is neither the box's grey nor the ground's."""
import sys
from pixels import Picture

def near(p, q, by=2):
    return all(abs(a - b) <= by for a, b in zip(p, q))

BOX = (242, 242, 247)
scale = float(sys.argv[1])
print("| screenshot | size, points | box bottom from display bottom | box top | box leading edge | box trailing edge from display's | ground below the box |")
print("|---|---|---|---|---|---|---|")
for path in sys.argv[2:]:
    im = Picture(path)
    w, h = im.size
    x = int(100 * scale)
    runs, y = [], h - 1
    while y >= 0:
        if near(im.getpixel((x, y)), BOX):
            start = y
            while y >= 0 and near(im.getpixel((x, y)), BOX):
                y -= 1
            if start - y >= 20 * scale:
                runs.append((start, y + 1))
                break
        else:
            y -= 1
    name = path.split("/")[-1].replace(".png", "")
    if not runs:
        print(f"| {name} | {w/scale:.0f}x{h/scale:.0f} | no box found | | | | |")
        continue
    bottom, top = runs[0]
    mid = (bottom + top) // 2
    cols = [c for c in range(w) if near(im.getpixel((c, mid)), BOX)]
    left, right = min(cols), max(cols)
    ground = im.getpixel((x, min(h - 1, bottom + int(3 * scale))))
    print(f"| {name} | {w/scale:.0f}x{h/scale:.0f} | {(h-1-bottom)/scale:.2f} | {(h-top)/scale:.2f} | {left/scale:.2f} | {(w-1-right)/scale:.2f} | rgb {ground} to the display's bottom: {all(near(im.getpixel((x, yy)), ground) for yy in range(bottom + int(3*scale), h))} |")
