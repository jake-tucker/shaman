# Follow a Config Document changed by any hand, by reading it again

_Changes what a Config Document is to a running Server. Bears on ADR-0026 and ADR-0043, and says where the second one's bound ends._

Until now a Server read its Config Document at start and after each edit it made itself, so a hand's edit took effect at the next restart or the next change from the settings panel. Dotfiles and a company's rollout (#612, a Relay pinned in a Config Document) are hand edits by definition, and they are meant to take effect on machines nobody is looking at. So a running Server now looks at its Config Documents about once a second and puts what they say in force through the same adoption a panel edit goes through, with no restart.

## What it does

- **Every Setting is live, not only the one that asked for it.** There is one adoption path, and giving one Setting a live route while the others waited on a restart would make a document mean different things depending on which line of it changed. Provider runtimes take the new Settings, the Serving listener moves, Relay entries follow, every Client is told, Approval Postures are reconciled on live Sessions, and Workspace Roots are rediscovered.
- **It polls.** A look `stat`s each name a document may go by, following symlinks, and reads a document only where that could tell anything new. See *Why polling*, below.
- **Two reads running must agree before anything is adopted.** Suru's own writes are not atomic and nor are all of a hand's, and an emptied file parses cleanly as a document pinning nothing. Adopted, a blip like that would reset every Setting for a second, including what an Approval Posture allows. What it costs is about a second more before a change takes effect, so a change lands within about two seconds. It does not protect against a document that stays wrong; nothing here can. A final read is made inside the one-at-a-time lock on changes, so a look that saw one of Suru's own writes before it was adopted changes nothing.
- **A document that cannot be read whole holds what is in force.** It is told as a diagnostic, and every Setting stays as it was until the document is mended. A half-finished edit takes nothing away from a running Server: not a Serving listener, not a pinned Relay's refusal. A Server *starting* has nothing to hold, so it runs on defaults, as it always has. So a broken document and a restart together reset what a broken document alone does not.

### What a look can miss, and the bound on that

A look skips reading when the stamp (modification time, length, and on Unix the device and inode) is unchanged. A stamp can fail to change:

- **An edit of the same length within one tick of the modification time.** Git calls this a racy file. A stamp modified within five seconds of the look is not trusted, so such an edit is read as long as the tick it fell in could still be current. Five seconds covers FAT's two-second tick and a look's interval.
- **An edit that puts the time back, or a clock that disagrees** (`rsync -t`, a network home). A time ahead of the clock counts as recent. Beyond that, the next rule bounds it.
- **A file swapped behind a link, with the same time and length.** A Nix store gives every file the same time, so this is ordinary there. On Unix the inode tells them apart. Windows has no stable equivalent in std, so it falls to the next rule.

Every 60 looks, about a minute, the document is read whatever the stamp says. That is the whole bound: **anything no stamp can tell is followed within about a minute.** The idle cost is two `stat`s a second and one parse a minute.

## Why polling rather than OS file notification

`notify` (inotify, FSEvents, ReadDirectoryChangesW) would be quicker and idle more cheaply. Polling was chosen because this file is watched for the machines nobody is looking at, and so a missed event is the failure that matters:

- Atomic saves replace the file, so a watch must be on the directory, and a dotfile manager's link means watching the directory of a target that can itself be repointed.
- Network and container-mounted homes commonly deliver no events at all, and a missed event is silent. A watcher would need a periodic read as a backstop anyway, which is then the part that has to be right.
- It is a new dependency with per-platform behaviour, on a code path that Suru must keep working on all three platforms, for a file that changes a few times a year.

A second of latency is no cost here. If idle cost ever matters, a watcher can be added in front of the same looks without changing what they promise.

## Who can use this, and where ADR-0043's bound ends

ADR-0043 bounds a Sidekick by the Tools it is offered: none touches the Settings governing Serving or a Pairing, and none changes an Approval Posture. That bound is on **Tools**, and it holds. It does not extend to the file. **Suru does nothing to protect its config directory.** It is an ordinary directory the user owns, unlike the owner-only records in the data directory, and any process running as that user can write it. Agents are such processes, Sidekicks included. So:

- An Agent able to write outside its Workspace can write `suru.jsonc`, and **`serving.enabled: true` opens a listener within seconds**. A reviewer measured under three seconds.
- By the same path, and this is the sharper of the two, it can write `provider.claude.permissionMode: "bypassPermissions"`, or the Codex or Copilot equivalents, and **live Sessions following that Setting are moved to the wider posture, as a panel change moves them, the writing Agent's own Session among them.** ADR-0026's postures and ADR-0043's exclusion of them both stop at the file. An Approval Posture bounds what an Agent may do through its Provider's checks. It does not defend the file that sets it, unless those checks do.

What stands in the way today is the Provider's own gate on the write, under the Settings' defaults (`src/protocol.rs`):

| Provider | Default | What that does about a write to `~/.config/suru/suru.jsonc` |
|---|---|---|
| Codex | `sandboxMode: workspace-write`, `approvalPolicy: on-request` | Suru sends `workspace-write` with no extra writable roots, so a config directory outside the Workspace is outside the sandbox and escaping it asks. Unless a Workspace contains the config directory: a user's home as a Workspace, or their dotfiles repository where the document is linked from, is in reach. |
| Claude | `permissionMode: auto` | Claude's own classifier decides. Suru adds no rule about this path. What the classifier does with it was not verified and may change. |
| Copilot | `permissions: auto` | Copilot's safety judge reviews each request, and Suru approves what it recommends. Suru adds no rule about this path. What the judge does with it was not verified and may change. |
| Any | `danger-full-access`, `bypassPermissions`, `allowAll` | Nothing stands in the way. |

**Why this is accepted.** The file gate was never ADR-0043's to hold, and the re-read did not open the door, only shortened the wait:

1. The document could already be written. Before this, the write took effect at the next restart or the next panel change, whichever came first, which a user would not connect to it. What changed is seconds instead of later, and a shorter delay is not a different privilege.
2. Whoever can write the config directory can write the data directory beside it too, as the same user, whatever its file modes: `peers.json` among it, which a Server reads at start. That is a more direct way to be trusted by a machine than a Setting.
3. An open listener admits only pinned Peers. Turning Serving on opens a port, not the machine: enrollment still takes an Invite, and a Pairing's key is pinned (ADR-0017, ADR-0046). The damage alone is a port listening where the user did not ask for it. That is not nothing, and it is less than it sounds. Widening a posture is the larger of the two, and it stays inside the Provider checks above.
4. A protection that held would have to be enforced by the operating system against the user's own processes, which Suru has no means to do from where it runs.

## Reconsider if

- An Agent is ever run under a different OS user or a real sandbox than the user's Config Documents, so that writing them is a boundary it does not already have. Then the file is worth defending, and live adoption becomes the thing that makes crossing it instant.
- Provider defaults change so that an Agent's default write reach takes in the config directory. Then the table above is the first thing to re-check, and the answer to *who can use this* gets much wider.
- A Setting comes to do what no later change undoes, or what a Peer or the network can exploit at once. Live adoption would then deserve an exception for it, and the honest way to do that is a Setting the file alone cannot turn on, such as asking in the Client for a posture or Serving change that arrived by file.
- A document changing under a running Server proves surprising, as opposed to useful. Scoping the re-read to named Settings is the retreat, at the price of the mixed meaning above.

## Consequences

- `ServerTimings.config_document_check_interval` injects the pace, and the rest of it counts in looks, so tests see a change at once.
- Worth knowing for anyone editing a document on a live machine: a typo in a value is ignored alone, a broken document changes nothing until mended, and everything else takes effect in about two seconds.
