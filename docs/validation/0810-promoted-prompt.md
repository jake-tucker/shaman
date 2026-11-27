# Whether a queued Prompt promoted to steer reaches its Agent

Seen for [#810](https://github.com/jake-tucker/shaman/issues/810), on `982daad7`, macOS 27.2.1 on arm64, 2026-11-28.

It does not. On a Server that never stops, a Prompt queued behind a working Turn and then promoted ("Steer now") is given to its Provider at no point, and reads `delivered` once that Turn settles.

## How it was seen

`tests/codex_integration/promoted_prompt.rs` is a probe, not a test. Each case:

1. starts a real `shaman __server` process against a scripted Codex that begins every Turn it is asked for, holds it until let go, and accepts every `turn/steer`;
2. begins a Session with Prompt **A**, whose Turn Codex is then working on;
3. walks a few steps a reader might take, and after each reads the Session over HTTP and the script's own log of what Codex was asked.

"Given to Codex" below means Codex received a `turn/steer` or a `turn/start` carrying the Prompt's text. The reports are in [`0810-promoted-prompt/`](0810-promoted-prompt), one a case, a step a key.

Run it again with:

```
PROBE_810_OUT=<an existing directory> cargo nextest run --test integration -E 'test(/probe_810/)'
```

## What was seen

| Case | Steps | What Codex was given | How B reads at the end |
| --- | --- | --- | --- |
| Admitted as a steer (control) | B admitted `steer` | `turn/steer` B, within 300 ms | `delivered` as soon as Codex accepted |
| Left queued (control) | B admitted `queue`; Turn 1 ends | `turn/start` B, as Turn 1 ended | `delivered`, the Prompt that began Turn 2 |
| Promoted | B queued; promoted; 1.5 s; Codex runs a command; 1.5 s; Turn 1 ends | nothing | `delivered` |
| Promoted twice | B queued; promoted; promoted again; 1.5 s; Turn 1 ends | nothing | `delivered` |
| Promoted, then another steer | B queued; promoted; C admitted `steer`; Turn 1 ends | `turn/steer` C alone | `delivered`, its Message after C's |
| Promoted, another queued behind | B queued; D queued; B promoted; Turn 1 ends | `turn/start` D | `delivered`, in Turn 1; D began Turn 2 |
| Promoted, then interrupted | B queued; promoted; the reader interrupts | `turn/interrupt` | `delivered`, in the interrupted Turn |
| A steer Codex refused | Codex set to refuse; B admitted `steer`; Turn 1 ends | `turn/steer` B, refused | `delivered` |

### A promotion asks its Provider for nothing

The promotion answers `200` with `delivery: steer` and `status: pending`. Codex is asked nothing then, nothing in the 1.5 seconds after, nothing when its Turn runs a command (a tool round), and nothing in the 1.5 seconds after that. A second promotion answers the same and changes nothing.

### The Prompt reads delivered when the Turn settles

When Turn 1 ends, completed or interrupted, B reads `delivered` and stands in that Turn as a user Message. Codex was never given its text. No Turn follows for it: the Session reads `idle`.

- With D queued behind it, D begins Turn 2 and B, written first and asked for sooner, is passed over.
- With C admitted as a steer after the promotion, C is steered and delivered at once and B is not; at settlement B's Message lands after C's, so the Transcript has them out of the order they were written in.

### A steer Codex refuses reads delivered too

This was not the question, and it is the same reading. A Prompt admitted as a steer, which Codex refused, stayed `pending` under an Error ("Provider steering failed: …"), as `scripted_codex_rejection_keeps_the_steer_pending_and_reports_the_failure` holds it should. When the Turn ended it read `delivered`, a user Message in that Turn, and Codex had refused the only request that carried it.

## What the source says beside this

- `promote` (`src/sessions/prompts.rs`) changes the Prompt's delivery and nothing else. The route (`promote_prompt` in `src/server.rs`) calls it and tells no Provider actor.
- An admission that is to steer does two things a promotion does not (`admit` in `src/sessions/prompts.rs`, and its caller in `src/server/operations.rs`): it binds the Prompt to the Turn it steers (`steer_targets`), and it sends the actor `ProviderCommand::SteerPrompt` (`schedule_steer`).
- The actor steers only on that command, with `next_pending_steer`, which passes over a Prompt bound to no Turn. So a later steer's command does not carry a promoted Prompt along, as the case with C shows.
- `finish_provider_turn` (`src/sessions/settlement.rs`) marks every Prompt still pending as a steer `delivered` as its Turn settles, and writes its Message, whether or not its Provider was ever given it. `continuation_boundary` does the same and is called by a test alone.
- Every adapter is reached by a steer through one method, `ProviderSession::steer_turn`, which the actor calls from that one place. So nothing here is in an adapter: Copilot's and Claude's Sessions are passed over by the same code before either is asked.
- An admission makes checks of a steer that a promotion skips: whether the Provider takes a Skill Invocation in a steer (Claude does not, and says "queue this Prompt instead"), and whether the Worktree was recreated under a working Agent. A promotion refuses only while a Compaction's Turn runs (ADR 0041).
- A promoted Prompt cannot be cancelled: `cancel` takes a queued Prompt alone.
- Two tests in the suite hold today's reading in place. `pending_prompt_mutations_and_interruption_converge_across_clients` (`tests/session_integration/prompts.rs`) expects a promoted Prompt to read `delivered` after an interruption, against a Provider that is never asked to steer. `scripted_codex_handles_pending_steers_before_starting_the_queued_turn` (`tests/codex_integration/steering.rs`) admits a steer while Codex has yet to answer `turn/start`, and expects it `delivered` with no `turn/steer` sent and its text in no `turn/start`. The second was run here and passes.

## Not seen

- Windows and Linux; the probe's Codex is a shell script.
- Copilot and Claude. They were read from source as above and neither was run.
- A live Provider. A scripted Codex accepts a steer by answering the request, and says nothing of what a real one does with it.
- A promotion under a Continuation, which takes no steer. From source, such a Turn's settlement does not sweep its pending steers, so a Prompt promoted there would stay `pending` as a steer on an idle Session until some later Turn settled.
- A promoted Prompt carrying a Skill Invocation or an Attachment.
- A Remote's Session, promoted through a Pairing.
- Any change to the Server. Nothing was built beyond the probe.
