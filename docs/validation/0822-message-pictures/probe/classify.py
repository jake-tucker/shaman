#!/usr/bin/env python3
"""Says how each kept capture's Messages name the picture, and whether ADR-0086's rule finds it.

    cargo run --example message_picture_names -- ../captures/*/*/*.json > names.jsonl
    python3 classify.py names.jsonl > ../summary.json
    python3 classify.py names.jsonl --table > table.md

`names.jsonl` is what `examples/message_picture_names.rs` printed: each Message read by
`pulldown-cmark` into the places a file could be named. For every capture this takes the pictures
the case is about (the one asked for, or every raster picture the Turn made) and, for each
Message, finds the strongest way it names each one:

- `image`: an image's destination that the rule of ADR-0086 reads to that file. The only one the
  rule as written finds.
- `link`: a link's destination that the same reading takes to that file (a trailing `:12` or
  `#L12` line taken off first, as Codex writes them).
- `code`: a code span that is the path, or holds it.
- `block`: a code block that holds the path.
- `prose`: plain text that holds the path or the file's name.
- `broken-image`: text that looks like `![…](…)` and names the file, but that CommonMark did not
  read as an image, as happens when a destination holds a bare space.
- `none`: the Message does not name it.

A name is read as the ADR reads one, on text alone and never against the disk: an absolute path
as written; `~/` from the home directory; a `file:` URL with no host or `localhost` as its path,
percent-decoded; anything else from the Execution Directory, as written and then percent-decoded.
`/private` before `/var` or `/tmp` is taken off both sides, as following symlinks would.

For each naming it also records the path's `form` (`absolute`, `relative`, `tilde`, `file-url`,
`name-only`) and, where the path holds a space, how the Message's own bytes spell it (`as-is`,
`percent`, `angle`, `escaped`). `tool_returned` says whether the Turn also read the file with the
tool that hands a picture back as a Tool Call's result (`Read`, `view`, `view_image`), which is
the picture ADR-0088 keeps whatever the Message says.
"""
import json, os, re, sys
from pathlib import Path
from urllib.parse import unquote, urlparse

HOME = "/Users/reader"
RASTER = ("png", "jpeg", "gif", "webp")
ASKED = {"show-in": "{cwd}/docs/architecture.png", "show-find": "{cwd}/docs/architecture.png",
         "show-out": "{out}/screens/dashboard.png", "show-space": "{cwd}/design notes/login flow.png",
         "show-home": "{shared}/team-photo.jpg"}
# The tools that hand a picture back as a Tool Call's result, which ADR-0088 keeps: Claude's, Copilot's, Codex's.
VIEWERS = ("Read", "view", "imageView")
RANK = ["image", "link", "code", "block", "prose", "broken-image", "none"]


def real(path):
    path = os.path.normpath(path)
    return path[len("/private"):] if path.startswith(("/private/var/", "/private/tmp/")) else path


def real_all(text):
    return text.replace("/private/var/", "/var/").replace("/private/tmp/", "/tmp/")


def readings(name, cwd):
    """Every file the ADR's reading of one name could mean."""
    name = name.strip()
    if not name:
        return []
    if name.startswith("file:"):
        url = urlparse(name)
        return [real(unquote(url.path))] if url.netloc in ("", "localhost") else []
    if re.match(r"^[A-Za-z][A-Za-z0-9+.-]+:", name):
        return []
    found = []
    for spelling in dict.fromkeys((name, unquote(name))):
        if spelling.startswith("~/"):
            found.append(real(HOME + spelling[1:]))
        elif spelling.startswith("/"):
            found.append(real(spelling))
        elif not spelling.startswith("~"):
            found.append(real(os.path.join(cwd, spelling)))
    return found


def form_of(name):
    if name.startswith("file:"):
        return "file-url"
    if name.startswith("~"):
        return "tilde"
    return "absolute" if name.startswith("/") else "relative"


def space_spelling(text, name, kind):
    """How the Message's bytes write the spaces of a destination pulldown-cmark handed over as `name`."""
    if " " not in unquote(name):
        return None
    if "%20" in name:
        return "percent"
    if kind in ("image", "link"):
        if f"(<{name}" in text:
            return "angle"
        if name.replace(" ", "\\ ") in text:
            return "escaped"
    return "as-is"


def spellings(target, cwd):
    """The ways prose or code could write a file, longest first, each with its form."""
    relative = os.path.relpath(target, cwd)
    ways = [("/private" + target, "absolute"), (target, "absolute")]
    if target.startswith(HOME + "/"):
        ways.append(("~" + target[len(HOME):], "tilde"))
    if not relative.startswith(".."):
        ways += [("./" + relative, "relative"), (relative, "relative")]
    ways.append((os.path.basename(target), "name-only"))
    return ways


def held(value, target, cwd):
    for spelling, form in spellings(target, cwd):
        for written, spaces in ((spelling, "as-is"), (spelling.replace(" ", "%20"), "percent"),
                                (spelling.replace(" ", "\\ "), "escaped")):
            if written in value:
                return {"form": form, **({"spaces": spaces} if " " in spelling else {})}
    return None


def naming(places, text, target, cwd):
    """The strongest way one Message names one file."""
    best = {"how": "none"}

    def offer(how, **more):
        nonlocal best
        if RANK.index(how) < RANK.index(best["how"]):
            best = {"how": how, **{key: value for key, value in more.items() if value is not None}}

    for place in places:
        kind, value = place["kind"], place["value"]
        if kind in ("image", "link"):
            name = value if kind == "image" else re.sub(r"(:\d+(:\d+)?|#L\d+.*)$", "", value)
            if target in readings(name, cwd):
                offer(kind, written=value, form=form_of(name), spaces=space_spelling(text, value, kind))
        elif kind in ("code", "code_block", "text", "html"):
            how = {"code": "code", "code_block": "block"}.get(kind, "prose")
            if kind == "text":
                for broken in re.finditer(r"!\[[^\]]*\]\(([^)\n]*)\)", value):
                    if target in readings(broken.group(1), cwd):
                        offer("broken-image", written=broken.group(1), form=form_of(broken.group(1)),
                              spaces="as-is" if " " in broken.group(1) else None)
            found = held(value, target, cwd)
            if found:
                offer(how, exact=(value.strip() if kind == "code" else None), **found)
    return best


def classify(capture, places):
    cwd, out = capture["cwd"], real(capture["out"])
    case = capture["case"].split(".")[0]
    shared = str(Path(cwd).parents[4] / "shared")
    if case in ASKED:
        targets = [real(ASKED[case].format(cwd=cwd, out=out, shared=shared))]
    else:
        targets = list(dict.fromkeys(real(picture["path"]) for picture in capture["pictures"]
                                     if picture["new"] and picture["kind"] in RASTER))
    messages = [(message, found) for message, found in zip(capture["messages"], places) if not message.get("subagent")]
    # A picture a Message names that the capture did not list, as one in Copilot's own session
    # directory, which is gone with its Session by the time the capture looks: taken from the name.
    listed = {real(picture["path"]) for picture in capture["pictures"]}
    unlisted = []
    for _, found in messages:
        for place in found:
            if place["kind"] in ("image", "link", "code"):
                read = readings(re.sub(r"(:\d+(:\d+)?|#L\d+.*)$", "", place["value"]), cwd)
                known = listed.union(targets, unlisted)
                if read and "/" in place["value"] and not known.intersection(read) \
                        and read[-1].lower().endswith((".png", ".jpg", ".jpeg", ".gif", ".webp")):
                    unlisted.append(read[-1])
    if case not in ASKED:
        targets += unlisted
    rows = []
    for target in targets:
        per_message = [naming(found, message["text"], target, cwd) for message, found in messages]
        final = per_message[-1] if per_message else {"how": "none"}
        earlier = min((entry["how"] for entry in per_message[:-1]), key=RANK.index, default="none")
        rows.append({"picture": target, "on_disk": target in listed or case in ASKED, "final": final, "earlier": earlier,
                     "rule_finds": "image" in [entry["how"] for entry in per_message],
                     "tool_returned": any(tool.split(":")[0] in VIEWERS and
                                          (held(real_all(tool), target, cwd) or {}).get("form", "name-only") != "name-only"
                                          for tool in capture["tools"])})
    # Images the rule would take that are none of the pictures above: a URL, a file elsewhere.
    others = sorted({place["value"] for _, found in messages for place in found if place["kind"] == "image"
                     and not any(target in readings(place["value"], cwd) for target in targets)})
    svg = [picture["path"] for picture in capture["pictures"] if picture["new"] and picture["kind"] == "svg"]
    return {"provider": capture["provider"], "model": capture.get("model"), "case": capture["case"],
            "messages": len(messages), "pictures": rows, "other_images": others, "new_svg": svg}


def main():
    table = "--table" in sys.argv
    results = []
    for line in open(sys.argv[1], encoding="utf-8"):
        entry = json.loads(line)
        capture = json.load(open(entry["file"], encoding="utf-8"))
        result = classify(capture, entry["messages"])
        result["file"] = "/".join(Path(entry["file"]).parts[-3:])
        results.append(result)
    if not table:
        json.dump(results, sys.stdout, ensure_ascii=False, indent=1)
        print()
        return
    print("| Capture | Picture | Final Message | Form | Spaces | An earlier Message | Rule finds it | A tool returned it |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for result in results:
        rows = result["pictures"] or [None]
        for row in rows:
            if row is None:
                print(f"| `{result['file']}` | none made | | | | | | |")
                continue
            final = row["final"]
            print(f"| `{result['file']}` | `{os.path.basename(row['picture'])}` | {final['how']} | {final.get('form', '')} "
                  f"| {final.get('spaces', '')} | {row['earlier']} | {'yes' if row['rule_finds'] else 'no'} "
                  f"| {'yes' if row['tool_returned'] else 'no'} |")


if __name__ == "__main__":
    main()
