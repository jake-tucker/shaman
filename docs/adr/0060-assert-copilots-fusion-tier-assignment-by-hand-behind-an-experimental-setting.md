# Assert Copilot's Fusion tier assignment by hand, behind an experimental Setting

_Amended by ADR-0070: a Session already open no longer stays as it opened. It keeps its assignment until it resumes, but turning the Setting off moves it off a Routing Tier at its next Turn, which is what such a Session does. The tier picker and lowering a tier onto a Session are no longer to come, and the projection neither (ADR-0069), though the Setting stays off by default while it is Experimental. The Session Shaman opens to learn which tiers Copilot offers carries the assignment too, so it is no longer only a conversation Session that does, though Errands and Skill discovery still never do. Everything else here still governs the assignment._

Copilot offers Fusion's routing tiers only to a Session whose experiment assignments carry `copilot_cli_dynamic_auto_tiers`. The CLI fetches assignments for its own modes and gives a Session driven over the SDK none, and no request that identified itself as anything but the Copilot CLI got that parameter back from the service (docs/validation/copilot-dynamic-auto-tiers.md). The SDK does let its integrator inject assignments on create and resume. We decided Shaman writes the assignment itself and calls no experimentation service: two parameters, `copilot_cli_dynamic_auto_tiers` and `copilot_cli_hydrafusion_plan_v2`, both `true`, under the Config `default`, with no Features, Flights or assignment context, defined once in `src/provider/copilot/fusion_tiers.rs`. A conversation Session supplies it on create and on resume while the `provider.copilot.fusionTiers` Setting is on. Errands and Skill discovery never do.

This asserts to Copilot something its service did not issue, which is why a later reader might question it. The assignment names only what Shaman needs and states openly that it is Shaman's own. It claims no identity that isn't Shaman's and reads no credential and no file of the CLI's. How far Copilot's services still hold an account to its plan with the assignment in place has been seen on one account only.

The second parameter is there because the first alone lists the tiers but routes on Fusion's older plan, at about 36 times the cost for the same one-word Prompt.

The Setting is Experimental and off by default. With it off, `session.create` and `session.resume` carry no assignment, exactly as before. Its key names the Provider it configures, but only the Experimental tab presents it, not Copilot's own Settings on the Providers tab, so a reader meets it knowing it is still finding its shape. A Sidekick may read it but not change it, since asserting an assignment to Copilot's service is the user's to do. It takes effect as a Session opens or resumes, because that is the only time an assignment reaches the CLI: a Session already open stays as it opened.

## Considered Options

- **Fetch from the service as Shaman.** Rejected: the service answers without a credential, but only with six generic variants, none carrying the parameter, so a fetch would deliver nothing that turns on the tiers.
- **Fetch from the service as the Copilot CLI.** Rejected: it would mean Shaman claiming to be another program, and it would also need the account's endpoint and tracking ID, which Shaman would have to obtain through auth RPCs whose results can also carry a token.
- **Read the CLI's cached assignments.** Rejected: the cache is the CLI's private file, and its contents switch on every other experiment the account is in.
- **Copy the CLI's whole issued assignment into Shaman.** Rejected: its contents are account-derived, and they would switch on experiments Shaman's projection has never seen.
- **Set `COPILOT_EXP_COPILOT_CLI_DYNAMIC_AUTO_TIERS` on the spawned CLI.** Rejected: it lists the tiers too, but it applies to the whole process, Errands and Skill discovery included, and the CLI's schema marks that catalog internal, where the SDK's field is meant for an integrator.
- **Leave Fusion unavailable.** Rejected: the user wants the tiers available to try, and an off-by-default Setting costs nothing to anyone who doesn't.

## Consequences

- Shaman's assignment can fall behind Copilot's: a parameter renamed or a new one Fusion comes to need won't reach Shaman's Sessions, and nothing announces it. The Experimental group is where a Setting that may stop working belongs.
- Routing is confirmed only for one-word Turns. The other parameters the CLI's issued assignment carries stay out, and their effect on a hard Prompt or a multi-turn Session is untested, so turning the Setting on may spend differently from the CLI on the same work.
- The account is treated as assigned whether or not the service would have assigned it. What that does on a plan without Fusion is unknown.
- The SDK marks `exp_assignments` `#[doc(hidden)]`, so an SDK upgrade may move or remove it.
- Turning the Setting off does not reach an open Session until it resumes, and a resume without the assignment lists no tiers. Should Shaman later select a tier, it will have to decide what such a Session does.
- A tier picker, lowering a tier onto a Session, and the Fusion projection are separate work. Until a committed phase stops doubling in the Transcript, this Setting stays off by default.
