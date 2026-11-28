#!/usr/bin/env python3
"""Writes the small project every run of the 0822 probe works in, and the pictures that stand outside it.

    <venv>/bin/python make_project.py <template-dir> <shared-dir>

Needs matplotlib and Pillow, which is why the probe keeps a venv: the same venv's `bin` leads the
PATH each agent runs under, so `python3` there can plot.

`<template-dir>` becomes the project, copied afresh for each run:

    README.md                       names `docs/revenue.png`, which is deliberately not there
    data/sales.csv                  twelve months of revenue
    src/{app,report,charts,db}.py   four modules that import one another
    docs/architecture.png           a picture already on disk, under the working directory
    design notes/login flow.png     one at a path holding spaces

`<shared-dir>` is given `team-photo.jpg`, a picture outside any project and under the home
directory, and `dashboard.png`, which each run copies to a temporary directory of its own.
"""
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from PIL import Image, ImageDraw

template, shared = Path(sys.argv[1]), Path(sys.argv[2])
for directory in (template / "data", template / "src", template / "docs", template / "design notes", shared):
    directory.mkdir(parents=True, exist_ok=True)

(template / "README.md").write_text("""# Acme dashboard

A small sales dashboard. `src/app.py` is the entry point.

## Revenue

![Monthly revenue](docs/revenue.png)

The chart above is drawn from `data/sales.csv`.
""")
(template / ".gitignore").write_text("__pycache__/\n")
(template / "data" / "sales.csv").write_text("month,revenue,orders\n" + "".join(
    f"2026-{month:02d},{revenue},{orders}\n" for month, (revenue, orders) in enumerate(
        [(41200, 310), (39800, 298), (45100, 342), (47300, 351), (52900, 398), (51000, 380),
         (56400, 421), (58800, 440), (61200, 455), (59900, 449), (66300, 492), (71800, 530)], 1)))
(template / "src" / "db.py").write_text('''"""Reads the sales data."""
import csv
from pathlib import Path

DATA = Path(__file__).resolve().parent.parent / "data" / "sales.csv"


def monthly_sales():
    with open(DATA, newline="") as source:
        return [{"month": row["month"], "revenue": int(row["revenue"]), "orders": int(row["orders"])}
                for row in csv.DictReader(source)]
''')
(template / "src" / "charts.py").write_text('''"""Turns sales rows into the series a chart draws."""
from db import monthly_sales


def revenue_series():
    rows = monthly_sales()
    return [row["month"] for row in rows], [row["revenue"] for row in rows]
''')
(template / "src" / "report.py").write_text('''"""Summarises the year."""
from charts import revenue_series
from db import monthly_sales


def summary():
    months, revenue = revenue_series()
    orders = sum(row["orders"] for row in monthly_sales())
    return {"months": len(months), "revenue": sum(revenue), "orders": orders}
''')
(template / "src" / "app.py").write_text('''"""Prints the dashboard's summary."""
from report import summary

if __name__ == "__main__":
    print(summary())
''')


def boxes(path, title, labels, size=(900, 420), colour="#0F766E"):
    picture = Image.new("RGB", size, "white")
    draw = ImageDraw.Draw(picture)
    draw.text((30, 20), title, fill="black")
    width = (size[0] - 60 - 30 * (len(labels) - 1)) // len(labels)
    for index, label in enumerate(labels):
        left = 30 + index * (width + 30)
        draw.rounded_rectangle((left, 150, left + width, 270), radius=14, outline=colour, width=4)
        draw.text((left + 20, 200), label, fill="black")
        if index:
            draw.line((left - 30, 210, left, 210), fill=colour, width=4)
    picture.save(path)


boxes(template / "docs" / "architecture.png", "Acme dashboard: architecture", ["app", "report", "charts", "db"])
boxes(template / "design notes" / "login flow.png", "Login flow", ["visit", "sign in", "two-factor", "dashboard"],
      colour="#7C3AED")
boxes(shared / "team-photo.jpg", "The team, offsite 2026", ["Ana", "Bo", "Chi"], size=(800, 400), colour="#B45309")

figure, axes = plt.subplots(figsize=(8, 4.5), dpi=100)
axes.plot(range(1, 13), [310, 298, 342, 351, 398, 380, 421, 440, 455, 449, 492, 530], marker="o")
axes.set_title("Orders by month")
figure.savefig(shared / "dashboard.png")
print("wrote", template, "and", shared)
