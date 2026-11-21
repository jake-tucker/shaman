# Shaman

<img width="3316" height="1906" alt="image" src="https://github.com/user-attachments/assets/0792f2d3-f954-4505-8a16-415637a9f8ef" />

Shaman is an agent orchestrator, drawing inspiration from tools like OpenCode and T3 Code.

It does not run LLMs by itself. Instead, it orchestrates other harnesses, such as Codex, Claude Code,
and Copilot, while providing a single consolidated view into their work. 

The goal is to provide a high-performance, configurable, and provider-agnostic agentic coding experience.

This repo is still very early in development - expect bugs and rough edges.

## Installation

Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/jake-tucker/shaman/main/scripts/install.sh | bash
```

Windows:

```powershell
irm https://raw.githubusercontent.com/jake-tucker/shaman/main/scripts/install.ps1 | iex
```

You can check an installed binary was built by this repository's release workflow with `gh attestation verify ~/.local/bin/shaman --repo jake-tucker/shaman`.

To uninstall, delete the binary: `~/.local/bin/shaman`, or `%LOCALAPPDATA%\Programs\shaman\shaman.exe` on Windows.

## Features

The core "type a prompt and stuff happens" flow works as you might expect, so here are some of the additional
cool things Shaman does:

- Cross-provider subagents - a Claude-managed Opus session can spin up a Codex-managed Astra session to review its work.
- Remote access - run `/serve` to issue an invite on one machine, then paste it into `/pair` on another. Machines must
  have a direct network connection via the local network, via a VPN such as Tailscale, or use a [relay server](docs/relay/README.md).
- `/sidekick` - an agent with a view over all your sessions across all of your machines. Use it to answer meta questions,
  or to orchestrate work across workspaces and remotes.
- `/metrics` - see how much you're spending over time, broken down by project or model.
