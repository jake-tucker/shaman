# Selecting a Routing Tier on Copilot's `auto`

Recorded 2026-11-07 against GitHub Copilot CLI 1.0.105 on macOS, driven through
github-copilot-sdk 1.0.29-preview.5, on an `individual_pro` account, with
`examples/copilot_fusion_capture.rs` (`clear`, `first-switch`, `store`) and
`examples/copilot_routing_tier_live.rs`. ADR 0070 records what Suru decided from it.

## Clearing a tier with the assignment gone

A Session with one Turn on `hydrafusion`, disconnected and resumed by a new CLI with no assignment:

| Way | Answer | `getCurrent` after | Next Prompt | Prompt after |
| --- | --- | --- | --- | --- |
| `set_model(auto, reset)` | Ok | `{auto, hydrafusion}` | `session.model_change {autoTier: null}`, `auto_mode_resolved`, ordinary Turn | ordinary Turn, `{auto}` |
| `set_auto_tier(None)` | `Pending` | `{auto, hydrafusion}` | the same | the same |

So clearing needs no assignment, and takes effect at the next Turn, which then runs and bills as plain
Auto rather than failing with "requires dynamic Auto tiers". Suru uses `set_model(auto, reset)`, since
it switches Models with that same request.

## Reaching a tier from a Session with no Model

Suru creates a Session with no Model and switches as its first Turn begins. With the assignment,
`set_model(auto, hydrafusion)` after a `session.model.list` answers Ok and `getCurrent` says
`{auto, pendingAutoTier: hydrafusion}`. `session.model_change {newModel: auto}` with no tier fields
arrives at once, and the one naming `hydrafusion` after `session.fusion_completed`. That first change
says nothing of a tier, which is one reason Suru does not read refusals off `model_change`. On disk,
`events.jsonl` spells the tier of a change `autoTierIdCompat` where the stream says `autoTier`.

## What Copilot holds for a Session that chose no tier

`copilot_fusion_capture plain`: a Session with the assignment and no Model, `set_model(auto)` naming no
tier, one one-word Turn, and a resume with the assignment. `getCurrent` answered `{modelId: auto}` after
the switch, after the Turn, and after the resume, with no `autoTier` at any point. So a Session naming
no tier resumes with nothing to clear, and Suru sends it no `switchTo`. `autoTier: balance` is reported
only for a Session that named `balance`, as `s8-tiers` did when it opened on it.

## What a deleted Session leaves in `session-store.db`

Each Session deleted with `delete_session`, then looked up by ID in every table with a `session_id`:

| Settings | Turn | `sessions` | `turns` | `assistant_usage_events` | `search_index` |
| --- | --- | --- | --- | --- | --- |
| An Errand's: `enable_session_store(false)`, no Tools, no Skills | none | 0 | 0 | 0 | 0 |
| A conversation's: defaults | none | 0 | 0 | 0 | 0 |
| An Errand's | one, on `gpt-5.6-luna` | 0 | 0 | 0 | 0 |
| A conversation's | one, on `gpt-5.6-luna` | 1 | 1 | 1 | 1 |

So a discovery Session, which runs no Turn, would leave no row either way, and the Errand's settings are
what keep an Errand's Turn out. A conversation Session keeps its rows after `delete_session`, which is
Copilot's own behaviour and outside what this touches. Its directory under `session-state` goes either
way.

## A real Suru Session

`copilot_routing_tier_live`, from a scratch directory, with a one-word Prompt each Turn, on the code
that stores no tier for Copilot's default:

| Step | Turn | Selection after | Status row | Route row |
| --- | --- | --- | --- | --- |
| Setting on | none | catalog 3s later: `balance` Standard, `hydrafusion` High, `hydrafusion-max` Max, default `balance`; `fast` absent; no discovery Session left in `session-state` | | |
| Create on `hydrafusion` | Completed | `hydrafusion` | | `gpt-6-luna` |
| `not-a-real-tier` | Completed | `hydrafusion` | Copilot refused it, in Copilot's words | `gpt-6-luna` |
| `efficiency` | Completed | `hydrafusion` | Copilot refused it (the request failed) | `gpt-6-luna` |
| `hydrafusion` | Completed | `hydrafusion` | | `gpt-6-luna` |
| Restart, Setting on | Completed | `hydrafusion` | | `gpt-6-luna` |
| Asks for `balance`, Copilot's default | Completed | none, and `session.model_change {autoTier: null}` | | none: plain Auto |
| Restart, Setting off, `hydrafusion` chosen again | Completed | none | Routing Tiers are turned off | none: plain Auto |
| Next Prompt | Completed | none | | none |

The Session's `totalNanoAiu` came to 397,542,100 across eight Turns, 0.40 credits.

## Not verified

- **A disabled tier.** Every tier reported itself enabled on this account, so a disabled one was never
  refused live. Suru treats it as either refusal above.
- **A disabled default tier.** Nor was one seen, so what Copilot routes on with no tier set and its
  default disabled is unknown. Suru offers no option then, which only the scripted CLI exercises.
- **A hung or failing discovery.** Only the scripted CLI exercises one.
- **A Subagent on a tier's Session, live.** Its Selection is built without one, which the tests read.
- **Windows and Linux.** Every observation above is from macOS.
