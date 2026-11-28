#!/usr/bin/env python3
"""Drives a live Claude Code CLI the way Shaman's adapter does and logs what it says, for the 0807 probe.

    python3 claude_capture.py --scratch <dir> --fixtures <dir> <case> <prompt> [<prompt> ...]
    python3 claude_capture.py --scratch <dir> --fixtures <dir> --resume <session-id> <case> <prompt>

One CLI process is launched with the flags `src/provider/claude/transport.rs` and `session.rs` give
it (stream-json both ways, partial messages, the permission prompt on stdio, `--session-id` to mint
or `--resume` to continue), plus `picture_mcp.py` as its only MCP server under the name `pictures`.
Each prompt is one turn: it is sent as a stream-json user message, and the next is sent once the
CLI's `result` arrives. `{fixtures}` in a prompt is replaced by the fixtures directory, and
`{extra}` by the directory `--extra-fixtures` names (what `claude_extra_fixtures.py` wrote).

Every stdout line is written raw to `<scratch>/<case>.raw.jsonl` (never commit that: it holds the
pictures), what was sent to `<scratch>/<case>.stdin.jsonl`, stderr to `<scratch>/<case>.stderr`,
and the session id to `<scratch>/<case>.session`. The elided capture goes to
`../claude/<case>.jsonl` beside this script, through `elide.py` and then `scrub` below, which takes
out what describes the machine and account rather than the picture: the home directory becomes `~`,
and the `init` event's lists of skills, slash commands, agents and plugins, the rate limit report
and the credential's expiry become stubs. `_raw_bytes` stays the size of the line as it
arrived. `--only-elide` writes the elided capture again from the raw one without running the CLI.
A `can_use_tool` request, should one arrive despite `--allowedTools`, is allowed and shows in the
capture.

The prompts the captures in `../claude/` were made with each asked for one call, for example:

    'Call the Read tool once on {fixtures}/small.png and then reply with only the word done.'
    'Call the picture tool of the pictures server once with kind "png" and then reply with only the word done.'

A case named for several files (`read-jpeg-gif-webp`) is one process given one such prompt a file.

Needs `claude` on PATH and signed in, and Python 3.9 or later. It spends the account's tokens:
`--model` defaults to Haiku. The CLI's own settings are read (`--setting-sources user,project`, as
Shaman's sessions launch) but nothing of the user's configuration is written.
"""
import argparse, json, os, subprocess, sys, threading, time, uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent

parser = argparse.ArgumentParser()
parser.add_argument("--scratch", required=True, type=Path)
parser.add_argument("--fixtures", required=True, type=Path)
parser.add_argument("--extra-fixtures", type=Path, help="the directory {extra} stands for in a prompt")
parser.add_argument("--resume", help="continue this session rather than minting one")
parser.add_argument("--model", default="haiku")
parser.add_argument("--turn-timeout", type=float, default=240.0)
parser.add_argument("--no-partial", action="store_true", help="leave out --include-partial-messages")
parser.add_argument("--extra", action="append", default=[], help="one more CLI argument, repeatable")
parser.add_argument("--only-elide", action="store_true", help="elide <case>.raw.jsonl again; run nothing")
parser.add_argument("case")
parser.add_argument("prompts", nargs="*")
args = parser.parse_args()
if not args.only_elide and not args.prompts:
    parser.error("a prompt is needed")

HOME = str(Path.home())
INIT_LISTS = ("slash_commands", "terminal_slash_commands", "skills", "agents", "plugins")


def scrub(value):
    """Takes what names the machine's owner or their account out of one elided event."""
    if isinstance(value, dict):
        if value.get("type") == "system" and value.get("subtype") == "init":
            value = {key: ({"_scrubbed": "list", "entries": len(item)} if key in INIT_LISTS else item)
                     for key, item in value.items()}
        if value.get("type") == "rate_limit_event" and "rate_limit_info" in value:
            value = dict(value, rate_limit_info={"_scrubbed": "object"})
        if "credential_expires_at" in value:
            value = dict(value, credential_expires_at={"_scrubbed": "timestamp"})
        return {key: scrub(item) for key, item in value.items()}
    if isinstance(value, list):
        return [scrub(item) for item in value]
    if isinstance(value, str):
        return value.replace(HOME, "~")
    return value


def write_elided(raw_path):
    elided = HERE.parent / "claude" / f"{args.case}.jsonl"
    elided.parent.mkdir(exist_ok=True)
    with open(raw_path, "rb") as source:
        lines = subprocess.run([sys.executable, str(HERE / "elide.py")], stdin=source, stdout=subprocess.PIPE,
                               check=True).stdout.decode().splitlines()
    with open(elided, "w") as target:
        for line in lines:
            target.write(json.dumps(scrub(json.loads(line)), ensure_ascii=False) + "\n")
    return elided


if args.only_elide:
    print(write_elided(args.scratch.resolve() / f"{args.case}.raw.jsonl"))
    sys.exit(0)

scratch, fixtures = args.scratch.resolve(), args.fixtures.resolve()
work = scratch / "work"
work.mkdir(parents=True, exist_ok=True)
session = args.resume or str(uuid.uuid4())
(scratch / f"{args.case}.session").write_text(session + "\n")

mcp_config = scratch / f"{args.case}.mcp.json"
mcp_config.write_text(json.dumps({"mcpServers": {"pictures": {
    "command": sys.executable, "args": [str(HERE / "picture_mcp.py"), str(fixtures)],
    "env": {"PICTURE_MCP_LOG": str(scratch / f"{args.case}.mcp.log")}}}}))

command = ["claude", "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
           "--setting-sources", "user,project"]
if not args.no_partial:
    command.append("--include-partial-messages")
command += ["--permission-mode", "default", "--permission-prompt-tool", "stdio",
            "--resume" if args.resume else "--session-id", session,
            "--mcp-config", str(mcp_config), "--strict-mcp-config",
            "--allowedTools", "Read", "mcp__pictures",
            "--add-dir", str(fixtures),
            "--model", args.model] + args.extra
extra = args.extra_fixtures.resolve() if args.extra_fixtures else None
if extra:
    command += ["--add-dir", str(extra)]

# The probe may itself be run from inside a Claude Code session; the child must not inherit that.
env = {key: value for key, value in os.environ.items() if not key.startswith("CLAUDE")}
env["CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR"] = "1"

raw_path = scratch / f"{args.case}.raw.jsonl"
with open(raw_path, "wb") as raw, open(scratch / f"{args.case}.stdin.jsonl", "w") as sent, \
        open(scratch / f"{args.case}.stderr", "wb") as stderr:
    child = subprocess.Popen(command, cwd=work, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr)

    def send(value):
        line = json.dumps(value)
        sent.write(line + "\n")
        sent.flush()
        child.stdin.write(line.encode() + b"\n")
        child.stdin.flush()

    def prompt(text):
        send({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]},
              "parent_tool_use_id": None, "session_id": "", "uuid": str(uuid.uuid4())})

    pending = [text.replace("{fixtures}", str(fixtures)).replace("{extra}", str(extra)) for text in args.prompts]
    deadline = [time.monotonic() + args.turn_timeout]

    def watchdog():
        while child.poll() is None:
            if time.monotonic() > deadline[0]:
                print("turn timed out; killing the CLI", file=sys.stderr)
                child.kill()
                return
            time.sleep(0.5)

    threading.Thread(target=watchdog, daemon=True).start()
    prompt(pending.pop(0))
    counts = {}
    for line in child.stdout:
        raw.write(line)
        raw.flush()
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            counts["(not json)"] = counts.get("(not json)", 0) + 1
            continue
        kind = message.get("type")
        counts[kind] = counts.get(kind, 0) + 1
        if kind == "control_request" and message.get("request", {}).get("subtype") == "can_use_tool":
            send({"type": "control_response", "response": {"subtype": "success", "request_id": message["request_id"],
                  "response": {"behavior": "allow", "updatedInput": message["request"].get("input", {})}}})
        elif kind == "result":
            if pending:
                deadline[0] = time.monotonic() + args.turn_timeout
                prompt(pending.pop(0))
            else:
                child.stdin.close()
    code = child.wait()

elided = write_elided(raw_path)
print(json.dumps({"case": args.case, "session": session, "exit": code, "lines": counts,
                  "raw": str(raw_path), "elided": str(elided)}))
