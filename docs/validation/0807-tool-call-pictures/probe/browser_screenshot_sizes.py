#!/usr/bin/env python3
"""Asks a browser MCP server for screenshots and writes each answer as it arrived, for 0807.

    PLAYWRIGHT_BROWSERS_PATH=<scratch> python3 browser_screenshot_sizes.py > raw.jsonl
    python3 elide.py < raw.jsonl > browser-screenshots.jsonl

No Provider is involved: this speaks MCP over stdio to `npx @playwright/mcp` itself, so what is
measured is what a Provider would be handed as the tool's result. Each line written is one
`tools/call` response, with `_probe` naming the call. The browser is downloaded into
`PLAYWRIGHT_BROWSERS_PATH` beforehand, by
`npx -y @playwright/mcp@0.0.86 install-browser chrome-for-testing` run under the same variable.
"""
import json, subprocess, sys

SERVER = ["npx", "-y", "@playwright/mcp@0.0.86", "--headless", "--browser", "chromium"]
PAGES = [("text", "https://en.wikipedia.org/wiki/Rust_(programming_language)"),
         ("photos", "https://commons.wikimedia.org/wiki/Commons:Featured_pictures/Animals/Birds")]
SIZES = [(1280, 800), (1920, 1080), (2560, 1440)]

server = subprocess.Popen(SERVER, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
next_id = 0


def ask(method, params, probe=None):
    global next_id
    next_id += 1
    server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": next_id, "method": method, "params": params}) + "\n")
    server.stdin.flush()
    while True:
        line = server.stdout.readline()
        if not line:
            raise SystemExit("the server closed its output")
        message = json.loads(line)
        if message.get("id") == next_id:
            if probe:
                message["_probe"] = probe
                print(json.dumps(message), flush=True)
            return message


def call(name, arguments, probe=None):
    return ask("tools/call", {"name": name, "arguments": arguments}, probe)


ask("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "probe-0807", "version": "0"}})
server.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
server.stdin.flush()
tools = ask("tools/list", {})["result"]["tools"]
shot = next(tool for tool in tools if tool["name"] == "browser_take_screenshot")
print(json.dumps({"_probe": "the screenshot tool as listed", "tool": shot}), flush=True)
for label, url in PAGES:
    for width, height in SIZES:
        call("browser_resize", {"width": width, "height": height})
        call("browser_navigate", {"url": url})
        for kind in ("png", "jpeg"):
            for full in (False, True):
                call("browser_take_screenshot", {"type": kind, "fullPage": full},
                     {"page": label, "width": width, "height": height, "type": kind, "fullPage": full})
        call("browser_take_screenshot", {"type": "png", "scale": "device"},
             {"page": label, "width": width, "height": height, "type": "png", "fullPage": False, "scale": "device"})
server.stdin.close()
server.wait(timeout=30)
