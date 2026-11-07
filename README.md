# Suru

<img width="3316" height="1906" alt="image" src="https://github.com/user-attachments/assets/0792f2d3-f954-4505-8a16-415637a9f8ef" />

Suru is an agent orchestrator, drawing inspiration from tools like OpenCode and T3 Code.

It does not run LLMs by itself. Instead, it orchestrates other harnesses, such as Codex, Claude Code,
and Copilot, while providing a single consolidated view into their work. 

The goal is to provide a high-performance, configurable, and provider-agnostic agentic coding experience.

This repo is still very early in development - expect bugs and rough edges.

## Installation

Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/suru-ai/suru/main/scripts/install.sh | bash
```

Windows:

```powershell
irm https://raw.githubusercontent.com/suru-ai/suru/main/scripts/install.ps1 | iex
```

Each release attests its archives' build provenance, and the binary's inside them, so you can check an installed
binary was built by this repository's release workflow with `gh attestation verify ~/.local/bin/suru --repo suru-ai/suru`.

To uninstall, delete the binary: `~/.local/bin/suru`, or `%LOCALAPPDATA%\Programs\suru\suru.exe` on Windows.

## Features

The core "type a prompt and stuff happens" flow works as you might expect, so here are some of the additional
cool things Suru does:

- Cross-provider subagents - a Claude-managed Opus session can spin up a Codex-managed Astra session to review its work.
- Remote access - run `/serve` to issue an Invite on one machine, then paste it into `/pair` on another, which also lists
  your machines logged in at your Relays, so that you can see and drive the first machine's Sessions from the second.
  Reaching it directly needs a network path between the two machines; where there is none, a Relay can carry the
  connection instead (below). Every connection, redeeming the Invite included, is TLS 1.3 pinned to the Serving machine's
  key, with the other machine presenting its own key in a client certificate.
- Relays - where your machines cannot reach each other directly, run a Relay of your own that each connects out to
  over HTTPS. It carries the Pairing's end-to-end TLS without being able to read it, and admits users by GitHub login.
  See [Running a Relay](docs/relay/README.md).
- Start at login - run `suru server install` so a machine comes back after a reboot with no one starting Suru on it.
  See [Starting a Server at login](docs/relay/README.md#starting-a-server-at-login) for what each platform covers.
- From the command line - `suru relay` and `suru serve invite` add a Relay, log in and issue an Invite on a machine with
  no one at its screen. See [From the command line](docs/relay/README.md#from-the-command-line).
- `/sidekick` - an agent with a view over all your sessions across all of your machines. Use it to answer meta questions,
  or to orchestrate work across workspaces and remotes.

## Releasing

Releases are cut by running the Release workflow from GitHub Actions. It signs and notarizes the macOS binary with
secrets a maintainer sets up on a Mac by running `scripts/macos-signing-wizard.sh`, which walks through creating
each one and uploads them; run it again to renew the certificate or rotate the notarization key.
