# Write a plain-text copy to the terminal after the native write settles

ADR-0021 kept plain-text copies writing both routes at once: the native Clipboard through the worker, and the terminal's through OSC 52 in the same step. On a local machine both routes are one Clipboard. A terminal that honours OSC 52 could take it between the two halves of the native write, clearing and then writing, and the native write then failed. The copy still arrived, by the terminal, but macOS printed the failure on stderr, over the frame.

A plain-text copy now waits for the native write to settle before asking the terminal, and asks whether that write delivered or not. The wait is the one a formatted copy already has: at most 250 milliseconds, after which the terminal is asked anyway, and exit asks for the latest copy after the worker's bounded shutdown. A newer copy replaces a waiting one, so only the latest reaches the terminal.

Plain-text copies still reach both routes, as #274 decided. A formatted copy is unchanged: the terminal is asked only if native delivery fails or runs out of time.

## Considered Options

- **Keep stderr off the screen while the TUI runs.** Not taken here: it hides the message and leaves the native write failing. It may still be worth doing for whatever else writes there.
- **Skip the terminal when the native write delivers.** Rejected: over SSH the native Clipboard is the remote machine's, and only the terminal reaches the reader's.

## Consequences

- The terminal is asked a few milliseconds later than before, and up to 250 milliseconds later if the native write stalls.
- A copy replaced while it waits never reaches the terminal, where each plain-text copy used to. The one that replaced it is delivered as its own kind is, so a formatted one that delivers natively leaves the terminal unasked.
- A native write that outlasts the wait can still overlap the terminal's; ADR-0021 already accepted that a call in flight cannot be cancelled. So can the native write of a copy made while the terminal is still acting on the one before: Suru cannot see when a terminal has finished.
- A run that ends on an error rather than an exit does not ask the terminal for a copy still waiting.
