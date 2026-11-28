#!/usr/bin/env python3
"""Turns a raw capture into one that can be committed, for the 0807 probe.

    python3 elide.py < raw.jsonl > capture.jsonl

Each line of the raw capture is one JSON value as it arrived. Every string of 256 characters or
more that is base64, bare or in a `data:` URL, is replaced by an object saying what it was:

    {"_elided": "base64", "chars": 5592408, "bytes": 4194304, "sha256": "…", "sniffed": "image/png",
     "head": "iVBORw0KGgoAAAANSUhEUgAA", "data_url_prefix": "data:image/png;base64,"}

`sniffed` is the kind the decoded bytes begin as, whatever the event called them. Any other string
past 4,000 characters is cut to its first 400 with its length. Each line gains `_raw_bytes`, the
length of the line as it arrived, so the cost of an event survives the eliding.
"""
import base64, binascii, hashlib, json, re, sys

B64 = re.compile(r"^[A-Za-z0-9+/\-_\r\n]+={0,2}$")
DATA_URL = re.compile(r"^(data:[^;,]*(?:;[^;,]*)*;base64,)(.*)$", re.S)
MAGIC = [(b"\x89PNG\r\n\x1a\n", "image/png"), (b"\xff\xd8\xff", "image/jpeg"), (b"GIF8", "image/gif"),
         (b"BM", "image/bmp"), (b"II*\x00", "image/tiff"), (b"MM\x00*", "image/tiff"), (b"%PDF", "application/pdf")]


def sniff(data):
    for magic, kind in MAGIC:
        if data.startswith(magic):
            return kind
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "image/webp"
    if data[4:8] == b"ftyp":
        return "image/" + data[8:12].decode("latin1").strip()
    if data.lstrip()[:5] in (b"<svg ", b"<?xml"):
        return "image/svg+xml"
    return None


def elide(value):
    if isinstance(value, dict):
        return {key: elide(item) for key, item in value.items()}
    if isinstance(value, list):
        return [elide(item) for item in value]
    if not isinstance(value, str) or len(value) < 256:
        return value
    prefix, body = None, value
    found = DATA_URL.match(value)
    if found:
        prefix, body = found.group(1), found.group(2)
    if B64.match(body):
        try:
            data = base64.b64decode(body.replace("-", "+").replace("_", "/") + "=" * (-len(body) % 4))
        except (binascii.Error, ValueError):
            data = None
        if data is not None:
            stub = {"_elided": "base64", "chars": len(value), "bytes": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(), "sniffed": sniff(data), "head": body[:24]}
            if prefix:
                stub["data_url_prefix"] = prefix
            return stub
    if len(value) > 4000:
        return {"_elided": "text", "chars": len(value), "head": value[:400]}
    return value


for line in sys.stdin:
    stripped = line.rstrip("\n")
    if not stripped.strip():
        continue
    try:
        value = json.loads(stripped)
    except json.JSONDecodeError:
        value = {"_not_json": stripped[:400], "chars": len(stripped)}
    value = elide(value)
    if isinstance(value, dict):
        value["_raw_bytes"] = len(stripped.encode())
    else:
        value = {"_value": value, "_raw_bytes": len(stripped.encode())}
    print(json.dumps(value, ensure_ascii=False))
