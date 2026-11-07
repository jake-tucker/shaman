# Keep the command line a Client, and say what it means for an Agent with a shell

_Says where ADR-0043's bound ends for the Agent's shell, as ADR-0067 says it for the Agent's file writes._

A machine with no one at its screen has to reach a Relay and issue an Invite without the TUI (#613). `suru relay` and `suru serve` do that as a **Client**: they reach the Channel's Server as any Client does, starting it as one would, and speak to it over its loopback API with its token. Nothing about what they may do is theirs.

**The logic stays in the Server.** Anything that would make a command differ from `/relay` or `/serve` is behind a route they share. `/serve` used to decide for itself whether to turn Serving on and to list the machine's addresses from its own process, so those moved to `POST /v1/pairing/serving`. Which ways an Invite offers by default is decided by `POST /v1/pairing/invites/offered`, and whether a Relay can be offered at all by one rule, `RelayWithheld::of`, that the picker and the route both read. Only the Client's rendering of each differs. Moving this fixed a fault in passing: `/serve` listed every interface address even for a listener bound to loopback, so an Invite could offer ways that could not work. Now the Server lists only what another machine could reach.

`PUT /v1/relays/{address}/login` is the one way a login begins, for every Client, and the Server decides with the entry's operations held. A login under way is left to go on and answered, whatever is asked, so no Client's asking ever ends another's code, as listing first and then beginning could. A Login that stands is answered, and none begun, unless the request says `afresh`: `/relay`'s Enter on a row whose Login stands, for a reader who would stand under another Account, as a Relay joining no two Accounts tells them to. The command line never asks that; it says the Login stands, and `relay remove` is its way to another Account. There was a `POST` that began one regardless and replaced any under way. Nothing needed that replacing, so it went.

**Output is for a script.** What was asked for goes to standard output, and only that; everything else goes to standard error, with no colour anywhere. `relay login`'s address and code share one line, so `| head -1` has both.

## Decisions left open by the issue

- **Adding a Relay already held is success.** Running a provisioning script twice is normal, and what it asked for holds. Refusing would have scripts write `|| true` and hide real failures.
- **Removing a Relay not held is a failure.** A mistyped address would otherwise report success and leave the machine logged in at the Relay the script took it to have left. The two differ because only one of them can hide an opening.
- **An address no Relay could be at is a bad command line.** The Server says so, so that canonicalizing stays in one place, and the command exits 2. A command line can therefore need a Server to be found bad, which is cheaper than two rules for an address.
- **`relay login` exits by how it ended.** Refused by someone (4) and ended with no one refusing it (5) are told apart because a script should retry only the second. A Relay that could not be reached at the start is a plain failure (1): no login began.
- **A short code goes to standard error only where that is a terminal.** It is as secret as the Invite, and standard error is what a provisioning script's log keeps while it captures standard output into a variable. It needs an Account's Login to use, and lives ten minutes, but a machine-readable log outlives the reason for writing it down. `--json` carries it on standard output for whoever asks. #610 asked that the command line print the short form too; this is how it does.
- **A listing never shows a login's code.** `relay list` says one is under way and that `relay login` shows it, since a listing is what lands in logs.
- **A refusal that can be told beforehand turns nothing on.** Asking for a Relay the Server cannot offer fails before Serving is turned on. Where only trying can tell, as with a listener that cannot bind, Serving stays on, as `/serve` leaves it, and the failure says so.
- **The JSON is the command line's own shape,** declared in `src/cli.rs` and documented in `docs/relay/README.md`, not the Server's protocol types. A rename on the API must not silently break a script.
- **A command opens no stream and recovers nothing.** A Server that goes away ends a following `relay login` as a failure, and the next command starts another. Letting it recover would have a one-shot command relaunch a Server behind whatever stopped it.

## What this means for an Agent with a shell

The issue says "there is no new way in". That is true of authority and misleading about reach, so this records both.

**True: the authority already existed.** An Agent's shell runs as the user. The runtime descriptor holding the loopback token is owner-only, which any process of that user passes, so an Agent could already read it and `POST /v1/pairing/invites`, add a Relay, or turn Serving on. ADR-0050 declined to put the token in the credential store for the reason that still holds: every Client reads it to attach. These commands add no route that token could not already reach except the ones above, each of which does what its surface does, and each is refused to Peers and offered by no Sidekick Tool.

**Also true: it is now one obvious command.** `suru serve invite` prints a working Invite on standard output, and `--help` says so. An Agent no longer has to find the token and form a request. So:

- An Agent able to run commands and reach loopback can issue an Invite and send it anywhere. Whoever redeems it holds a Pairing, and a Peer can begin Sessions here, which run commands here. A Peer's user prompting an Agent here can have it do the same, though Peers cannot manage Peers by any route (ADR-0017): that bound is on routes, and this is the shell.
- `suru relay serve-through` and `suru relay remove` are the same.
- An Invite printed in an Agent's shell lands in its Transcript and goes to its Provider. An Agent's next `suru serve invite` also kills the user's outstanding Invite.

**What stands in the way** is the Provider's gate, as ADR-0067's table has it for a file write, and it is no more verified here than there: Claude's classifier and Copilot's judge decide on the command, what they do with it was not tested and may change, and Codex's `workspace-write` sandbox was not tested against a loopback connection. `danger-full-access`, `bypassPermissions` and `allowAll` stop nothing.

**An Agent's shell cannot be told apart, and these commands do not try.** A Provider process inherits the Server's environment plus only what its harness sets. Suru sets no `SURU_*` variable naming a Session, and the Broker's per-Session token does not reach a shell's environment. One could be added, and the commands could refuse under it, but the Agent could unset it, or read the descriptor and use `curl` as before. It would look like a bound and be a speed bump, so it is not built. What would hold is what ADR-0067 names: running Agents under another operating system user or a real sandbox.

## Considered Options

- **Refuse these commands from an Agent's shell by an environment marker.** Rejected, as above.
- **Put the default ways in the command.** Rejected: it is the very divergence the issue warns of. Two copies of "what `/serve` ticks" would drift without a test noticing.
- **Keep a route that begins a login regardless, for `/relay`.** Rejected: two Clients opening `/relay` at once would have the second end the first's code, for a replacing nobody needs.
- **Print the short code on standard error always.** Rejected, as above.

## Consequences

- A command run from a build other than the running Server's replaces that Server, as any Client does (ADR-0001), which ends what it was doing. A script using another install of `suru` should expect that.
- A command after `suru server stop` starts a Server, as a Client does. A stop holds a Keeper back, not a person running a command, and a command that only reads, such as `relay list`, is no exception: the issue has every command start one. Whoever stopped a Server to keep it stopped must not run these.
- A command attaches to a running Server whatever `SURU_CONFIG_DIR` it was given. That is read only where it launches one.

## Reconsider if

- Agents come to run under another user or a real sandbox: then a marker, or the token itself, becomes a bound worth using.
- Issuing an Invite ever gets a Notice in attached Clients. Telling the person that one was issued, by whichever hand, is the visibility a same-user boundary can offer, and this decision would then say so.
