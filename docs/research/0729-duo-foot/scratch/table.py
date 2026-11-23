#!/usr/bin/env python3
"""table.py <walk log>...: one row a dump: the window, its safe area, the hinge, and what SwiftUI / UIKit read of the
vertical bar's edge at the root, in the Sidebar, the main view, the Aside and a sheet. A reader out of the window is
left out; `!` marks a reader whose view controllers above it do not all hold the edge it read."""
import re, sys
print("| pose and where | window | safe t,l,b,r | root | sidebar | main | aside | sheet | hinge |")
print("|---|---|---|---|---|---|---|---|---|")
for path in sys.argv[1:]:
    dumps, order = {}, []
    for line in open(path, errors="replace"):
        m = re.search(r"dump\[(.*?)\] (.*)", line)
        if m:
            if m.group(1) not in dumps:
                dumps[m.group(1)] = []
                order.append(m.group(1))
            dumps[m.group(1)].append(m.group(2))
    for label in order:
        read, window, safe, hinge = {}, "", "", ""
        for rest in dumps[label]:
            m = re.match(r"probe (\S+) (swiftui|uikit): edge=(\w+)(.*)", rest)
            if m:
                name, kind, edge, more = m.groups()
                if kind == "uikit":
                    if "NOT-IN-A-WINDOW" in more:
                        read.setdefault(name.split("-")[0], {})["out"] = True
                        continue
                    w = re.search(r"window=(\S+) windowSafe=(\S+)", more)
                    if name == "root":
                        window, safe = w.group(1), w.group(2)
                    chain = more.split("chain: ")[-1]
                    if len(set(re.findall(r":(nil|leading|trailing)", chain))) > 1:
                        edge += "!"
                read.setdefault(name.split("-")[0], {})[kind] = edge
            m = re.match(r"root hinge: (.*)", rest)
            if m:
                hinge = m.group(1)
        cells = []
        for name in ["root", "sidebar", "main", "aside", "sheet"]:
            r = read.get(name)
            if not r or r.get("out"):
                cells.append("")
            else:
                cells.append(f"{r.get('swiftui','?')} / {r.get('uikit','?')}")
        print(f"| {label} | {window} | {safe} | " + " | ".join(cells) + f" | {hinge} |")
