#!/usr/bin/env python3
"""feet.py <foot log>...: for each way of standing the foot, where the field and the foot's ground stand with the
keyboard down, up and down again, where the list's last row ends, and whether the field kept focus."""
import re, sys
print("| log | state | field bottom from window bottom | foot content top..bottom from bottom | ground bottom | last row ends at | list's bottom inset | row to foot gap |")
print("|---|---|---|---|---|---|---|---|")
for path in sys.argv[1:]:
    name = path.split("/")[-1].replace(".log", "").replace("foot-", "")
    states = {}
    order = []
    focus = []
    for line in open(path, errors="replace"):
        m = re.search(r"dump\[(\S+) (\S+) (\S+)\] (mark (\w+)|list \w+): (.*)", line)
        if m:
            state = m.group(3)
            if state not in states:
                states[state] = {}
                order.append(state)
            if m.group(5):
                f = re.search(r"fromWindowBottom=(\S+) topFromWindowBottom=(\S+)", m.group(6))
                states[state][m.group(5)] = (float(f.group(1)), float(f.group(2)))
            else:
                f = re.search(r"adjustedInset=([\d.,-]+)", m.group(6))
                states[state]["inset"] = f.group(1).split(",")[2]
        m = re.search(r"field focused=(\w+)", line)
        if m:
            focus.append(m.group(1))
    for state in order:
        s = states[state]
        field, foot, ground, last = s.get("field"), s.get("foot"), s.get("ground"), s.get("lastrow")
        gap = f"{last[0] - foot[1]:+.1f}" if last and foot else "?"
        print(f"| {name} | {state} | {field[0] if field else '?'} | {foot[1] if foot else '?'}..{foot[0] if foot else '?'} | {ground[0] if ground else '?'} | {last[0] if last else 'not on screen'} | {s.get('inset','?')} | {gap} |")
    print(f"| {name} | focus | {' -> '.join(focus) or 'never focused'} | | | | | |")
