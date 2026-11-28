#!/usr/bin/env python3
"""Drives a live `codex app-server` the way Shaman's Codex adapter does, for the 0807 probe.

    python3 codex_capture.py <case> --scratch <dir> --fixtures <dir> [options] [prompt ...]
    python3 codex_capture.py --list

Standard library only (Python 3.9). One run is one `codex app-server` process: `initialize` and
`initialized` as `src/provider/codex/transport.rs` sends them, then `thread/start` (or
`thread/resume` with `--resume <thread-id>`) as `src/provider/codex/session.rs` does, with
`picture_mcp.py` configured as the MCP server `pictures` through the thread's `config` map, then one
`turn/start` per prompt, each waited out to its `turn/completed`. Every approval or elicitation
request the app-server makes is answered with an acceptance.

`<case>` names the run's directory. A case `--list` prints also brings its own prompt, the fixtures
to copy into the working directory and the script below, so `codex_capture.py mcp-picture-png
--scratch … --fixtures …` is a whole run; any other name needs a prompt.

Who plays the model:

- By default the signed-in account does: the run uses the Codex home and sign-in of whoever runs
  it, each turn spends that account, and each thread stays in that home's history.
- With `--scripted`, or one or more `--item <json>`, a scripted stand-in does, and no account is
  touched. The driver serves a loopback Responses API, points `codex app-server` at it with
  launch-time `-c` overrides (`model_provider`, `model_providers.scripted.*`) and gives it a Codex
  home of its own, `<scratch>/home`. The stand-in answers one output item to each request the
  app-server makes of the model, in order, then the message `done` to every request after.
  `--scripted` takes the items from the named case; each `--item` is one item written out, in
  which `arguments` may be an object, a string `@fixture:<name>` is replaced by that fixture's
  bytes in base64, and `{fixtures}` and `{work}` by those directories. For example:

      --item '{"type":"custom_tool_call","name":"exec","input":"image(await tools.view_image({path: \"small.png\"}));"}'

  With `--image-service <fixture>` the stand-in also plays the service Codex's image generation
  tool asks for a picture (`POST …/images/generations`), answering that fixture as `b64_json`, and
  the app-server is told the scripted provider signs in as OpenAI's does
  (`requires_openai_auth = true`); the Codex home is then `<scratch>/home-image-service`, holding
  an `auth.json` with a made-up API key that only the loopback stand-in is ever sent. On Codex
  0.167.0 this was not enough to have the tool offered: it takes a ChatGPT sign-in, so the
  `image-generation` case shows a picture only when the signed-in account plays the model.

  The models this was written against take every tool through one, `exec`, which runs JavaScript
  (`tool_mode: code_mode_only` in Codex's model catalog), hence the shape of the scripts. Everything
  the app-server does with an item is its own doing: the tool call, the notifications, the thread
  it stores. Only the model's choice of call is scripted.

Everything is written under `<scratch>/<case>/`, never into the repo:

- `received.jsonl`: every line the app-server wrote, raw, in order. Lines can be megabytes of
  base64; read them through `elide.py`, which is also how a capture becomes committable:

      python3 elide.py < <scratch>/<case>/received.jsonl > ../codex/<case>.jsonl

- `sent.jsonl`: every line this driver wrote to the app-server.
- `model-requests.jsonl`: under a stand-in, the body of every request the app-server made of the
  model, which is where a picture handed to the model shows. Read it through `elide.py` too.
  `image-requests.jsonl` is the same for the image service.
- `stderr.log`: the app-server's stderr. `mcp.log`: what `picture_mcp.py` was asked.
- `work/`: the thread's working directory; `--copy <name>` copies a fixture into it first.

Options:

- `--model` (default `gpt-5.6-luna`) and `--effort` (default `low`) go on each `turn/start`.
- `--resume <thread-id>`: `thread/resume` in place of `thread/start`. With no prompt it runs no
  turn: it only reads the thread back. `--cwd <dir>` reuses the first run's working directory.
- `--read <thread-id>`: also send `thread/read` with `includeTurns: true`, for comparison.
- `--request <method>=<json params>` (repeatable): one more request, sent once the thread is
  started or resumed, in which `{thread}` stands for the thread's id.
- `--raw-events`: `initialize` with `experimentalApi: true` and start the thread with
  `experimentalRawEvents: true`, which Shaman does not do, to see `rawResponseItem/completed`.
- `--no-mcp`: leave the `pictures` server out of the thread's config.
- `--codex-home <dir>`: the `CODEX_HOME` the app-server runs under.
- `-c key=value` (repeatable): passed to `codex app-server` as a launch-time override.
- `--list`: print the named cases and stop.

The summary printed at the end counts the notifications by method and lists every line over 64 KiB.
"""
import argparse, base64, collections, http.server, json, os, queue, shutil, subprocess, sys, threading, time
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("case", nargs="?")
    parser.add_argument("prompts", nargs="*")
    parser.add_argument("--scratch")
    parser.add_argument("--fixtures")
    parser.add_argument("--model", default="gpt-5.6-luna")
    parser.add_argument("--effort", default="low")
    parser.add_argument("--resume")
    parser.add_argument("--read")
    parser.add_argument("--request", dest="requests", action="append", default=[])
    parser.add_argument("--cwd")
    parser.add_argument("--copy", action="append", default=[])
    parser.add_argument("--item", dest="items", action="append", default=[])
    parser.add_argument("--scripted", action="store_true")
    parser.add_argument("--image-service")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--raw-events", action="store_true")
    parser.add_argument("--no-mcp", action="store_true")
    parser.add_argument("--codex-home")
    parser.add_argument("-c", dest="overrides", action="append", default=[])
    parser.add_argument("--codex", default="codex")
    parser.add_argument("--turn-timeout", type=float, default=420.0)
    args = parser.parse_intermixed_args()
    if args.list:
        for name, case in CASES.items():
            print(f"{name}: {case['prompt']}")
        return
    if not (args.case and args.scratch and args.fixtures):
        parser.error("a case, --scratch and --fixtures are needed")

    scratch = Path(args.scratch).resolve()
    out = scratch / args.case
    out.mkdir(parents=True, exist_ok=True)
    work = Path(args.cwd).resolve() if args.cwd else out / "work"
    work.mkdir(parents=True, exist_ok=True)
    fixtures = Path(args.fixtures).resolve()
    places = {"{fixtures}": str(fixtures), "{work}": str(work)}
    case = CASES.get(args.case, {"prompt": None, "items": [], "copy": []})
    prompts = args.prompts or ([place(case["prompt"], places)] if case["prompt"] and not args.resume else [])
    items = [json.loads(item) for item in args.items] or (case["items"] if args.scripted else [])
    generated = args.image_service or (case.get("image_service") if args.scripted else None)
    for name in args.copy or case["copy"]:
        shutil.copyfile(fixtures / name, work / name)

    overrides, home = list(args.overrides), args.codex_home
    if generated and args.codex_home:
        parser.error("--image-service writes a made-up auth.json, so it keeps to its own Codex home")
    if items or args.scripted:
        port = serve_scripted_model([fill(item, fixtures, places) for item in items], out,
                                    fixtures / generated if generated else None)
        home = home or str(scratch / ("home-image-service" if generated else "home"))
        if generated:
            overrides.append("model_providers.scripted.requires_openai_auth=true")
            Path(home).mkdir(parents=True, exist_ok=True)
            (Path(home) / "auth.json").write_text(json.dumps({"OPENAI_API_KEY": "sk-probe-807-not-a-key"}))
        overrides += ['model_provider="scripted"', 'model_providers.scripted.name="scripted"',
                      f'model_providers.scripted.base_url="http://127.0.0.1:{port}/v1"',
                      'model_providers.scripted.wire_api="responses"',
                      "model_providers.scripted.request_max_retries=0",
                      "model_providers.scripted.stream_max_retries=0"]
    environment = dict(os.environ)
    if home:
        Path(home).mkdir(parents=True, exist_ok=True)
        environment["CODEX_HOME"] = home

    command = [args.codex, "app-server"]
    for override in overrides:
        command += ["-c", override]
    server = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=open(out / "stderr.log", "wb"), cwd=str(work), env=environment)
    received = open(out / "received.jsonl", "wb")
    sent = open(out / "sent.jsonl", "w")
    lines = queue.Queue()

    def pump():
        for line in server.stdout:
            received.write(line)
            received.flush()
            lines.put(line)
        lines.put(None)

    threading.Thread(target=pump, daemon=True).start()
    counts, large, next_id = collections.Counter(), [], [0]

    def write(message):
        text = json.dumps(message)
        sent.write(text + "\n")
        sent.flush()
        server.stdin.write(text.encode() + b"\n")
        server.stdin.flush()

    def take(timeout):
        """The next message, with every request the app-server makes answered on the way."""
        line = lines.get(timeout=timeout)
        if line is None:
            raise SystemExit(f"codex app-server exited ({server.wait()}); see {out / 'stderr.log'}")
        message = json.loads(line)
        label = message.get("method") or ("error" if "error" in message else "response")
        counts[label] += 1
        if len(line) > 64 * 1024:
            large.append((label, len(line) - 1))
        if "method" in message and "id" in message:
            write({"id": message["id"], **accept(message["method"], message.get("params") or {})})
        return message

    def request(method, params, timeout=180.0):
        next_id[0] += 1
        mine = next_id[0]
        write({"id": mine, "method": method, "params": params})
        while True:
            message = take(timeout)
            if message.get("id") == mine and "method" not in message:
                if "error" in message:
                    print(f"{method} failed: {json.dumps(message['error'])[:600]}")
                    return None
                return message["result"]

    request("initialize", {"clientInfo": {"name": "shaman", "title": "Shaman", "version": "0.0.0-probe-807"},
                           "capabilities": {"experimentalApi": args.raw_events}})
    write({"method": "initialized"})

    config = {"model_reasoning_effort": args.effort}
    if not args.no_mcp:
        config["mcp_servers.pictures"] = {
            "command": sys.executable, "args": [str(HERE / "picture_mcp.py"), str(fixtures)],
            "env": {"PICTURE_MCP_LOG": str(out / "mcp.log")},
            "tool_timeout_sec": 120.0, "default_tools_approval_mode": "approve"}
    thread = {"cwd": str(work), "approvalPolicy": "on-request", "sandbox": "workspace-write", "config": config}
    if args.resume:
        result = request("thread/resume", {"threadId": args.resume, **thread})
    else:
        if args.raw_events:
            thread["experimentalRawEvents"] = True
        result = request("thread/start", {**thread, "ephemeral": False})
    if result is None:
        raise SystemExit(1)
    thread_id = result["thread"]["id"]
    print(f"thread {thread_id} model {result.get('model')} cwd {work}")
    if args.read:
        request("thread/read", {"threadId": args.read, "includeTurns": True})
    for extra in args.requests:
        method, _, params = extra.partition("=")
        request(method, json.loads(params.replace("{thread}", thread_id)))

    for prompt in prompts:
        started = request("turn/start", {
            "threadId": thread_id, "input": [{"type": "text", "text": prompt}], "model": args.model,
            "approvalPolicy": "on-request", "summary": "auto", "sandboxPolicy": {"type": "workspaceWrite"},
            "effort": args.effort})
        if started is None:
            break
        deadline = time.monotonic() + args.turn_timeout
        while True:
            message = take(max(1.0, deadline - time.monotonic()))
            params = message.get("params") or {}
            if message.get("method") == "turn/completed" and params.get("threadId") == thread_id:
                turn = params.get("turn") or {}
                print(f"turn {turn.get('id')} {turn.get('status')} {json.dumps(turn.get('error'))[:400]}")
                break

    # Whatever trails the last answer or turn, then a clean close.
    try:
        while True:
            take(1.5)
    except (queue.Empty, SystemExit):
        pass
    server.stdin.close()
    try:
        server.wait(timeout=20)
    except subprocess.TimeoutExpired:
        server.kill()
    for label, count in sorted(counts.items()):
        print(f"{count:5d}  {label}")
    for label, size in large:
        print(f"large: {size} bytes  {label}")


def accept(method, params):
    """The acceptance for each request an app-server makes of its client."""
    if method in ("item/commandExecution/requestApproval", "item/fileChange/requestApproval"):
        return {"result": {"decision": "accept"}}
    if method == "item/permissions/requestApproval":
        return {"result": {"permissions": params.get("permissions") or {}, "scope": "turn"}}
    if method == "mcpServer/elicitation/request":
        return {"result": {"action": "accept", "content": {}}}
    if method == "item/tool/requestUserInput":
        return {"result": {"answers": {}}}
    return {"error": {"code": -32601, "message": f"the probe does not answer {method}"}}


def place(text, places):
    for name, path in places.items():
        text = text.replace(name, path)
    return text


def fill(value, fixtures, places):
    """An item as the stand-in sends it: fixtures read in, `arguments` as the string the API carries."""
    if isinstance(value, dict):
        filled = {key: fill(item, fixtures, places) for key, item in value.items()}
        if not isinstance(filled.get("arguments", ""), str):
            filled["arguments"] = json.dumps(filled["arguments"])
        return filled
    if isinstance(value, list):
        return [fill(item, fixtures, places) for item in value]
    if isinstance(value, str) and value.startswith("@fixture:"):
        return base64.b64encode((fixtures / value[len("@fixture:"):]).read_bytes()).decode()
    return place(value, places) if isinstance(value, str) else value


def script(source):
    """The one item a model in code mode answers with: JavaScript handed to its `exec` tool."""
    return [{"type": "custom_tool_call", "name": "exec", "input": source}]


def mcp_case(tool, arguments, ask):
    """Calls one tool of the `pictures` server and hands the model every picture it answered with."""
    return {"prompt": f"{ask} Show yourself each picture it returns, then reply with the one word done.", "copy": [],
            "items": script(
                f"const r = await tools.mcp__pictures__{tool}({json.dumps(arguments)});\n"
                "text(JSON.stringify(Object.keys(r)));\n"
                "for (const b of r.content) {\n"
                "  if (b.type !== \"image\") { text(JSON.stringify(b).slice(0, 300)); continue; }\n"
                "  try { image(b); } catch (e) { text(\"image() threw: \" + e); }\n"
                "}")}


def view_case(path, copy, detail=None):
    """Calls `view_image` and hands the model what it answered with."""
    arguments = {"path": path, **({"detail": detail} if detail else {})}
    return {"prompt": f"Look at the image file {path} with your view_image tool"
                      + (f" at detail {detail}" if detail else "") + ", then reply with the one word done.",
            "copy": copy,
            "items": script(
                "try {\n"
                f"  const r = await tools.view_image({json.dumps(arguments)});\n"
                "  text(JSON.stringify({keys: Object.keys(r), detail: r.detail, chars: r.image_url.length,"
                " head: r.image_url.slice(0, 48)}));\n"
                "  image(r);\n"
                "} catch (e) { text(\"view_image threw: \" + e); }")}


KINDS = ["png", "jpeg", "gif", "webp", "tiff", "bmp", "heic", "svg"]
CASES = {
    **{f"mcp-picture-{kind}": mcp_case("picture", {"kind": kind},
                                       f'Call the picture tool of the pictures MCP server with kind "{kind}".')
       for kind in KINDS},
    "mcp-two-pictures": mcp_case("two_pictures", {}, "Call the two_pictures tool of the pictures MCP server."),
    **{f"mcp-big-picture-{mib}mib": mcp_case("big_picture", {"mib": mib},
                                             f"Call the big_picture tool of the pictures MCP server with mib {mib}.")
       for mib in (1, 3, 6)},
    "mcp-resource-picture": mcp_case("resource_picture", {}, "Call the resource_picture tool of the pictures MCP server."),
    "mcp-linked-picture": mcp_case("linked_picture", {}, "Call the linked_picture tool of the pictures MCP server."),
    **{f"view-image-{kind}": view_case(f"small.{kind}", [f"small.{kind}"]) for kind in KINDS},
    **{f"view-image-noise-{mib}mib": view_case(f"noise-{mib}mib.png", [f"noise-{mib}mib.png"]) for mib in (3, 6)},
    "view-image-noise-6mib-original": view_case("noise-6mib.png", ["noise-6mib.png"], "original"),
    "view-image-absolute": view_case("{fixtures}/small.png", []),
    "view-image-missing": view_case("no-such-file.png", []),
    # Under the stand-in the picture is a fixture the stand-in's image service answers with, so what
    # the real service answers (its kind, its size) is not what this case shows.
    "image-generation": {
        "prompt": "Generate a simple picture of a red circle on a white background, then reply with the one word done.",
        "copy": [], "image_service": "small.png",
        "items": script(
            "// @exec: {\"yield_time_ms\": 120000}\n"
            "const r = await tools.image_gen__imagegen({prompt: \"A red circle on a white background.\"});\n"
            "text(JSON.stringify(Object.fromEntries(Object.entries(r).map(([k, v]) =>"
            " [k, typeof v === \"string\" && v.length > 200 ? v.slice(0, 40) + \" (\" + v.length + \" chars)\" : v]))));\n"
            "generatedImage(r);")},
    # The Responses API's own `image_generation_call` output item, as Codex's tests write one, with a
    # fixture as its result: the way a picture was generated before the tool above.
    "image-generation-call-item": {
        "prompt": "Generate a simple picture of a red circle on a white background, then reply with the one word done.",
        "copy": [],
        "items": [{"type": "image_generation_call", "id": "ig_probe_1", "status": "completed",
                   "revised_prompt": "A red circle on a white background.", "result": "@fixture:small.png"}]},
}


def serve_scripted_model(items, out, generated):
    """A loopback Responses API that answers `items` one to a request, then `done`; returns its port.

    With `generated`, a picture's path, it is the image service too, answering that picture."""
    remaining, lock, log = collections.deque(items), threading.Lock(), open(out / "model-requests.jsonl", "wb")

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_):
            pass

        def do_GET(self):
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
            if generated and self.path.split("?")[0].endswith(("/images/generations", "/images/edits")):
                with open(out / "image-requests.jsonl", "ab") as requests:
                    requests.write(json.dumps({"path": self.path, "body": json.loads(body)}).encode() + b"\n")
                payload = json.dumps({"created": int(time.time()), "data": [
                    {"b64_json": base64.b64encode(generated.read_bytes()).decode()}]}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                return self.wfile.write(payload)
            if not self.path.split("?")[0].endswith("/responses"):
                return self.do_GET()
            with lock:
                number = getattr(Handler, "number", 0) + 1
                Handler.number = number
                log.write(body.replace(b"\n", b" ") + b"\n")
                log.flush()
                item = remaining.popleft() if remaining else {
                    "type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "done"}]}
            item = dict(item)
            if item.get("type") == "function_call" or item.get("type") == "custom_tool_call":
                item.setdefault("call_id", f"call_probe_{number}")
            item.setdefault("id", f"item_probe_{number}")
            usage = {"input_tokens": 0, "input_tokens_details": None, "output_tokens": 0,
                     "output_tokens_details": None, "total_tokens": 0}
            events = [{"type": "response.created", "response": {"id": f"resp_probe_{number}"}},
                      {"type": "response.output_item.done", "item": item},
                      {"type": "response.completed", "response": {"id": f"resp_probe_{number}", "usage": usage}}]
            payload = "".join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n" for event in events).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server.server_address[1]


if __name__ == "__main__":
    main()
