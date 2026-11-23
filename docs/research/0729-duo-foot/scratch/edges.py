#!/usr/bin/env python3
"""edges.py <walk log>...: for each dump in a walk's log, what each reader read of the vertical bar's edge."""
import re, sys
for path in sys.argv[1:]:
    print("==", path.split("/")[-1])
    dumps = {}
    order = []
    for line in open(path, errors="replace"):
        m = re.search(r"dump\[(.*?)\] (.*)", line)
        if not m:
            continue
        label, rest = m.groups()
        if label not in dumps:
            dumps[label] = []
            order.append(label)
        dumps[label].append(rest)
    for label in order:
        cells = []
        window = ""
        for rest in dumps[label]:
            m = re.match(r"probe (\S+) (swiftui|uikit): edge=(\w+)(.*)", rest)
            if m:
                name, kind, edge, more = m.groups()
                out = " (not in a window)" if "NOT-IN-A-WINDOW" in more else ""
                if kind == "uikit":
                    w = re.search(r"window=(\S+) windowSafe=(\S+)", more)
                    if w and name == "root":
                        window = f"window {w.group(1)} safe {w.group(2)}"
                    chain = more.split("chain: ")[-1].strip() if "chain: " in more else ""
                    edges = set(re.findall(r":(nil|leading|trailing)", chain))
                    asks = " asks-off" if "asks-off" in chain else ""
                    cells.append(f"{name}/uikit={edge}{out} chain={'+'.join(sorted(edges))}{asks}")
                else:
                    cells.append(f"{name}/swiftui={edge}{out}")
            m = re.match(r"root hinge: (.*)", rest)
            if m:
                cells.append("hinge=" + m.group(1))
        print(f"  [{label}] {window}\n     " + " | ".join(cells))
