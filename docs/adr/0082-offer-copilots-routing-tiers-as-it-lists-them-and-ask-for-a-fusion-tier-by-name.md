# Offer Copilot's routing tiers as it lists them, and ask for a Fusion tier by name

_Supersedes ADR-0060, and amends ADR-0070 where it follows from the assignment: how the tiers are learned, and what a Session opened with the Setting off does once it is on. The rest of ADR-0070 still governs a Routing Tier._

ADR 0060 had Shaman write an experiment assignment by hand, because Copilot offered Fusion's routing tiers only to a Session carrying it, and routed a tier on an older plan at about 36 times the cost without its second parameter. Copilot CLI 1.0.114 does neither. It lists the tiers to anyone who asks for its Models, with or without a Session, and a Turn on a tier with no assignment routes on the same plan, policy and rule as one carrying Shaman's (docs/validation/copilot-dynamic-auto-tiers.md). We decided Shaman asserts nothing to Copilot's service. No Session carries an assignment, the Routing Tier option is read off the catalog Shaman already fetches, and the Session a discovery opened to learn the tiers is gone along with its deadline.

The `provider.copilot.fusionTiers` Setting stays, Experimental and off by default, and now governs only what Shaman does: whether `auto` offers a Routing Tier, and whether a Session runs on one. Copilot would take a tier however the Setting stands, so with it off it is Shaman that asks for none, for ADR 0070's reason: a user who turns it off has said to stop spending on tiers. Nothing about a tier is fixed as a Session opens any more, so one opened with the Setting off takes a tier from its next Turn once the Setting is on, where it used to wait until it reopened.

A Selection goes on naming a tier by Copilot's routing ID, such as `hydrafusion`, because that is what Copilot reports a Session as holding everywhere it says so: `session.model.getCurrent`, `session.auto_tier_switch_failed` and `session.model_change`. Copilot now takes a Fusion tier by the name its own CLI does, `standard`, `high` or `max`, and deprecates asking by ID. So a switch asks for a tier by the Fusion name the catalog gives it, read from the last catalog listed on the harness process, and by its ID where the catalog gives it none or this SDK build cannot say the name.

## Considered Options

- **Keep asserting the assignment.** Rejected: it tells Copilot's service something it did not issue, to no effect, through an SDK field marked `#[doc(hidden)]`.
- **Drop the Setting as well, and always offer the tiers.** Rejected for now: it would change what every user's `auto` offers and take away the one switch that stops spending on tiers. Whether the Setting leaves the Experimental group is a decision of its own.
- **Name a tier in a Selection by its Fusion name.** Rejected: Copilot reports what a Session holds by routing ID, in its events as well, so every report would need a catalog at hand to read. A tier with no Fusion name would still go by its ID, and Selections already stored name IDs.
- **Write the three names into Shaman beside their IDs.** Rejected: the IDs are the part Copilot deprecates, and its catalog says which name each stands for.
- **Open a Session to learn the tiers where the catalog lists none.** Rejected: the catalog's listing is documented as the same as a Session's, and no CLI the SDK now speaks to was seen to differ.

## Consequences

- A CLI that lists no tiers without the assignment offers `auto` no Routing Tier, and nothing announces it. The SDK accepts only protocol 4, and 1.0.114 is the one version checked. Which version first listed them is unknown.
- Whether an account is offered a tier is now Copilot's to say rather than Shaman's to assert. One `individual_pro` account was checked. What an account without Fusion is listed is unknown.
- A catalog refresh opens and deletes no Session, and offers the tiers as soon as the Models are listed.
- A switch made before any catalog was listed on its harness process asks by ID. A Session lists the catalog as it starts, for its prices, so none was seen to.
- Should Copilot stop taking routing IDs, a Selection naming a tier the catalog no longer lists is refused, and the Turn runs on what Copilot holds with a Status row saying so, as any refused tier does.
