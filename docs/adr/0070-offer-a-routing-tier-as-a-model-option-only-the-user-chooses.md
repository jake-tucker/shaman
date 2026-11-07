# Offer a Routing Tier as a Model Option only the user chooses

ADR 0060 has Suru assert the assignment that makes Copilot offer Fusion's tiers. We decided a Routing Tier is a Model Option on Copilot's `auto`, present only while `provider.copilot.fusionTiers` is on, under a new `ModelOptionRole::RoutingTier` that always goes by the ID `routing_tier`, so that what reads a Selection can tell one with no catalog at hand.

The role itself carries who may choose it. Everything that takes a Selection from an Agent asks `is_the_users_alone` rather than knowing about Copilot: `list_providers` lists no such option, `spawn_subagent` and `begin_session` refuse one named, alike whether the user has any on offer, and a Session an Agent begins under the Landing takes none from it. A Subagent therefore runs on plain `auto`, an Errand never carries one, and the Setting pinning an Errand's Agent offers none.

The tiers are in `session.model.list` only, and only for a Session with the assignment, so a discovery opens a Session under an Errand's least-capable settings, lists, and deletes it, with a 15-second bound. Whatever goes wrong costs `auto` its option and nothing else. A Provider's catalog depends on its Settings through two new `ProviderRuntime` methods, which refresh the catalog when the Setting changes and take away at once what it no longer offers, including from a catalog remembered across a restart. A discovery running as the Setting turns is followed by another.

Copilot holds a tier as Session state and claims it at a later Turn. Suru switches to it as a Turn begins, and the Selection says what the user chose from then on, through the claim, because Copilot's `session.model_change` arrives only at the end of a Fusion Turn and says nothing Suru does not already hold: per ADR 0005, the Selection is corrected only where Copilot says otherwise. It says so as an RPC error, or `session.auto_tier_switch_failed` once tried, and neither fails the Turn: the Turn runs on what Copilot holds, with a Status row saying why, through a new `ProviderEvent::AgentSelectionCorrected`.

With the Setting off Suru clears a tier itself as the next Turn begins, even on a Session still holding the assignment, because a user who turns it off has said to stop spending on tiers. This goes further than ADR 0060, which left an open Session as it opened: the assignment stays until the Session resumes, but its Selection does not. A Session opened without the assignment and asked for a tier gets the same treatment rather than a refusal, since that is what the CLI would answer. Clearing without the assignment was checked live (docs/validation/copilot-routing-tier-selection.md).

Whether a tier can be had is left to Copilot. `normalize_selection` carries a tier the catalog no longer offers, or marks disabled, through to the Turn rather than refusing it from the last thing the catalog heard, so a remembered Selection never strands a Prompt.

## Considered Options

- **Stage the tier on `session.create`.** Rejected: `ProviderSessionRequest` carries no Selection, because a Session is created before Suru knows one, and a create on a tier with no assignment fails at once. The first Turn switches as it does for any Option.
- **A Copilot case in the Broker.** Rejected: every future Provider's tier would have to repeat it, where a role asks nothing of the Broker.
- **A field on `ModelOptionDescriptor` for who may choose it.** Rejected for now: a Routing Tier is the only such option, and the role already names it.
- **Keep a Session on its tier until it resumes, once the Setting is off.** Rejected: an Experimental Setting that keeps spending after it is turned off is the wrong way to fail.
- **Act on `session.model_change`.** Rejected: `set_model` emits one at once, ahead of the Turn that claims the tier and naming no tier, so reading it as Copilot's word on the tier would take every switch for a refusal. A refusal has an event of its own.
- **Offer no default until a tier is chosen.** Rejected: the spec has Copilot's default preselected. A Selection naming none stays valid all the same, for a Subagent and for a Session from before the Setting.

## Consequences

- With the Setting on, a new Session on `auto` names `balance` explicitly, where a Session from before names none. They route alike unless Copilot changes its default, which the second follows and the first does not.
- Each refresh opens and deletes a Session. It costs no credits, about 150ms the first time plus the listing, and leaves nothing in Copilot's picker or its store.
- A Selection naming a tier with the Setting off tells one Status row per Session, so a Landing that remembers one tells each new Session in turn.
- A tier Copilot moves on its own, with no refusal, goes unnoticed until the Session next opens. None was seen.
