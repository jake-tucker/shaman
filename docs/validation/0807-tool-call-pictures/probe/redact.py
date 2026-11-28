#!/usr/bin/env python3
"""Takes what names this machine and its account out of an elided capture, for the 0807 probe.

    python3 elide.py < raw.jsonl | python3 redact.py > capture.jsonl

A capture made on the signed-in account runs under the Codex home of whoever ran it, so it names
their home directory, login, host, installation and plan. Each is replaced by `<redacted>` (the
home directory by `/Users/<redacted>`, so a path still reads as one). Nothing is read from the
capture to decide what to replace: the home directory, the login and the host are this machine's
own, and the rest are values of the keys named below.
"""
import getpass, json, re, socket, sys
from pathlib import Path

KEYS = {"serverName", "installationId", "planType", "timezone", "timeZone", "email", "accountId", "userId"}
HOME = str(Path.home())
NAMES = [name for name in (getpass.getuser(), socket.gethostname().split(".")[0]) if name]


def redact(value, key=None):
    if isinstance(value, dict):
        return {name: redact(item, name) for name, item in value.items()}
    if isinstance(value, list):
        return [redact(item) for item in value]
    if isinstance(value, str):
        if key in KEYS:
            return "<redacted>"
        value = value.replace(HOME, "/Users/<redacted>")
        for name in NAMES:
            value = re.sub(rf"\b{re.escape(name)}\b", "<redacted>", value, flags=re.I)
        return value
    return value


for line in sys.stdin:
    if line.strip():
        print(json.dumps(redact(json.loads(line))))
