#!/usr/bin/env python3
"""A stdio MCP server whose tools return pictures, for the 0807 probe.

    python3 picture_mcp.py <fixtures-dir>

Speaks newline-delimited JSON-RPC, standard library only. The pictures are the files
`make_fixtures.py` wrote. Tools:

- `picture {kind}`: one image block; kind is png, jpeg, gif, webp, tiff, bmp, heic or svg.
- `two_pictures`: a text block, a PNG block, a text block and a JPEG block.
- `big_picture {mib}`: one PNG block of about 1, 3 or 6 MiB.
- `resource_picture`: an embedded resource whose blob is a PNG.
- `linked_picture`: a resource link to a PNG on disk.

With `PICTURE_MCP_LOG` set, each request and the size of each response is appended to that file.
"""
import base64, json, os, sys
from pathlib import Path

FIXTURES = Path(sys.argv[1])
MIME = {"png": "image/png", "jpeg": "image/jpeg", "gif": "image/gif", "webp": "image/webp",
        "tiff": "image/tiff", "bmp": "image/bmp", "heic": "image/heic", "svg": "image/svg+xml"}
LOG = os.environ.get("PICTURE_MCP_LOG")


def log(line):
    if LOG:
        with open(LOG, "a") as handle:
            handle.write(line + "\n")


def b64(name):
    return base64.b64encode((FIXTURES / name).read_bytes()).decode()


def image(name, kind):
    return {"type": "image", "data": b64(name), "mimeType": MIME[kind]}


TOOLS = [
    {"name": "picture", "description": "Returns one small picture of the kind asked for.",
     "inputSchema": {"type": "object", "properties": {"kind": {"type": "string", "enum": sorted(MIME)}}, "required": ["kind"]}},
    {"name": "two_pictures", "description": "Returns text and two pictures, a PNG and a JPEG.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "big_picture", "description": "Returns one large PNG of about the number of MiB asked for (1, 3 or 6).",
     "inputSchema": {"type": "object", "properties": {"mib": {"type": "integer", "enum": [1, 3, 6]}}, "required": ["mib"]}},
    {"name": "resource_picture", "description": "Returns a PNG as an embedded resource.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "linked_picture", "description": "Returns a link to a PNG as a resource link.",
     "inputSchema": {"type": "object", "properties": {}}},
]


def call(name, arguments):
    if name == "picture":
        kind = arguments.get("kind", "png")
        return [image(f"small.{kind}", kind)]
    if name == "two_pictures":
        return [{"type": "text", "text": "first, a PNG"}, image("small.png", "png"),
                {"type": "text", "text": "then a JPEG"}, image("small.jpeg", "jpeg")]
    if name == "big_picture":
        return [image(f"noise-{int(arguments.get('mib', 1))}mib.png", "png")]
    if name == "resource_picture":
        return [{"type": "resource", "resource": {"uri": "picture://small.png", "mimeType": "image/png", "blob": b64("small.png")}}]
    if name == "linked_picture":
        return [{"type": "resource_link", "uri": (FIXTURES / "small.png").resolve().as_uri(), "name": "small.png", "mimeType": "image/png"}]
    raise KeyError(name)


def answer(message):
    method, params = message.get("method"), message.get("params") or {}
    if method == "initialize":
        return {"protocolVersion": params.get("protocolVersion", "2025-06-18"), "capabilities": {"tools": {}},
                "serverInfo": {"name": "pictures", "version": "0.807.0"}}
    if method == "ping":
        return {}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        try:
            return {"content": call(params.get("name"), params.get("arguments") or {}), "isError": False}
        except Exception as error:  # a tool's failure is a result, not a protocol error
            return {"content": [{"type": "text", "text": f"failed: {error!r}"}], "isError": True}
    return None


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    message = json.loads(line)
    log("< " + line[:400])
    if "id" not in message:
        continue
    result = answer(message)
    reply = {"jsonrpc": "2.0", "id": message["id"]}
    if result is None:
        reply["error"] = {"code": -32601, "message": f"no such method: {message.get('method')}"}
    else:
        reply["result"] = result
    out = json.dumps(reply)
    log(f"> {len(out)} bytes for {message.get('method')}")
    sys.stdout.write(out + "\n")
    sys.stdout.flush()
