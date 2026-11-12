# Follow an outside edit of a Config Document only for Settings that declare it

_Changes what a Config Document is to a running Server. Bears on ADR-0026 and ADR-0043, and says where the second one's bound ends._

A Server reads its Config Document at start, and writes it when its user or a Sidekick changes a Setting. Anything else may edit the file too: an editor, dotfiles, a company's rollout, or an Agent. This records what such an **outside edit** does to a running Server.

**It takes effect at the next start, unless the Setting declares otherwise.** Every Setting declares `outside_edit` in the compile-time schema, as it declares what a Sidekick may do with it. `AtNextStart` is what all but one say. `Followed` means the Server adopts an outside edit of that key about two seconds after it lands. Only `serving.relays` declares it (#612), because a pin, by definition, is an entry and nothing more: it logs in to nothing, connects to nothing, and leaves Serving through the Relay this machine's choice. A test states the followed keys whole, so adding one is a deliberate act.

**A change of a Setting adopts only what it changed.** `change_setting`, which the panel and a Sidekick's `set_setting` both go through, puts in force the key it edited and the followed keys, and leaves everything else as in force, whatever it finds in the file. Before this, a change adopted the whole document, so an outside edit took effect at the next unrelated change.

So what is in force and what the document says may differ until the next start. Both are honest about it: the panel shows the value in force, because that is what governs, and each differing key gets a diagnostic.

## Why not every Setting

The first version of this decision made every Setting live, on the grounds that there is one adoption path, and that giving one Setting a live route would make a document mean different things depending on which line of it changed. It was overruled, and that option should not come back without answering this.

An Agent runs as the user, so it can write the file if its Provider lets it. Under that option:

- An Agent writing `provider.claude.permissionMode: "bypassPermissions"`, under a prompt injection say, or incidentally because its Workspace is the user's dotfiles repository, would **widen its own Approval Posture and use it within the same Turn**, with no restart and no human in between. Reconciling postures moves live Sessions following the Setting, its own among them.
- An Agent writing `serving.enabled: true` would **open a listener within seconds**. A reviewer measured under three. An open listener admits only pinned Peers, but it is still a port the user did not open.

An earlier draft called that acceptable because the file could already be written, and a sooner effect on it is only a matter of when. That was wrong. For an Agent's own Session the delay is the difference: waiting for a restart ends the Turn and the Agent's process with it, so nothing it writes helps the Turn writing it, and a human is around for what follows. That the data directory can also be written supports the same conclusion, since `peers.json` and its like are read at start too: **nothing an Agent writes takes effect live**, and following every Setting would have made the Config Document the one exception.

#612 needs only `serving.relays` live. A theme waiting for a restart is how things already were, so there is no list of harmless Settings: each would be another line someone has to judge. The narrow thing is also the one that can be widened later.

## The door through a change

This was found while checking the narrow decision, and it predates #612. A Sidekick that can write the file, with the `set_setting` Tool it is offered, could write `permissionMode: "bypassPermissions"`, set `appearance.mode`, and have its own posture widened at once. `change_setting` read the whole document, so it adopted the edit and reconciled the posture. A test through the real Tool surface, run against the code before this change, saw the Provider asked to widen it. ADR-0043 keeps `provider.*.permissionMode` from a Sidekick's Tools, and this went round it. A change now adopts only what it changed, so that door is closed, and the same test holds it shut.

## What the reader sees

An outside edit that isn't followed does not vanish. Each key where the document differs from what is in force gets a diagnostic of its own severity, `AtNextStart`, naming the file and the key and **never the value**, since every diagnostic reaches the Log (ADR-0008).

- The Notice says, once, `shaman.jsonc differs on serving.enabled, provider.claude.permissionMode; takes effect at the next start`. Which keys those are is its identity, so dismissing it holds until the set changes.
- `/settings` leads a focused Setting's pane with it, and its row reads the value in force. A change from the panel goes on from that value and writes over the edit, and the two then agree.
- The Log has it at `info`, once as it appears. A diagnostic already standing is not logged again by later changes.

These are worked out afresh from the document and what is in force, with no record of what was told. A look that finds nothing new adopts nothing, so they do not repeat, and an edit taken back takes its diagnostic with it. They are not "problems": nothing was ignored, and they do not count against the guard that keeps Roots from being taken away on a document's unreliable word.

## How a look works

The Server looks about once a second.

- **It polls.** A look `stat`s each name a document may go by, following symlinks, and reads a document only where that could tell anything new.
- **Two reads running must agree before anything is adopted.** Shaman's own writes are not atomic and nor are all of a hand's, and an emptied file parses cleanly as a document pinning nothing. A blip like that would unpin every Relay for a second. Agreement costs about a second, so a followed key lands in about two. A final read is made inside the one-at-a-time lock on changes, so a look that saw one of Shaman's own writes before it was adopted changes nothing.
- **A document that cannot be read whole holds everything in force**, pins among them, and says so. A half-finished edit neither unpins a Relay nor lifts its refusal. A document **deleted** does say something: nothing is pinned. So pins follow that, their entries standing, while everything else holds and is told of. A Server *starting* has nothing to hold, so a broken document runs it on defaults, as it always has.

### What a look can miss, and the bound on that

A look skips reading when the stamp (modification time, length, and on Unix the device and inode) is unchanged. A stamp can fail to change:

- **An edit of the same length within one tick of the modification time.** Git calls this a racy file. A stamp modified within five seconds of the look is not trusted, so such an edit is read as long as the tick it fell in could still be current. Five seconds covers FAT's two-second tick and a look's interval.
- **An edit that puts the time back, or a clock that disagrees** (`rsync -t`, a network home). A time ahead of the clock counts as recent. Beyond that, the next rule bounds it.
- **A file swapped behind a link, with the same time and length.** A Nix store gives every file the same time, so this is ordinary there. On Unix the inode tells them apart. Windows has no stable equivalent in std, so it falls to the next rule.

Every 60 looks, about a minute, the document is read whatever the stamp says. That is the whole bound: **anything no stamp can tell is noticed within about a minute.** The idle cost is two `stat`s a second and one parse a minute.

### Why polling rather than OS file notification

`notify` (inotify, FSEvents, ReadDirectoryChangesW) would be quicker and idle more cheaply. Polling was chosen because this file is watched for the machines nobody is looking at, and so a missed event is the failure that matters:

- Atomic saves replace the file, so a watch must be on the directory, and a dotfile manager's link means watching the directory of a target that can itself be repointed.
- Network and container-mounted homes commonly deliver no events at all, and a missed event is silent. A watcher would need a periodic read as a backstop anyway, which is then the part that has to be right.
- It is a new dependency with per-platform behaviour, on a code path that Shaman must keep working on all three platforms, for a file that changes a few times a year.

A second of latency is no cost here. If idle cost ever matters, a watcher can be added in front of the same looks without changing what they promise.

## Where ADR-0043's bound still ends

ADR-0043 bounds a Sidekick by the Tools it is offered. This decision keeps an outside edit from taking effect while a Server runs, but **Shaman still does nothing to protect its config directory.** It is an ordinary directory the user owns, and any process running as that user can write it, Agents included. So an Agent able to write there can still:

- **Have it take effect at the next start.** Every Setting, `serving.enabled` and postures among them. An Agent with a shell can bring that start about, by `shaman server stop` then `start`, or by killing the Server where a Keeper restarts it (ADR-0063). That ends its own Session's process with it, and what the document said is told of nowhere after the start: the diagnostics compare a document with what is in force, and after a start they agree. A user who wants to know has the file.
- **Pin a Relay at once.** `serving.relays` is followed for an Agent as for anyone. That adds an entry reading **Login needed**, which connects nowhere and stays after it is unpinned. It can also make an entry's removal be refused, or take away that refusal. It opens nothing unless the user logs in at an address they did not choose, and a Relay holds no trust (ADR-0045): it still cannot make one machine trust another. What it could do there is ask the user to log in at an identity provider it names, so an unexpected entry deserves the same doubt as any unexpected login page.

What stands in the way of the write itself is the Provider's gate, under the Settings' defaults (`src/protocol.rs`):

| Provider | Default | What that does about a write to `~/.config/shaman/shaman.jsonc` |
|---|---|---|
| Codex | `sandboxMode: workspace-write`, `approvalPolicy: on-request` | Shaman sends `workspace-write` with no extra writable roots, so a config directory outside the Workspace is outside the sandbox and escaping it asks. Unless a Workspace contains the config directory: a user's home as a Workspace, or their dotfiles repository where the document is linked from, is in reach. |
| Claude | `permissionMode: auto` | Claude's own classifier decides. Shaman adds no rule about this path. What the classifier does with it was not verified and may change. |
| Copilot | `permissions: auto` | Copilot's safety judge reviews each request, and Shaman approves what it recommends. Shaman adds no rule about this path. What the judge does with it was not verified and may change. |
| Any | `danger-full-access`, `bypassPermissions`, `allowAll` | Nothing stands in the way. |

A protection that held would have to be enforced by the operating system against the user's own processes, which Shaman has no means to do from where it runs.

## Reconsider if

**Widening.** Declaring another Setting `Followed` should have to show that it neither opens this machine nor widens what an Agent may do, whatever value it holds, and that being live is what its use needs: rollouts that can't wait for a restart, as with `serving.relays`. A Setting meeting the first test alone stays as it is, since a theme waiting for a restart is no harm.

Following every Setting would become defensible only if **an Agent could not write the file at all**: run under another OS user or a real sandbox than the user's Config Documents. Then an outside edit is a person's, and live adoption stops being a way to widen an Agent's own posture. Asking in the Client before a posture or Serving change that arrived by file takes effect would be another way there.

**Narrowing.** Reconsider `serving.relays` itself if a pin comes to do more than make an entry: if pinning ever connects, logs in, or offers the Relay to anyone.

**Elsewhere.** Provider defaults changing so that an Agent's default write reach takes in the config directory makes the table above the first thing to re-check. And the next start being a door an Agent with a shell can open, as above, is worth its own decision if telling the user what a start newly adopted ever becomes possible.

## Consequences

- A hand editing a document on a live machine, or a rollout changing anything but `serving.relays`, needs a restart. The reader is told which keys wait, and a change from the panel of that very key takes effect at once, writing over the edit.
- `in_force` and the document are no longer always one thing, so anything asking "what does the user want" must say which it means. The code keeps one rule, `next_in_force`, for looks and changes alike.
- `ServerTimings.config_document_check_interval` injects the pace, and the rest of it counts in looks, so tests see a change at once.
