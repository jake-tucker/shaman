#!/usr/bin/env python3
"""PROTOTYPE, throwaway. Candidate marks for Shaman (map ticket #821).

Five candidates, each a glyph over a fill, drawn as SVG. For each this writes
an Icon Composer `.icon`, the whole picture for Windows and Linux on a plate
and standing free, renders all of them at the sizes an OS shows, and writes
`index.html`, the sheet a person chooses from.

    python3 assets/icon/prototype/build.py && open assets/icon/prototype/index.html

Needs a Mac with Xcode 26 or later (`ictool`, `sips`). Nothing else.
"""

import json
import math
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "candidates"
ICTOOL = (
    "/Applications/Xcode.app/Contents/Applications/Icon Composer.app"
    "/Contents/Executables/ictool"
)


def rrect(x, y, w, h, r):
    return (
        f"M{x + r} {y}H{x + w - r}A{r} {r} 0 0 1 {x + w} {y + r}V{y + h - r}"
        f"A{r} {r} 0 0 1 {x + w - r} {y + h}H{x + r}A{r} {r} 0 0 1 {x} {y + h - r}"
        f"V{y + r}A{r} {r} 0 0 1 {x + r} {y}Z"
    )


def spiral(turns=2.35, r0=30, r1=292, start=-0.25 * math.pi, steps=220):
    pts = []
    for i in range(steps + 1):
        t = i / steps
        a = start + t * turns * 2 * math.pi
        r = r0 + (r1 - r0) * t
        pts.append(f"{512 + r * math.cos(a):.1f} {512 + r * math.sin(a):.1f}")
    return "M" + "L".join(pts)


# Each glyph is an SVG fragment on a 1024 canvas, drawn in `{c}`.
MASK = (
    '<path fill="{c}" fill-rule="evenodd" d="'
    "M512 190C704 190 800 296 800 452C800 644 672 806 512 842"
    "C352 806 224 644 224 452C224 296 320 190 512 190Z"
    "M292 448Q394 644 496 448Q394 540 292 448Z"
    "M732 448Q630 644 528 448Q630 540 732 448Z"
    + rrect(488, 246, 48, 128, 24)
    + rrect(488, 640, 48, 136, 24)
    + '"/>'
)

DRUM = (
    '<g fill="none" stroke="{c}" stroke-width="68" stroke-linecap="round" '
    'stroke-linejoin="round">'
    '<circle cx="512" cy="512" r="270"/>'
    '<path d="M404 404L512 512L404 620"/><path d="M552 620H644"/></g>'
)

TRIDENT = (
    '<g fill="none" stroke="{c}" stroke-width="76" stroke-linecap="round" '
    'stroke-linejoin="round">'
    '<path d="M512 232V796"/>'
    '<path d="M296 262C296 452 388 566 512 578"/>'
    '<path d="M728 262C728 452 636 566 512 578"/></g>'
)

def serpent():
    """An S whose stroke thins from a head to a tail, as one outline."""
    segments = [
        ((684, 344), (644, 232), (340, 208), (340, 384)),
        ((340, 384), (340, 544), (684, 480), (684, 640)),
        ((684, 640), (684, 816), (380, 792), (340, 680)),
    ]
    points = []
    for a, b, c, d in segments:
        for i in range(60):
            t = i / 60
            u = 1 - t
            points.append(tuple(
                u**3 * a[k] + 3 * u * u * t * b[k] + 3 * u * t * t * c[k] + t**3 * d[k]
                for k in (0, 1)
            ))
    points.append(segments[-1][3])
    last = len(points) - 1
    left, right = [], []
    for i, (x, y) in enumerate(points):
        x0, y0 = points[max(i - 1, 0)]
        x1, y1 = points[min(i + 1, last)]
        length = math.hypot(x1 - x0, y1 - y0)
        nx, ny = -(y1 - y0) / length, (x1 - x0) / length
        t = i / last
        half = 64 - 44 * t + 20 * math.exp(-((t / 0.06) ** 2))
        left.append(f"{x + nx * half:.1f} {y + ny * half:.1f}")
        right.append(f"{x - nx * half:.1f} {y - ny * half:.1f}")
    (hx, hy), (tx, ty) = points[0], points[-1]
    return (
        '<g fill="{c}">'
        f'<path d="M{"L".join(left + right[::-1])}Z"/>'
        f'<circle cx="{hx}" cy="{hy}" r="84"/><circle cx="{tx}" cy="{ty}" r="20"/></g>'
    )


SERPENT = serpent()

SPIRAL = (
    '<path fill="none" stroke="{c}" stroke-width="72" stroke-linecap="round" '
    f'stroke-linejoin="round" d="{spiral()}"/>'
)

# key, name, what it is, glyph, fill, glyph colour on the fill, colour standing free
CANDIDATES = [
    ("a", "Mask", "A face: the shaman's mask, eyes and teeth cut out of one shape.",
     MASK, "#332B7A", "#F4EEDF", "#7566E0"),
    ("b", "Drum", "A frame drum with a prompt on its skin: the plainest 'this is a terminal tool'.",
     DRUM, "#1D1C22", "#FF6B35", "#FF6B35"),
    ("c", "Trident", "Three strokes into one: three Providers through one Server. Also a psi.",
     TRIDENT, "#0F766E", "#FFFFFF", "#14B8A6"),
    ("d", "Serpent", "A letter: an S in one stroke, a snake or a curl of smoke.",
     SERPENT, "#E2522B", "#FFF4E0", "#E2522B"),
    ("e", "Spiral", "A petroglyph: the spiral cut into rock, the oldest mark for a trance.",
     SPIRAL, "#EFE7D6", "#221E1B", "#C8862A"),
]

RENDITIONS = ["Default", "Dark", "TintedLight", "TintedDark", "ClearLight", "ClearDark"]
DEFAULT_SIZES = [16, 32, 64, 128, 256, 1024]
OTHER_SIZES = [32, 128]
SVG_SIZES = [16, 24, 32, 48, 256]


def svg(body):
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" '
        f'viewBox="0 0 1024 1024">{body}</svg>\n'
    )


def p3(hex_colour):
    r, g, b = (int(hex_colour[i : i + 2], 16) / 255 for i in (1, 3, 5))
    return f"display-p3:{r:.5f},{g:.5f},{b:.5f},1.00000"


def dark(hex_colour):
    r, g, b = (int(hex_colour[i : i + 2], 16) / 255 for i in (1, 3, 5))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b < 0.4


def plate(glyph, fill, colour):
    return svg(
        f'<rect x="64" y="64" width="896" height="896" rx="208" fill="{fill}"/>'
        + glyph.format(c=colour)
    )


def free(glyph, colour):
    return svg(
        '<g transform="translate(512 512) scale(1.42) translate(-512 -512)">'
        + glyph.format(c=colour)
        + "</g>"
    )


def run(args):
    done = subprocess.run(args, capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(f"{' '.join(map(str, args))}\n{done.stdout}{done.stderr}")


def write_sources():
    jobs = []
    for key, _name, _what, glyph, fill, colour, alone in CANDIDATES:
        root = OUT / key
        icon = root / "AppIcon.icon"
        renders = root / "renders"
        (icon / "Assets").mkdir(parents=True, exist_ok=True)
        renders.mkdir(parents=True, exist_ok=True)
        (icon / "Assets" / "mark.svg").write_text(svg(glyph.format(c=colour)))
        layer = {"image-name": "mark.svg", "name": "mark"}
        if dark(colour):
            # A dark glyph is lost on the black plate macOS gives the dark and
            # tinted forms, so there it takes the fill's colour.
            layer["fill-specializations"] = [
                {"appearance": appearance, "value": {"solid": p3(fill)}}
                for appearance in ("dark", "tinted")
            ]
        (icon / "icon.json").write_text(
            json.dumps(
                {
                    "fill": {"automatic-gradient": p3(fill)},
                    "groups": [
                        {
                            "layers": [layer],
                            "shadow": {"kind": "neutral", "opacity": 0.5},
                            "translucency": {"enabled": False, "value": 0.5},
                        }
                    ],
                    "supported-platforms": {"squares": ["macOS"]},
                },
                indent=2,
            )
            + "\n"
        )
        (root / "app-icon-plate.svg").write_text(plate(glyph, fill, colour))
        (root / "app-icon-free.svg").write_text(free(glyph, alone))
        for rendition in RENDITIONS:
            for size in DEFAULT_SIZES if rendition == "Default" else OTHER_SIZES:
                jobs.append([
                    ICTOOL, icon, "--export-image",
                    "--output-file", renders / f"mac-{rendition}-{size}.png",
                    "--platform", "macOS", "--rendition", rendition,
                    "--width", str(size), "--height", str(size), "--scale", "1",
                ])
        for form in ("plate", "free"):
            for size in SVG_SIZES:
                jobs.append([
                    "sips", "-s", "format", "png", "-z", str(size), str(size),
                    root / f"app-icon-{form}.svg",
                    "--out", renders / f"{form}-{size}.png",
                ])
    return jobs


CSS = """
:root { color-scheme: light dark; font: 14px/1.45 -apple-system, "Segoe UI", sans-serif; }
body { margin: 0; padding: 32px 40px 120px; background: #f3f3f5; color: #1c1c1e; }
@media (prefers-color-scheme: dark) { body { background: #161618; color: #ececf0; } }
h1 { font-size: 22px; margin: 0 0 4px; } h2 { font-size: 15px; margin: 40px 0 12px; }
p.note { margin: 0 0 8px; opacity: .7; max-width: 76ch; }
table { border-collapse: collapse; } td, th { padding: 10px 14px; text-align: center; vertical-align: middle; }
th { font-weight: 600; font-size: 12px; opacity: .7; } th.row { text-align: right; white-space: nowrap; }
td.cand { cursor: pointer; border-radius: 10px; } td.cand.on { outline: 2px solid #0a84ff; outline-offset: -2px; }
img { display: block; margin: auto; } img.px { image-rendering: pixelated; }
.strip { display: inline-flex; gap: 14px; align-items: center; padding: 10px 16px; border-radius: 14px; }
.dock-l { background: linear-gradient(135deg, #9fc4e8, #e8c7d8); } .dock-d { background: linear-gradient(135deg, #1b2440, #3a2145); }
.bar { display: inline-flex; gap: 18px; align-items: center; padding: 8px 18px; border-radius: 6px; }
.win-d { background: #1f1f23; } .win-l { background: #eef1f6; }
.title { display: inline-flex; gap: 8px; align-items: center; padding: 6px 12px; font-size: 12px; border-radius: 6px 6px 0 0; }
.title.win-d { color: #fff; } .title.win-l { color: #111; }
.rows { display: grid; grid-template-columns: max-content 1fr; gap: 12px 20px; align-items: center; }
.rows > span { font-size: 12px; opacity: .7; text-align: right; }
.play { display: flex; gap: 28px; align-items: center; flex-wrap: wrap; }
.sw { width: 28px; height: 28px; border-radius: 8px; border: 2px solid transparent; cursor: pointer; box-shadow: 0 0 0 1px #8886; }
.sw.on { border-color: #0a84ff; } .swatches { display: flex; gap: 8px; align-items: center; margin: 6px 0; }
.pair { display: flex; gap: 24px; } #switcher[hidden] { display: none; }
.swatches > span { width: 9ch; font-size: 12px; opacity: .7; }
#switcher { position: fixed; left: 50%; bottom: 22px; transform: translateX(-50%); display: flex; gap: 6px; align-items: center;
  background: #000; color: #fff; padding: 8px 10px; border-radius: 999px; box-shadow: 0 6px 24px #0008; font-weight: 600; z-index: 9; }
#switcher button { all: unset; cursor: pointer; padding: 4px 12px; border-radius: 999px; } #switcher button:hover { background: #fff3; }
#switcher span { min-width: 15ch; text-align: center; }
"""

JS = """
const C = __CANDIDATES__;
const keys = C.map(c => c.key);
const q = new URLSearchParams(location.search);
let cur = keys.includes(q.get('variant')) ? q.get('variant') : 'a';
let fill = null, ink = null;
const $ = s => document.querySelector(s);

function play() {
  const c = C.find(c => c.key === cur);
  const f = fill || c.fill, i = ink || c.ink;
  const glyph = c.glyph.replaceAll('{c}', i);
  const one = s => `<svg width="${s}" height="${s}" viewBox="0 0 1024 1024"><rect x="64" y="64" width="896" height="896" rx="208" fill="${f}"/>${glyph}</svg>`;
  const sw = (list, on, kind) => list.map(v => `<div class="sw ${v === on ? 'on' : ''}" style="background:${v}" data-${kind}="${v}" title="${v}"></div>`).join('');
  const fills = [...new Set(C.map(c => c.fill))], inks = [...new Set(C.map(c => c.ink))];
  $('#play').innerHTML = `
    <div><div class="swatches"><span>fill</span>${sw(fills, f, 'fill')}</div>
    <div class="swatches"><span>glyph</span>${sw(inks, i, 'ink')}</div></div>
    ${one(256)}${one(128)}${one(64)}${one(32)}${one(16)}`;
  document.querySelectorAll('[data-fill]').forEach(e => e.onclick = () => { fill = e.dataset.fill; play(); });
  document.querySelectorAll('[data-ink]').forEach(e => e.onclick = () => { ink = e.dataset.ink; play(); });
}

function go(k) {
  cur = k; fill = ink = null;
  q.set('variant', k); history.replaceState(null, '', '?' + q);
  document.querySelectorAll('td.cand').forEach(td => td.classList.toggle('on', td.dataset.k === k));
  document.querySelectorAll('section.detail').forEach(d => d.hidden = d.dataset.k !== k);
  const c = C.find(c => c.key === k);
  $('#label').textContent = `${k.toUpperCase()} (${c.name})`;
  $('#playing').textContent = c.name;
  play();
}
function step(d) { go(keys[(keys.indexOf(cur) + d + keys.length) % keys.length]); }
document.querySelectorAll('td.cand').forEach(td => td.onclick = () => go(td.dataset.k));
$('#prev').onclick = () => step(-1); $('#next').onclick = () => step(1);
addEventListener('keydown', e => { if (e.key === 'ArrowLeft') step(-1); if (e.key === 'ArrowRight') step(1); });
$('#switcher').hidden = false;
go(cur);
"""


def img(key, file, size, cls=""):
    return (
        f'<img src="candidates/{key}/renders/{file}.png" width="{size}" '
        f'height="{size}" class="{cls}">'
    )


def overview():
    def bar(form):
        return lambda k: (
            '<div class="bar win-d">'
            + "".join(img(k, f"{form}-{size}", size) for size in (48, 24, 16))
            + "</div>"
        )

    rows = [
        ("macOS, 128", lambda k: img(k, "mac-Default-128", 128)),
        ("64 (a Dock at 2x)", lambda k: img(k, "mac-Default-64", 64)),
        ("32", lambda k: img(k, "mac-Default-32", 32)),
        ("16", lambda k: img(k, "mac-Default-16", 16)),
        ("16, eight times over", lambda k: img(k, "mac-Default-16", 128, "px")),
        ("Windows or Linux, on a plate<br>48 / 24 / 16", bar("plate")),
        ("standing free<br>48 / 24 / 16", bar("free")),
        ("16 on a plate, eight times over", lambda k: img(k, "plate-16", 128, "px")),
        ("16 standing free, eight times over",
         lambda k: '<div class="bar win-d">' + img(k, "free-16", 128, "px") + "</div>"),
    ]
    out = "<tr><th></th>" + "".join(
        f"<th>{c[0].upper()} · {c[1]}</th>" for c in CANDIDATES
    ) + "</tr>"
    for label, cell in rows:
        out += f'<tr><th class="row">{label}</th>' + "".join(
            f'<td class="cand" data-k="{c[0]}">{cell(c[0])}</td>' for c in CANDIDATES
        ) + "</tr>"
    return out


def detail(key, name, what):
    def dock(cls, size, at):
        return f'<div class="strip {cls}">' + "".join(
            img(key, f"mac-{form}-{at}", size) for form in RENDITIONS
        ) + "</div>"

    def win(cls, form):
        return (
            f'<div><div class="title {cls}">{img(key, form + "-16", 16)} Shaman</div><br>'
            f'<div class="bar {cls}">'
            + "".join(img(key, f"{form}-{size}", size) for size in (24, 32, 48))
            + img(key, f"{form}-256", 96)
            + "</div></div>"
        )

    return f"""
<section class="detail" data-k="{key}">
<h2>{key.upper()} · {name}</h2><p class="note">{what}</p>
<div class="rows">
  <span>macOS at 1024, shown at half</span><div>{img(key, "mac-Default-1024", 512)}</div>
  <span>The six forms macOS draws<br>(default, dark, tinted light and dark,<br>clear light and dark), at 64 pt</span>
  <div>{dock("dock-l", 64, 128)} {dock("dock-d", 64, 128)}</div>
  <span>the same at 32 px</span><div>{dock("dock-l", 32, 32)} {dock("dock-d", 32, 32)}</div>
  <span>Windows or Linux on a plate:<br>title bar 16, then 24, 32, 48, 96</span>
  <div class="pair">{win("win-d", "plate")}{win("win-l", "plate")}</div>
  <span>standing free</span><div class="pair">{win("win-d", "free")}{win("win-l", "free")}</div>
</div>
</section>"""


def write_sheet():
    data = [
        {"key": k, "name": n, "glyph": g, "fill": f, "ink": c}
        for k, n, _what, g, f, c, _alone in CANDIDATES
    ]
    details = "".join(detail(k, n, w) for k, n, w, *_ in CANDIDATES)
    html = f"""<!doctype html>
<meta charset="utf-8"><title>PROTOTYPE: Shaman's mark</title>
<style>{CSS}</style>
<h1>Shaman's mark: five candidates</h1>
<p class="note">Throwaway prototype for map ticket #821. Every macOS picture is what <code>ictool</code> renders
from the candidate's <code>.icon</code> at that size; every Windows and Linux picture is <code>sips</code> on its SVG at that size.
Nothing is scaled by the browser but the rows that say so. The purple of the tinted forms is <code>ictool</code>'s own; a person's Mac tints with the colour they chose. Press a column, or ← and →, to see one candidate whole.</p>
<h2>Side by side</h2>
<table>{overview()}</table>
{details}
<h2>Any glyph on any fill: <span id="playing"></span></h2>
<p class="note">Drawn by the browser from the SVG, so judge the pairing here and the pixels above.</p>
<div id="play" class="play"></div>
<div id="switcher" hidden><button id="prev">←</button><span id="label"></span><button id="next">→</button></div>
<script>{JS.replace("__CANDIDATES__", json.dumps(data))}</script>
"""
    (HERE / "index.html").write_text(html)


def main():
    jobs = write_sources()
    with ThreadPoolExecutor(max_workers=8) as pool:
        list(pool.map(run, jobs))
    write_sheet()
    print(f"{len(jobs)} pictures; open {HERE / 'index.html'}")


if __name__ == "__main__":
    main()
