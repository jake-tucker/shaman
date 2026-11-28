#!/usr/bin/env python3
"""Probe for #813, not for `main`: the Copilot CLI with its loop's first idle held back.

Stands where the CLI does on Shaman's stdio wire and relays both ways. What Shaman sends reaches
the CLI at once. What the CLI sends is relayed in order, but the first frame carrying an
`assistant.idle` waits `PROBE_HOLD_MS` first, and so does everything behind it: that event comes a
millisecond ahead of `session.idle`, and Shaman settles its Turn on whichever it reads first. That
is the order a steer sent between Copilot stopping and Shaman hearing of it meets, made wide enough
to hit.

`COPILOT_REAL` names the CLI (default `copilot`). `PROBE_FRAME_LOG`, when set, gets one line per
frame: milliseconds since launch, direction, and the frame's method and event type.
"""
import json
import os
import subprocess
import sys
import threading
import time

started = time.monotonic()
hold_ms = int(os.environ.get("PROBE_HOLD_MS", "0"))
log_path = os.environ.get("PROBE_FRAME_LOG")
log = open(log_path, "a", buffering=1) if log_path else None
log_lock = threading.Lock()


def note(direction, body, extra=""):
    if log is None:
        return
    try:
        frame = json.loads(body)
    except ValueError:
        frame = {}
    params = frame.get("params") or {}
    event = params.get("event") or {}
    data = event.get("data") or {}
    what = {
        "at_ms": int((time.monotonic() - started) * 1000),
        "dir": direction,
        "method": frame.get("method"),
        "id": frame.get("id"),
        "event": event.get("type"),
    }
    if frame.get("method") == "session.send":
        what["mode"] = params.get("mode")
        what["prompt"] = (params.get("prompt") or "")[:60]
    if event.get("type") == "user.message":
        what["delivery"] = data.get("delivery")
        what["content"] = (data.get("content") or "")[:60]
    if event.get("type") == "assistant.message":
        what["content"] = (data.get("content") or "")[:60]
    if event.get("type", "").startswith("permission."):
        what["data"] = json.dumps(data)[:200]
    if "error" in frame:
        what["error"] = frame["error"]
    if extra:
        what["probe"] = extra
    with log_lock:
        log.write(json.dumps(what) + "\n")


def frames(stream):
    while True:
        length = None
        while True:
            line = stream.readline()
            if not line:
                return
            if line in (b"\r\n", b"\n"):
                break
            name, _, value = line.partition(b":")
            if name.strip().lower() == b"content-length":
                length = int(value.strip())
        if length is None:
            continue
        body = stream.read(length)
        if len(body) < length:
            return
        yield body


def write(stream, body):
    stream.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    stream.flush()


def main():
    cli = subprocess.Popen(
        [os.environ.get("COPILOT_REAL", "copilot")] + sys.argv[1:],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    )

    def inbound():
        for body in frames(sys.stdin.buffer):
            note("to_cli", body)
            write(cli.stdin, body)
        cli.stdin.close()

    threading.Thread(target=inbound, daemon=True).start()
    held = False
    for body in frames(cli.stdout):
        if hold_ms > 0 and not held and b'"type":"assistant.idle"' in body:
            held = True
            note("from_cli", body, f"held {hold_ms} ms from here")
            time.sleep(hold_ms / 1000)
            note("from_cli", body, "let go")
        else:
            note("from_cli", body)
        write(sys.stdout.buffer, body)
    sys.exit(cli.wait())


if __name__ == "__main__":
    main()
