# Show a Route's staged work live and settle it at its commit by ID

On a Routing Tier, Copilot runs a Turn as one or more Routes, whose phases stage their Messages and Tools as ephemeral events and whose commit then re-sends the winning phase's, once, in order, under the same `messageId` and `toolCallId` (docs/validation/copilot-fusion-routes.md). Nothing retracts a losing phase. A phase can run for minutes, and its Tools change the workspace as they run. We decided Suru shows staged work as it arrives, treats the first copy of an item as the one that stands, and reads the commit for one thing: which phase won. A committed copy of an item already shown adds nothing, which holds whichever reaches Suru first, and one still running settles on its copy. The Messages of the Route's other phases become Superseded Messages, a state of the Message itself. Their Tools and Reasoning stay as they are, since that work happened and the files still hold it. A new Provider-neutral `Route` Activity records each decision where it was made: the Route chosen, a verdict, a handoff, a fallback, and, on an interrupt, a failure or a crash, that the Provider discarded the conversation.

Only a Route's own events key its state, by `fusionId`. `turnId` restarts at each phase and `session.fusion_completed` closes a Route, not a Turn, since one Turn can hold two, so a Turn settles where any Copilot Turn does, at the loop's idle (ADR 0015). A failed Route's fallback runs live with neither a commit nor a completion, and goes through the ordinary streaming path.

A Message is identified to the store with a new event beside its start rather than by a field on the start, because only Routes speak of a Message after it ends, and the rest of `ProviderEvent` is left as every Provider already sends it.

## Considered Options

- **Hold staged work until the commit.** Rejected: a Route running for minutes would look like a hang, and an abort or crash would erase work whose file changes remain.
- **Show only the commit, dropping what the other phases did.** Rejected: a rejected attempt's edits stay in the workspace, and hiding the commands that made them would mislead.
- **Replace staged rows at the commit.** Rejected: nothing changes in a committed copy but its `commitId`, so a replacement would only redraw rows and lose the shell output `tool.shell_output` streamed, which has no durable copy.
- **Retract a losing phase's Messages from the Transcript.** Rejected: the reader watched them being written, and one that vanishes reads as a fault. They stay, dimmed and labelled.
- **Mark Superseded Messages in the TUI alone.** Rejected: every reader of a Turn's final Message, a Subagent Report's excerpt among them, must agree, and those readers are on the Server.
- **Use Status rows for a Route's decisions.** Rejected: a typed kind lets a Turn Fold keep them outside and lets a reading label them, and it leaves Provider-specific wording out of the store.
- **Keep a row for the Route's commit.** Rejected: it decides nothing the reader did not already see, and it would stand on every one-Model Route beside the row naming its Model.

## Consequences

- A Route's rows stay outside a closed Turn Fold, so a one-Model Route costs a settled Turn one line, every time. A verdict's account can run to 4 KB, so only its sentence stands there, with the account folded beneath it.
- A person who denies a review's Approval may spend more rather than less: the one denial seen failed the judge, and the Route escalated to its repair, passing over the solver's answer though nothing rejected it.
- A Tool a losing phase leaves running hears nothing further, so it settles with the Turn as `Failed` (ADR 0039) rather than by anything Copilot said. That was never seen outside an abort.
- A restart of Suru itself, unlike a crash of the CLI, settles a Route's Turn without the discarded-conversation row, since the projection that would add it went with the process.
- Copilot could change what a commit re-sends. A copy under an ID Suru has not seen is new work to it and would show, which errs toward showing the same work twice rather than losing it.
