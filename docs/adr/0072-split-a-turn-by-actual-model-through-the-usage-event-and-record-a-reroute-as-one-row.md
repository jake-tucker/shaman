# Split a Turn by Actual Model through the Usage event, and record a Reroute as one row

Every Turn was filed under the Model its Agent Selection named, though Copilot's Auto, Claude's aliases and native Subagents, and Codex's `model/rerouted` each say which Model did which part. We decided a Turn records **Model Shares**, one per Actual Model, carried on the `ProviderEvent::Usage` every Provider already emits. That way a Provider adds Shares without new machinery, and one with no evidence emits none and keeps its whole Turn under its Selection.

A Share is stated on its Turn's own terms. Its Usage is part of the Turn's Usage, for the same work. Its Cost has the Turn's Cost Basis and Cost Coverage. Where the Turn's Cost is a running total in a reporting lifetime, as Claude's is, the Share's Cost is that Model's running total in the same lifetime, never the Turn's increment of it, and Cost Coverage takes each Share's increments as it takes the Turn's. Like the reading it splits, each Usage event's Shares replace the last ones whole, so a Provider restates them rather than sending deltas. What the Shares leave uncovered is the **unattributed remainder**, held by the Selection.

The Turn's Usage is divided in the Shares' order (`protocol::DividedUsage`): each Share takes no more of each token count than the Shares before it left. The parts therefore always add up to the Turn's Usage and the remainder is never negative, even where a Provider's Shares state more than the Turn. Cost Coverage divides a Turn's Cost contribution the same way. It also counts a shortfall no larger than the Shares' own rounding to a billionth of a dollar as covered, so that rounding never files a wholly covered Turn under its Selection for nothing. A Turn holding a Share and a remainder counts under both keys in the Actual view, and once in the total.

An Actual key is labelled from the Model Catalog by `protocol::actual_model_row`, which the Transcript's Model names use too, so the two agree. That is the row listed as the Model, or else the row resolving to it (`ModelDetails::resolved_model`) with the shortest id, never the catalog's default. The default's name says how a Model was chosen, not what ran, and the shortest id is the base row ahead of its variants. A key no row names is labelled by its id. An alias and the Model it resolves to, labelled from one row, read as one name, and are told apart only where both are series of one report.

A **Reroute** — the Provider switching a Turn from one Model to another for reasons of its own — is one Provider-neutral `Activity::Reroute` that every Provider records alike and a client shows as a Status row. It names the Provider, the Model it left where it said, the Model it runs from there, and why in the Provider's own words, as display text never parsed. It never changes the Selection, and it settles nothing.

## Considered Options

- **A Model Share event of its own.** Rejected: a Provider would have to keep two readings of one Turn in step, and the Turn's Usage and its Shares could disagree between them.
- **Shares as increments added on each event.** Rejected: every other part of the Usage event replaces the last, and a Provider restating its reading after a lost event would count a Share twice.
- **A Share's Cost as the Turn's increment where its Cost is a running total.** Rejected: the metrics read takes the highest running figure per Model, so increments would be counted as zero on later Turns.
- **Spread the remainder across the Shares in proportion.** Rejected: it guesses which Model did work no evidence attributes.
- **Label an Actual key by the first row resolving to it.** Rejected: the label then depends on catalog order, so a 1M-context variant listed first names its base Model.
- **A Reroute row per Provider, or Provider-shaped reasons.** Rejected: every client would need each Provider's row type to show what is the same event to the user.
- **Show Copilot's `rate_limit_auto_switch` as a Reroute.** Rejected for now: it moves the rest of the Session onto Auto, which reaches wider than a switch within one Turn, and how to show it is a decision of its own. Auto choosing a Model per call is not a Reroute either; Model Calls already show it.

## Consequences

- Turns recorded before Shares were kept have none, and stay under their Selection in the Actual view. Nothing rebuilds them.
- A Provider's Shares must fit its Turn's Usage. One that states more has the excess clipped in order rather than failing the read.
- Claude's Usage now includes its native Subagents' tokens (ADR 0071), so the Actual view and the Selected view total the same work.
