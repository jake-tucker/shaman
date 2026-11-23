#!/usr/bin/env python3
"""regions.py <log>...: every distinct reserved region the root's reader was answered in a log, sampled each frame:
its kind, whether asked with .includeInactive, its frame, whether active, the window it was answered in, and whether
it reaches into the window's bottom 34 points."""
import re, sys
print("| log | frames sampled | window | kind | frame x,y wxh | active | reaches the bottom 34 pt | first seen at frame |")
print("|---|---|---|---|---|---|---|---|")
for path in sys.argv[1:]:
    name = path.split("/")[-1].replace(".log", "")
    seen = {}
    frames = 0
    for line in open(path, errors="replace"):
        f = re.search(r" f=(\d+) ", line)
        if f:
            frames = max(frames, int(f.group(1)))
        if "regions changed:" not in line and "root regions:" not in line:
            continue
        window = re.search(r"window=0,0 (\S+)", line).group(1)
        height = float(window.split("x")[1])
        for who, kind, inactive, body in re.findall(r"(reader|window)\.(occlusion|division)(\+inactive)?=((?:\[[^\]]*\])+|none)", line):
            for x, y, w, h, active in re.findall(r"\[([\d.-]+),([\d.-]+) ([\d.]+)x([\d.]+) active=(\w+)", body):
                low = float(y) + float(h) > height - 34
                key = (window, kind, f"{x},{y} {w}x{h}", active, "yes" if low else "no")
                seen.setdefault(key, int(f.group(1)) if f else 0)
    if not seen:
        print(f"| {name} | {frames} | | none answered | | | | |")
    for (window, kind, frame, active, low), first in sorted(seen.items(), key=lambda kv: kv[1]):
        print(f"| {name} | {frames} | {window} | {kind} | {frame} | {active} | {low} | {first} |")
