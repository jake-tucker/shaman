#!/usr/bin/env python3
"""Has a live Provider's agent come to show a picture, and keeps each Message it wrote, for the 0822 probe.

    python3 probe.py --scratch <dir> --provider claude|codex|copilot --model <model> [options]

One run is one case in one fresh session: the project `make_project.py` wrote is copied into a
directory of the run's own under `<scratch>/runs/`, made a git repository, and the agent is sent the
case's one prompt there. Each Provider is driven the way Shaman's adapter drives it:

- **claude**: `claude --print` with stream-json both ways and the flags `src/provider/claude` gives
  it, every `can_use_tool` request allowed.
- **codex**: `codex app-server`, with `initialize`, `thread/start` and `turn/start` as
  `src/provider/codex` sends them, in the `workspace-write` sandbox, every approval accepted.
- **copilot**: the Copilot CLI through the SDK, by the `copilot_message_picture_capture` example,
  whose built binary `--copilot-capture` names. Every permission request is approved.

`<scratch>` must already hold `template/` and `shared/` from `make_project.py` and `venv/`, whose
`bin` leads the PATH the agent runs under. It should stand under the home directory, as a real
Workspace does, so that an agent has the chance to write `~`.

Everything raw is written under `<scratch>/runs/<provider>/<model>/<case>/` and never into the repo.
What is kept is one JSON file a run, under `../captures/<provider>/<model>/`:

- `prompt`, `cwd` (the Session's Execution Directory) and `out` (the run's own directory outside it);
- `messages`: every Message the agent wrote, in order, its Markdown byte for byte as `text`;
- `tools`: what it ran, shortened, so that it can be seen how a picture came to be;
- `pictures`: every image file under `cwd` and `out` once the Turn settled, and whether it is new,
  with any other the agent named by an absolute or `~` path (`elsewhere`), as `/tmp/revenue.png`.

The home directory is written as `/Users/reader` throughout a kept capture, the one change made to
a Message's bytes: an absolute path stays absolute and `~` stays what the agent wrote.

Options: `--cases a,b` runs only those (default all, see CASES); `--tag r2` names a repeat, so
that `make-show-in` is kept beside `make-show-in.r2`; `--jobs` runs that many at once (default 4);
`--effort` is Codex's reasoning effort (default `low`); `--timeout` is seconds a run (default 600);
`--note` appends a sentence to Claude's system prompt, as Shaman's Broker note is appended, and
keeps the run under `<model>+note`.
"""
import argparse, concurrent.futures, json, os, queue, re, shutil, subprocess, sys, tempfile, threading, time, uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
HOME = str(Path.home())
STAND_IN = "/Users/reader"

BAR = "Plot monthly revenue from data/sales.csv as a bar chart"
SCRIPT = "Add scripts/plot_revenue.py, which charts monthly revenue from data/sales.csv to a PNG"
# name: (prompt, the working directory's name). {out} is the run's own directory outside the
# project, and {shared} the shared directory as its user would type it, from `~`.
CASES = {
    "show-in": ("Show me docs/architecture.png", "acme-dashboard"),
    "show-out": ("Show me {out}/screens/dashboard.png", "acme-dashboard"),
    "show-home": ("Show me {shared}/team-photo.jpg", "acme-dashboard"),
    "show-space": ("Show me the login flow picture from the design notes", "acme-dashboard"),
    "show-find": ("There's an architecture diagram somewhere in this repo. Can you show it to me?", "acme-dashboard"),
    "make-show-in": (BAR + " and show it to me.", "acme-dashboard"),
    "make-show-out": (BAR + ", save it in {out}/exports, and show it to me.", "acme-dashboard"),
    "make-show-space": (BAR + ", put it in the design notes folder, and show it to me.", "acme-dashboard"),
    "diagram-show": ("Draw a diagram of how the modules under src/ depend on each other, as a PNG, and show it to me.",
                     "acme-dashboard"),
    "make-where-in": (BAR + " and tell me where you saved it.", "acme-dashboard"),
    "make-where-out": (BAR + ". Keep it out of the repo, and tell me where you saved it.", "acme-dashboard"),
    "make-where-space": (BAR + ", put it in the design notes folder, and tell me where it is.", "acme-dashboard"),
    "unasked-in": (SCRIPT + ", and run it to check it works.", "acme-dashboard"),
    "unasked-out": (SCRIPT + " under {out}/exports, and run it to check it works.", "acme-dashboard"),
    "unasked-space": (SCRIPT + ", and run it to check it works.", "acme dashboard"),
    "unasked-fix": ("The README shows a revenue chart, but the picture it points at is missing. Fix that.",
                    "acme-dashboard"),
}

MAGIC = ((b"\x89PNG\r\n\x1a\n", "png"), (b"\xff\xd8\xff", "jpeg"), (b"GIF8", "gif"))


def kind_of(path):
    try:
        with open(path, "rb") as source:
            head = source.read(16)
    except OSError:
        return None
    for magic, kind in MAGIC:
        if head.startswith(magic):
            return kind
    if head[:4] == b"RIFF" and head[8:12] == b"WEBP":
        return "webp"
    return "svg" if path.suffix.lower() == ".svg" else None


def pictures_under(*roots):
    found = {}
    for root in roots:
        for directory, names, files in os.walk(root):
            names[:] = [name for name in names if name not in (".git", "__pycache__", "node_modules", ".venv")]
            for name in files:
                path = Path(directory) / name
                kind = kind_of(path)
                if kind:
                    found[str(path)] = {"path": str(path), "kind": kind, "bytes": path.stat().st_size}
    return found


def short(value, limit=400):
    text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)
    return text if len(text) <= limit else text[:limit] + "…"


def environment(scratch):
    env = {key: value for key, value in os.environ.items() if not key.startswith("CLAUDE")}
    env["PATH"] = f"{scratch / 'venv' / 'bin'}:{env['PATH']}"
    env["MPLBACKEND"] = "Agg"
    env["MPLCONFIGDIR"] = str(scratch / "mplconfig")
    env.pop("VIRTUAL_ENV", None)
    return env


def run_claude(args, cwd, prompt, raw, env):
    """One CLI process, one stream-json user message, read to its `result`."""
    command = ["claude", "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
               "--setting-sources", "user,project", "--include-partial-messages",
               "--permission-mode", "default", "--permission-prompt-tool", "stdio",
               "--session-id", str(uuid.uuid4()), "--model", args.model]
    if args.note:
        command += ["--append-system-prompt", args.note]
    env = dict(env, CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR="1")
    messages, tools, seen, facts = [], [], set(), {}
    with open(raw / "raw.jsonl", "wb") as log, open(raw / "stderr.log", "wb") as stderr:
        child = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr)
        watchdog = threading.Timer(args.timeout, child.kill)
        watchdog.daemon = True
        watchdog.start()

        def send(value):
            child.stdin.write(json.dumps(value).encode() + b"\n")
            child.stdin.flush()

        send({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": prompt}]},
              "parent_tool_use_id": None, "session_id": "", "uuid": str(uuid.uuid4())})
        for line in child.stdout:
            log.write(line)
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            kind = event.get("type")
            if kind == "system" and event.get("subtype") == "init":
                facts.update(cli=event.get("claude_code_version"), model=event.get("model"))
            elif kind == "control_request" and event.get("request", {}).get("subtype") == "can_use_tool":
                send({"type": "control_response", "response": {
                    "subtype": "success", "request_id": event["request_id"],
                    "response": {"behavior": "allow", "updatedInput": event["request"].get("input", {})}}})
            elif kind == "assistant":
                message = event.get("message") or {}
                for index, block in enumerate(message.get("content") or []):
                    # One API message arrives as an event a block; a block is taken once.
                    key = (message.get("id"), block.get("type"), block.get("id") or block.get("text"))
                    if key in seen:
                        continue
                    seen.add(key)
                    if block.get("type") == "text" and block.get("text"):
                        messages.append({"text": block["text"], "subagent": event.get("parent_tool_use_id") is not None})
                    elif block.get("type") == "tool_use":
                        tools.append(f"{block.get('name')}: {short(block.get('input'))}")
            elif kind == "result":
                facts.update(result=event.get("result"), stop=event.get("subtype"))
                child.stdin.close()
        facts["exit"] = child.wait()
        watchdog.cancel()
    return messages, tools, facts


def run_codex(args, cwd, prompt, raw, env):
    """One `codex app-server`, one thread, one turn, read to its `turn/completed`."""
    server = subprocess.Popen(["codex", "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=open(raw / "stderr.log", "wb"), cwd=cwd, env=env)
    log, lines, next_id = open(raw / "raw.jsonl", "wb"), queue.Queue(), [0]
    messages, tools, facts = [], [], {}

    def pump():
        for line in server.stdout:
            log.write(line)
            log.flush()
            lines.put(line)
        lines.put(None)

    threading.Thread(target=pump, daemon=True).start()

    def write(message):
        server.stdin.write(json.dumps(message).encode() + b"\n")
        server.stdin.flush()

    def accept(method, params):
        if method in ("item/commandExecution/requestApproval", "item/fileChange/requestApproval"):
            return {"result": {"decision": "accept"}}
        if method == "item/permissions/requestApproval":
            return {"result": {"permissions": params.get("permissions") or {}, "scope": "turn"}}
        if method == "mcpServer/elicitation/request":
            return {"result": {"action": "accept", "content": {}}}
        if method == "item/tool/requestUserInput":
            return {"result": {"answers": {}}}
        return {"error": {"code": -32601, "message": f"the probe does not answer {method}"}}

    def take(timeout):
        line = lines.get(timeout=timeout)
        if line is None:
            raise RuntimeError(f"codex app-server exited ({server.wait()})")
        message = json.loads(line)
        if "method" in message and "id" in message:
            tools.append(f"(approval asked) {message['method']}")
            write({"id": message["id"], **accept(message["method"], message.get("params") or {})})
        item = (message.get("params") or {}).get("item") or {}
        if message.get("method") == "item/completed":
            kind = item.get("type")
            if kind == "agentMessage":
                if item.get("text"):
                    messages.append({"text": item["text"], **({"phase": item["phase"]} if item.get("phase") else {})})
            elif kind == "commandExecution":
                tools.append(f"commandExecution: {short(item.get('command'))}")
            elif kind == "fileChange":
                tools.append(f"fileChange: {short([change.get('path') for change in item.get('changes') or []])}")
            elif kind in ("imageView", "imageGeneration"):
                tools.append(f"{kind}: {short(item.get('path') or item.get('savedPath'))}")
            elif kind not in ("userMessage", "reasoning"):
                tools.append(f"{kind}: {short({key: value for key, value in item.items() if key in ('tool', 'server', 'arguments', 'query')})}")
        return message

    def request(method, params, timeout=180.0):
        next_id[0] += 1
        mine = next_id[0]
        write({"id": mine, "method": method, "params": params})
        while True:
            message = take(timeout)
            if message.get("id") == mine and "method" not in message:
                if "error" in message:
                    raise RuntimeError(f"{method} failed: {short(message['error'])}")
                return message["result"]

    try:
        hello = request("initialize", {"clientInfo": {"name": "shaman", "title": "Shaman", "version": "0.0.0-probe-822"},
                                       "capabilities": {"experimentalApi": False}})
        facts["cli"] = hello.get("userAgent")
        write({"method": "initialized"})
        thread = request("thread/start", {"cwd": cwd, "approvalPolicy": "on-request", "sandbox": "workspace-write",
                                          "config": {"model_reasoning_effort": args.effort}, "ephemeral": False})
        thread_id = thread["thread"]["id"]
        request("turn/start", {"threadId": thread_id, "input": [{"type": "text", "text": prompt}],
                               "model": args.model, "approvalPolicy": "on-request", "summary": "auto",
                               "sandboxPolicy": {"type": "workspaceWrite"}, "effort": args.effort})
        facts.update(model=args.model, effort=args.effort)
        deadline = time.monotonic() + args.timeout
        while True:
            message = take(max(1.0, deadline - time.monotonic()))
            params = message.get("params") or {}
            if message.get("method") == "turn/completed" and params.get("threadId") == thread_id:
                turn = params.get("turn") or {}
                facts.update(stop=turn.get("status"), error=turn.get("error"))
                break
    except (queue.Empty, RuntimeError) as failure:
        facts["failed"] = str(failure) or "timed out"
    server.stdin.close()
    try:
        facts["exit"] = server.wait(timeout=20)
    except subprocess.TimeoutExpired:
        server.kill()
    return messages, tools, facts


def run_copilot(args, cwd, prompt, raw, env):
    """The example binary drives the CLI through the SDK and prints one JSON line an event."""
    env = dict(env, CAPTURE_MODEL=args.model, CAPTURE_CWD=cwd, CAPTURE_PROMPT=prompt)
    messages, tools, facts = [], [], {}
    with open(raw / "raw.jsonl", "wb") as log, open(raw / "stderr.log", "wb") as stderr:
        try:
            done = subprocess.run([args.copilot_capture], cwd=cwd, env=env, stdout=log, stderr=stderr, timeout=args.timeout)
            facts["exit"] = done.returncode
        except subprocess.TimeoutExpired:
            facts["failed"] = "timed out"
    for line in open(raw / "raw.jsonl", encoding="utf-8"):
        event = json.loads(line)
        data = event.get("data") or {}
        if event.get("_probe") == "capture":
            facts.update(cli=event.get("cli"), model=event.get("model"))
        elif event.get("type") == "assistant.message":
            if data.get("content"):
                messages.append({"text": data["content"], "subagent": bool(data.get("parentToolCallId"))})
        elif event.get("type") == "tool.execution_start":
            tools.append(f"{data.get('toolName')}: {short(data.get('arguments'))}")
        elif event.get("type") == "session.error":
            facts["error"] = short(data)
    return messages, tools, facts


RUNNERS = {"claude": run_claude, "codex": run_codex, "copilot": run_copilot}


def scrub(value):
    if isinstance(value, dict):
        return {key: scrub(item) for key, item in value.items()}
    if isinstance(value, list):
        return [scrub(item) for item in value]
    return value.replace(HOME, STAND_IN) if isinstance(value, str) else value


def one(args, case):
    scratch = args.scratch
    name = case + (f".{args.tag}" if args.tag else "")
    model = args.model + ("+note" if args.note else "")
    raw = scratch / "runs" / args.provider / model / name
    if raw.exists():
        shutil.rmtree(raw)
    raw.mkdir(parents=True)
    text, directory = CASES[case]
    cwd = raw / directory
    shutil.copytree(scratch / "template", cwd)
    git = ["git", "-C", str(cwd), "-c", "user.name=probe", "-c", "user.email=probe@example.invalid"]
    subprocess.run(git + ["init", "-q"], check=True)
    subprocess.run(git + ["add", "-A"], check=True)
    subprocess.run(git + ["commit", "-qm", "Initial commit"], check=True)
    out = Path(tempfile.mkdtemp(prefix="p822."))
    (out / "screens").mkdir()
    (out / "exports").mkdir()
    shutil.copy(scratch / "shared" / "dashboard.png", out / "screens" / "dashboard.png")
    shared = "~/" + str((scratch / "shared").relative_to(HOME))
    prompt = text.replace("{out}", str(out)).replace("{shared}", shared)
    before = pictures_under(cwd, out)
    started, wall = time.monotonic(), time.time() - 2
    messages, tools, facts = RUNNERS[args.provider](args, str(cwd), prompt, raw, environment(scratch))
    after = pictures_under(cwd, out)
    # A picture the agent put somewhere of its own choosing is found by its name in what was said
    # or run: an absolute or `~` path with no space in it that ends as a raster picture's does.
    said = "\n".join([message["text"] for message in messages] + tools)
    for named in set(re.findall(r"(?<![\w.])((?:~|/)[^\s\"'`<>()\[\]\\]*\.(?:png|jpe?g|gif|webp))", said)):
        path = Path(os.path.realpath(os.path.expanduser(named)))
        known = {os.path.realpath(listed) for listed in after} | {str(scratch / "shared" / "team-photo.jpg")}
        if str(path) not in known and kind_of(path):
            after[str(path)] = {"path": str(path), "kind": kind_of(path), "bytes": path.stat().st_size, "elsewhere": True}
            if path.stat().st_mtime < wall:
                before[str(path)] = after[str(path)]
    if args.note:
        facts["note"] = args.note
    capture = {"provider": args.provider, "case": name, **facts, "prompt": prompt, "cwd": str(cwd), "out": str(out),
               "seconds": round(time.monotonic() - started, 1), "messages": messages, "tools": tools,
               "pictures": [dict(entry, new=path not in before or before[path]["bytes"] != entry["bytes"])
                            for path, entry in sorted(after.items())]}
    (raw / "capture.raw.json").write_text(json.dumps(capture, ensure_ascii=False, indent=1) + "\n")
    kept = HERE.parent / "captures" / args.provider / model / f"{name}.json"
    kept.parent.mkdir(parents=True, exist_ok=True)
    kept.write_text(json.dumps(scrub(capture), ensure_ascii=False, indent=1) + "\n")
    return f"{args.provider}/{model}/{name}: {len(messages)} messages, {len(tools)} tools, " \
           f"{sum(entry['new'] for entry in capture['pictures'])} new pictures, {capture['seconds']}s " \
           f"{facts.get('failed') or facts.get('error') or ''}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--scratch", required=True, type=Path)
    parser.add_argument("--provider", required=True, choices=sorted(RUNNERS))
    parser.add_argument("--model", required=True)
    parser.add_argument("--effort", default="low")
    parser.add_argument("--cases", default=",".join(CASES))
    parser.add_argument("--tag", default="")
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--timeout", type=float, default=600.0)
    parser.add_argument("--note", help="claude only: a sentence appended to the system prompt, kept as `<model>+note`")
    parser.add_argument("--copilot-capture", help="the built copilot_message_picture_capture example")
    args = parser.parse_args()
    args.scratch = args.scratch.resolve()
    cases = [case for case in args.cases.split(",") if case]
    for case in cases:
        if case not in CASES:
            parser.error(f"no case {case}")
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for line in pool.map(lambda case: one(args, case), cases):
            print(line, flush=True)


if __name__ == "__main__":
    main()
