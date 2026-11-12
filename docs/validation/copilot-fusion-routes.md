# Copilot Fusion on the wire

Captured 2026-11-05 against GitHub Copilot CLI 1.0.105 and `github-copilot-sdk` 1.0.29-preview.5, on
the assignment ADR 0060 writes, with `examples/copilot_fusion_capture.rs` driving ten scripted
scenarios across 32 commits. ADR 0069 records what Shaman decided from it; #604 and #605 build on it.

Everything under a heading "Observed" was read from captures. "Inferred" is a reading of them. The
captures themselves are not committed. `tests/copilot_integration/fixtures/fusion-*.jsonl` are
scrubbed from them for what each test needs: only the events the projection reads, long text cut
the same way in a staged copy and its committed twin, and no home directory, username, account value
or opaque provider payload. Scratch paths read `/tmp/fusion-scratch`. Which capture each comes from:

| Fixture | Capture | What it shows |
|---|---|---|
| `fusion-cascade-repair` | `s6i` | A judge rejecting, and a repair committed |
| `fusion-two-routes` | `s6d` | A judge accepting, then a held steer's Route with no idle between |
| `fusion-critique-revision` | `s4-critique-a` | A critic asking for a revision, which commits |
| `fusion-critique-approval` | `s6a` | A critic approving, which commits the draft |
| `fusion-abort-mid-tool` | `s5f` | An interrupt during a running Tool |
| `fusion-route-failed` | `s11` | A failed route, its fallback running live |
| `fusion-review-accepted`, `fusion-review-denied` | Below | A judge's Approvals under the Ask posture, Tools left out |

## What a person is asked during a review (live, through Shaman)

Run after the scenarios below, through Shaman's own Server, projection and Approvals, with
`provider.copilot.permissions` at `ask` and every Approval decided over the client, on `hydrafusion`,
from a scratch copy of the template. The tier was forced for the run by an uncommitted hook, since
choosing one is #605's. All three Prompts routed `cascade` (`gpt-6.1-sol`, judge `gpt-6-luna`, repair
`claude-opus-5.5`).

| Run | Decision on the judge's request | Credits | What followed |
|---|---|---|---|
| Accept | Approved all three | 5.144 | Verdict `accept`, score 5, commit of `phase:0`, no Message superseded |
| Deny | Denied the first; solver's approved | 22.035 | `fusion_phase_failed {status: failed, reason: invalid_output, degradedToPhaseId: :repair}`, handoff to the repair, commit of `:repair`, `fusion_completed {outcome: degraded, degradedReason: judge_invalid_output}` |
| Deny and interrupt | Decline and interrupt on the first | 5.767 | `abort`, `fusion_phase_failed {status: cancelled}`, aborted idle: the ordinary interrupt below |

Observed.

- **What is asked:** an ordinary read, with a path and a reason. Each run's first was `kind: read`,
  `intention: "List directory: <cwd>"`, then in the Accept run `Read file: scheduler.py` and
  `test_scheduler.py`. No shell, no write, nothing outside the working directory. A judge reads with
  Tools no Tool event reports, so each Approval stands beside no Tool row, with a `toolCallId` nothing
  else carries, and between the solver's last Message and the verdict.
- **Asked as any Approval:** durable `permission.requested` with no `fusion` attribution, answered by
  `session.permissions.handlePendingPermissionRequest`, and `permission.completed {kind:
  denied-interactively-by-user}` on a denial. Shaman showed and settled them with no change to its
  Approval path. The review went on after each: each was answered within the 100ms the run polled at, so how long a review waits on a slow person was not measured.
- **A denial does not mean "no":** the one denied judge answered with nothing, failed as
  `invalid_output`, and the Route treated that as a reason to escalate. It ran the repair at about 2.3
  times the cost of the solver (15.301 credits against 6.702), and the solver's Messages were passed over though no review rejected
  them. One run, so whether a judge always gives up at a first denial is unknown. This is the first
  sighting of a phase failing on its own and of `outcome: degraded`.

Inferred. A person who denies a judge's read to save money or keep it from a file spends more, and
loses the answer in hand as the Transcript's final word. Nothing widens what they allowed: the repair
asks for its own Tools in its own right.

## Credits

Below the live runs above, every Tool was auto-approved. Those runs spent 32.946 more, for 601.863 in all.

The ledger charges each run the greatest of the Fusion, `assistant.usage` and `session.shutdown` totals, so
`s9`'s figure double-counts a resumed Session's carried total (true spend about 0.118).

| Scenario | Runs (credits) | Credits |
|---|---|---|
| 1 Hard on High, 2 Follow-up | s1-s2-high: turn 1 11.739, turn 2 4.294 | 16.033 |
| 3 Hard on Max | s3-max 167.445 (cut off), s3b-max-cascade 55.856, s3c-max-full 80.570 | 303.871 |
| 4 Critique | s4-routes-high 0, s4-routes-max 0, s4-critique-a 11.270 | 11.270 |
| 5 Interrupt | s5a 32.439, s5b 10.160, s5c 5.900, s5d 6.419, s5e 28.076, s5f 1.355 | 84.349 |
| 6 Steer | s6a 5.577, s6a2 8.437, s6b 3.380, s6c 12.264, s6d 4.401, s6e 10.283, s6f 9.942, s6g 17.466, s6h1 9.567, s6h2 8.413, s6h3 9.981, s6i 9.971, route probes 0 | 109.682 |
| 7 Crash and resume | s7a 33.509, s7b 9.301 | 42.810 |
| 8 Tier change | s8-tiers 0.334, s8b-claim 0.087 | 0.421 |
| 9 Assignment away | s9-unassigned 0.164, s9b-resume-empty 0 | 0.164 |
| 10 Discovery | s10-discovery 0 | 0 |
| Extra: forced route failure | s11 0.317, s11b 0, s11c 0 | 0.317 |
| **Total** | | **568.917** |

## Scenario results

### 1. Hard on High (`s1-s2-high.jsonl`)

Observed. `hydrafusion`, policy `standard`. Route `cascade`, rule `default`: primary `gpt-6.1-sol`, judge
`gpt-6-luna`, repair `claude-opus-5.5`, fallback `gpt-5.6-luna`, `followUpModel` `gpt-6.1-sol`. Primary ran
285s (10 requests, 12 Tools, 17 messages, 11.58 credits). Judge ran 18s (3 requests, 0.157), verdict `accept`,
score 5. The commit took `phase:0` and re-sent all 41 items in 44ms. `prompt.md` on `hydrafusion` resolved the
same route on all 7 runs that sent it (s1, s5a, s6f, s6h1-3, s7a). Its judge accepted all 5 runs that reached
it; yesterday's `fusion-high2.jsonl` is the one rejection on record.

### 2. Follow-up (`s1-s2-high.jsonl`)

Observed. It routed again: a new `fusion_route_started` and `fusion_resolved`, 1.2s of routing. Route `single`,
rule `solo_terra61_max_low_reasoning`, primary `gpt-6.1-terra`, not the prior `followUpModel`. It had a phase
and a commit like any other: 6 requests, 6 Tools, 12 messages, all staged and then re-sent. `followUp` was absent
from every `fusion_resolved` in every capture, so `reuse_primary` and `reroute` were never seen, and neither was
`selectedRuleId`. Every later Turn in every Session rerouted. The previous Turn's `followUpModel` appears as the
next route's `fallbackModel` (`gpt-6.1-sol` here; also in s5a-e and s6d).

### 3. Hard on Max (`s3-max.jsonl`, `s3c-max-full.jsonl`, `s3b-max-cascade.jsonl`)

Observed. `hydrafusion-max`, policy `max`. Route `cascade`, rule `cascade_opus55_xhigh_sol6_high_astra_xhigh_rule_1`:
primary `claude-opus-5.5` xhigh, judge `gpt-6-sol`, repair `gpt-6-astra`, fallback `gpt-5.6-sol`.

- `s3-max`: primary 573s, 88.8 credits. Judge rejected, score 1: the solver had copied files under the Session's
  `files/` directory, outside the directory the Prompt named. Handoff to Astra, which ran 300s and 74.8 credits
  before the capture tool's wait limit cut it off. No commit, no `fusion_completed`.
- `s3c-max-full`: same route, primary 446s, 78.3 credits, judge accept, commit of `phase:0` (34 items, 51ms).
- `s3b-max-cascade`: a smaller Prompt, primary 741s, judge accept, 55.856 credits.

A Max repair was never committed. A High repair was (`s6i`).

### 4. Critique on plan v2 (`s4-critique-a.jsonl`, `s4-routes-*.jsonl`, plus s5c, s5d, s6a, s6a2, s6c, s6e, s6g, s7b)

Observed. Route `critique`, rule `critique_opus55_medium_luna6_high_opus55_high`: draft `claude-opus-5.5` medium,
critic `gpt-6-luna` high, conditional revision `claude-opus-5.5` high. `critics` and `phasePlan` name the shape.

Prompt that gets it, on `hydrafusion` only: "Which is the better synchronization design for a retrying scheduler:
one condition variable, or separate queues per deadline with a timer thread? Argue it carefully and give a final
recommendation." Of 15 route probes on High, 6 routed `critique`, all design, plan or comparison Prompts. Of 10 on Max,
none did: 8 `single`, 2 `cascade`. Scores were identical between tiers for one Prompt, and across
repeats. Routing cost nothing measurable, even when aborted at once.

The critic asked for a revision in 6 of the 7 runs that reached its verdict; `s6a` approved. Critic content is JSON
`{assessment, feedback, defect}`, and `verdict` stays null. A revision committed `:revision`; an approval committed
`:phase:0`, the draft. Critic: 1 request, no Tools, 15-34s.

### 5. Interrupt mid-phase

Runs: `s5a` primary after 2 Tools, `s5b` judge, `s5c` critic, `s5d` revision, `s5e` single, `s5f` during a running
Tool.

Observed, the same in all six. `session.abort()` returned in 3-10ms. Then within about 10ms, in this order: durable
`abort {reason: "user_initiated"}`; ephemeral `agent.interrupted` (when a request was in flight), `model.turn_failed`;
durable `assistant.fusion_phase_failed {status: "cancelled", reason: "cancelled", errorMessage: "fusion phase was
cancelled"}`, carrying that phase's usage so far; durable `session.usage_checkpoint`; `assistant.idle`; `session.idle`.

- Never present: `session.fusion_commit_started`, `session.fusion_completed`, `assistant.turn_start`, re-sends.
- Nothing is committed, including an earlier phase that had finished. In s5b and s5c the solver's finished answer
  sits durable in `fusion_phase_completed.content` and no message carries it.
- `degradedToPhaseId` null, no failover to another phase.
- s5f: `tool.execution_start` had no matching `tool.execution_complete`, ever, and the command died (`marker.txt`
  absent). In s5a the abort came as a parallel Tool finished, and all completed.
- The next Prompt: a new Fusion with its own route. The original `user.message` stays in the conversation, staged
  assistant and Tool content does not: `messagesLength` 3 and `conversationTokens` back near the start in every
  case, and the Model says it has inspected nothing.
- s5a and s5e: asked only for a two-sentence status, `gpt-6.1-terra` restarted the whole original task. s5b re-answered
  the original question. s5c, s5d just answered. s5e took 12 minutes and 26.8 credits.

Inferred. The Model takes the unanswered first Prompt in history as still open, so what a follow-up after an interrupt
costs depends on how much was asked before it.

### 6. Steer mid-phase (Shaman's `MessageOptions::with_mode(DeliveryMode::Immediate)`)

| Run | Sent during | What happened |
|---|---|---|
| s6b, s6f, s6h1-3 | Primary, Tool in flight | Durable `user.message {delivery: "steering"}` within 3ms, no `fusion` attribution. Next request's `assistant.usage.fusion.hasUserSteering: true`. Answer obeyed it. |
| s6g | Draft, Tool in flight | Same; then critic asked to revise; revision obeyed it. |
| s6i | Primary, Tool in flight | Same; judge rejected; repair obeyed it. |
| s6a2 | Draft, single request, no Tool | Held. Draft and critic ran without it. `user.message` landed 34s later, at the revision's start. Revision obeyed it. |
| s6c | Critic | Held; delivered at the revision's start; obeyed. |
| s6d | Judge, which then accepted | Held past the commit. `fusion_completed`, then with no `assistant.idle` or `session.idle`, a second `fusion_route_started`, `fusion_resolved`, phase and commit, with `user.message {delivery: "queued"}`. Idle came only after the second. |

Observed.

- `hasUserSteering` appears only `true`, only on the request that consumed the steer, and only on `assistant.usage`.
  It is absent everywhere else, including on committed messages.
- Survival: `model.messages_snapshot` of s6i's repair holds, in order, original user, steer user, handoff user, then
  the repair's own work. Staged assistant and Tool messages are gone. The handoff text does not quote the steer.
  s6g's revision snapshot holds it too.
- Judge and critic snapshots never contain the steer, only the solver's quoted answer.
- The steer's `user.message` is durable before any of the phase's messages are, so the durable log reads user, steer,
  then all assistant messages from the commit, whichever came first live.

Inferred. A reviewing phase cannot hold a solver to a steered requirement, because it does not see the steer. A held
steer can make one host-level Turn span two Fusions.

### 7. Crash mid-phase and resume (`s7a`, `s7b`)

`s7a` killed the CLI during primary (cascade), after 2 Tools. `s7b` killed it during the critic, after the draft's
durable `fusion_phase_completed`.

Observed.

- The event history holds only durable events: `session.start`, `fusion_resolved`, `user.message`, `system.message`,
  `permission.requested` and `.completed` pairs, and in s7b the draft's `fusion_phase_completed`. Then `session.resume
  {autoTier: "hydrafusion", sessionWasActive: false, eventCount, ...}`. On disk, also `session.usage_record`.
- No staged event is replayed. No `abort`, `phase_failed` or completion is written: the Fusion is simply left open
  forever in the history, its `fusion_resolved` without a close.
- In 20s of waiting after the resume, nothing ran. `get_current` answered `{modelId: auto, autoTier: hydrafusion}`.
- Next Prompt: a new Fusion. `messagesLength` 3. s7b's Model says it has inspected nothing, though the draft's
  `projectionMessage` sits durable, so that checkpoint is not restored for an uncommitted Fusion. s7a's Model
  restarted the original task (cut off by the capture tool's wait limit).
- Spend survives: `usage_checkpoint` after the resume includes pre-crash requests (s7b 9.30 = 5.83 + 3.48).

### 8. Tier change mid-session (`s8-tiers.jsonl`, `s8b-claim.jsonl`)

Observed.

| Action | Answer | `get_current` | Events |
|---|---|---|---|
| `set_auto_tier(hydrafusion)` from `balance` | `Pending`, `effective: balance`, `pending: hydrafusion` | `{autoTier: balance, pendingAutoTier: hydrafusion}` | None until the next Turn, then durable `session.model_change {previousAutoTier: balance, autoTier: hydrafusion}` **after `fusion_completed`**, before `usage_checkpoint` and idle. |
| `set_auto_tier(not-a-real-tier)` | RPC error -32603: `Auto preference "not-a-real-tier" is unavailable for this account; choose an enabled Auto preference or another model` | Unchanged | None. No `auto_tier_switch_failed`. |
| `set_auto_tier(efficiency)` | `Pending` | `pending: efficiency`, then cleared | Ephemeral `session.auto_tier_switch_failed {reason: request_failed, requestedAutoTier: efficiency, effectiveAutoTier: balance}` at the Turn, before routing. The Turn runs on `balance`. |
| `set_auto_tier(intelligence)`, `(fast)`, `(balance)` | `Pending` | As above | `model_change` at the **start** of the Turn, before `auto_mode_resolved`. These are not Fusion. |
| `set_model(auto, hydrafusion-max)` | Ok | Pending | As the first row. |
| `set_model(auto, not-a-real-tier)` | Same error, naming `session.model.switchTo` | Unchanged | None. |
| `set_model(gpt-6-luna)` | Ok | `{modelId: gpt-6-luna, autoTier: hydrafusion-max}`: the tier is kept | `model_change` at once, no tier fields. Non-Fusion Turn. |
| `set_model(auto)` from concrete | Ok | `{modelId: auto, autoTier: hydrafusion-max}` | `model_change` at once. The next Turn is Fusion on the kept tier. |
| `set_model(auto, reset)` | Ok | `autoTier` unchanged until a Turn | Turn: `model_change {autoTier: null}`, `auto_mode_resolved`. Then `get_current` has no `autoTier`. |
| `set_auto_tier(hydrafusion)` then `set_model(concrete)` before a Turn | Ok | Pending cleared | The request is dropped: back on `auto` it does not return. |
| `set_auto_tier(None)` with nothing to change | `Unchanged` | Unchanged | None. |
| `set_auto_tier(balance)` right after a send, superseding a pending `hydrafusion` | `Pending`, `superseded: hydrafusion` | | The Turn just sent claimed `balance`: the newest request won. |

Not tested: a tier change while a phase is running.

### 9. The assignment taken away (`s9-unassigned.jsonl`, `s9b-resume-empty.jsonl`)

Observed.

- Create with it, one Turn, resume without it: the resume **succeeds**, and `get_current` still says `{auto,
  hydrafusion}`. `session.model.list` has no `auto` block. Each Prompt: `send` answers OK, then durable `session.error
  {errorType: "query", message: "Execution failed: model resolution leaf failed: Auto preference 'hydrafusion' requires
  dynamic Auto tiers; choose an available preference"}`, then idle. No `user.message`. Nothing is billed. The Session
  stays stuck like this.
- `set_auto_tier(hydrafusion-max)` on it: RPC error with the same wording.
- Create on `hydrafusion` with none: `session.create` fails at once, -32603, `session construction failed: model
  resolution leaf failed: Auto preference 'hydrafusion' requires dynamic Auto tiers; choose an available preference`.
  **A refused create left a directory in `~/.copilot/session-state`** (no `events.jsonl`). Those were deleted by ID.
- Plain `auto` with none: works, ordinary non-Fusion Turns with deltas. `set_auto_tier` and `set_model(auto, tier)`
  to a Fusion tier fail by RPC, same wording, and leave it usable.
- Unrelated to the assignment: a Session with no Turn cannot be resumed at all, with or without it: `session not found`,
  though its directory exists (s9b).

### 10. Discovery with no turn (`s10-discovery.jsonl`)

Observed. Three runs on one CLI: `session.shutdown.totalNanoAiu` 0, `totalPremiumRequests` 0, no Models called. Open
129-206ms, first `session.model.list` 1745ms (the CLI fetching), later ones about 6ms, disconnect and delete under 10ms.
CLI start about 440ms. A create staged on a tier took about 1850ms when it was the first thing on a CLI. During the
Session its directory exists with no `events.jsonl`; after `delete_session` it is gone, and the Session has no row in
`session-store.db`.

Offered (`session.model.list().auto`): `defaultTier: balance`, tiers `balance` "Standard" (`type: auto`, `fusionTier:
standard`), `hydrafusion` "High" (`type: fusion`), `hydrafusion-max` "Max" (`type: fusion`), each with `description`,
`status {enabled, pickerVisible}`, and a `billing.promo` banner (30% off to 2026-11-30); `fast` "Fast" with
`pickerVisible: false` and no billing. `efficiency` and `intelligence` are not listed, yet `intelligence` is accepted.

## Across all scenarios

### Ephemeral versus durable

Observed, across all captures. Flags agreed with disk: no ephemeral event was on disk. The exception: the `system.message`
at a revision phase's start is durable in the stream and absent from disk (s4, s5d, s6c, s6g; a repair's is on disk).

| Event | Flag | `fusion` attribution |
|---|---|---|
| `session.fusion_route_started`, `assistant.fusion_phase_started`, `assistant.fusion_phase_activity` | Ephemeral | Own fields |
| `assistant.usage` | Ephemeral | Every phase, including judge and critic |
| Staged `assistant.message`, `tool.execution_start`, `tool.execution_complete` | Ephemeral | Solver phases, no `commitId` |
| `tool.shell_output` | Ephemeral | Solver phases |
| `tool.execution_partial_result`, `session.usage_info`, `pending_messages.modified`, `model.*`, `assistant.idle`, `session.idle` | Ephemeral | None |
| `session.fusion_resolved`, `fusion_route_failed`, `fusion_handoff`, `fusion_commit_started`, `fusion_completed`, `assistant.fusion_phase_completed`, `fusion_phase_failed`, `session.usage_checkpoint`, `session.model_change`, `abort`, `session.error`, `user.message`, `system.message`, `permission.completed` | Durable | None or own fields |
| `permission.requested` | Durable | Solver phases: attributed. **Judge: unattributed.** |
| Committed `assistant.message`, `tool.execution_start`, `tool.execution_complete` | Durable | Carries `commitId` |
| `assistant.turn_start`, `assistant.turn_end` | Durable, inside the commit only | None |
| `session.usage_record`, `session.fusion_change_checkpoint` | Disk only, never streamed | |

`fusion.commitId` is on exactly those three committed kinds, in `primary`, `draft`, `revision` and `repair`. It was never on
a judge or critic event. Phase ids: `<fusion>:phase:0` (primary and draft), `:judge`, `:critic`, `:repair`, `:revision`.

### Which phases emit what

Observed.

- Solver phases (`scope: root`): all of the above, and `phase_activity` of `model_output`, `tool_started`, `tool_completed`.
- `judge` and `critic` (`scope: review`): only `phase_started`, `assistant.usage`, `phase_completed` or `phase_failed`, and
  `model.*`. No messages, no `tool.execution_*`, no `phase_activity`.
- A judge used read Tools with no Tool events, yet **its `permission.requested` and `permission.completed` surface, durable and
  unattributed**, with a `toolCallId` no Tool event ever carries. They appeared in the stream between `phase_started` and
  `phase_completed` of a judge. A critic has no Tools and asks nothing.
- `verdict` (`accept`, `reject`) on a judge only. Judge content is JSON `{score, rationale}`.

### The re-send at the commit

Observed, in all 32 commits. After `session.fusion_commit_started`, and strictly before `session.fusion_completed`, 1-77ms later:
`assistant.turn_start`, `assistant.message`, `tool.execution_start`, `tool.execution_complete`, `assistant.turn_end`, nothing else.

- **Every staged item of the source phase is re-sent, once, in the original order, with the same `messageId` or `toolCallId`.**
  Payload is identical but for `fusion.commitId`, a new event `id` and `parentId`, and `timestamp`. Same `turnId` and `result`. (In s6d the JSON text differs while the parsed values are equal, taken to be number formatting.)
- Items of other phases are never re-sent, and nothing retracts them.
- Not re-sent: `tool.shell_output`, `tool.execution_partial_result`, `assistant.usage`, `permission.*`.
- One `turn_start`/`turn_end` pair per Model request. `turnId` restarts at "0" per phase, so it collides across phases; a
  phase's `interactionId` differs from another's. `originatingMessageId` is the user message's.

### Reasoning and deltas

Observed. No `assistant.message_delta`, `assistant.streaming_delta`, `assistant.message_start`, or `assistant.reasoning` ever
came from a Fusion phase, with streaming on. Reasoning arrives as `reasoningText`, `reasoningOpaque`, `reasoningBlocks` on the
`assistant.message`, staged and re-sent alike. Deltas and `assistant.reasoning` come only from non-Fusion Turns and the fallback
after a failed route. GPT-6.1 Sol and Terra emit bursts of `assistant.message` with empty `content`, no Tools and `reasoningText`
(20 in a millisecond-spaced run in s5a), staged and re-sent.

### What the commit picks

Observed. `sourcePhaseId` equalled `finalSourcePhaseId` in every commit, with `kind: "text"`, `toolCallId: null`. It is not always
the last phase run: judge accept picks `:phase:0`, critic approve picks `:phase:0` (the last phase was the review), reject picks
`:repair`, revise picks `:revision`.

### Failure shapes

Observed. `degradedReason` was null and `outcome` `"completed"` on all 32 `fusion_completed`. The non-completed endings are the abort
and the crash above: no `fusion_completed` at all.

Failed route (`s11`, forced with `allowed_models`; s11b and s11c show the whole route is validated, repair and judge Models
included): durable `fusion_route_failed {reason: "invalid_response", errorMessage: "...references an unavailable model: gpt-6-luna",
fallbackModel}`, carrying `fusionId`. Phase `primary` on the fallback, **not staged**: `assistant.turn_start`, `message_start`,
deltas, one durable `assistant.message` with no attribution, all before `fusion_phase_completed {content: "", projectionMessage:
null, projectionMode: "staged"}`. Then no commit and no `fusion_completed`. `assistant.usage` stays attributed.

### Usage

Observed. One `assistant.usage` per request, ephemeral, with a unique `apiCallId`, attributed to its phase. Never re-sent, so no
double count. The sum per Fusion equalled `fusion_completed.totalNanoAiu` exactly in all 32. A phase's `usage` equals its share. The
only durable per-request record is the disk-only `session.usage_record`. A cancelled phase reports only finished requests (critic
aborted mid-request: usage 0).

### Relative to idle

Observed. `session.fusion_commit_started` ... re-sends ... `session.fusion_completed`, [`session.model_change` when the Turn
claimed a Fusion tier], `session.usage_checkpoint`, `assistant.idle`, `session.idle`, all within milliseconds. Nothing committed
falls after idle. Exception: the s6d second Fusion, with no idle after the first.

## Surprises for a transcript that shows staged work live and settles at the commit

1. **`single` stages too.** Even a one-word answer is ephemeral first, then re-sent. Only the failed-route fallback runs live, and
   it has neither commit nor completion, so a settling path that waits for one never ends. Its Turn ends at idle.
2. Settling by ID works, and is reliable: upsert on `messageId` and `toolCallId`. A staged row of the source phase is replaced by
   its identical twin. Tool output built from `tool.shell_output` has no durable copy but `result` on the complete.
3. **Nothing retracts a losing phase.** A reader must infer discards: from `sourcePhaseId` at the commit, `fusion_handoff`
   (rejected draft), `phase_failed` (abort, and it discards earlier finished phases too), and nothing at all on a crash.
4. **After an abort or a crash, the durable history has no assistant content for the Turn** and the Model has none either. Staged
   work the user watched live vanishes on resume. What remains: the `permission.*` pairs for those Tools, and any finished phase's
   `fusion_phase_completed.content`. Workspace changes remain (`s3-max` left a rewritten `scheduler.py` with no committed Tool) with
   no durable Tool event to explain them.
5. A crashed Fusion never closes, so resume must treat an unclosed `fusion_resolved` as dead. `sessionWasActive` was `false` even so.
6. **Steers are durable at once and unattributed**, so durable order and live order disagree, and a steer in a review phase, or
   in a last request, can sit for a long while (`pending_messages.modified` is the only signal), landing in a later phase or a whole
   new Fusion.
7. **One host Turn may hold two Fusions without an idle between them** (s6d). Per-Fusion state must not be assumed to end with a Turn,
   and `fusion_completed` must not settle a Turn.
8. Judge approvals are durable, unattributed, and match no Tool row. With real approvals the user would be asked about Tools the
   transcript never shows (since seen with a person, above).
9. `assistant.turn_start` is commit-only, so a Fusion Turn has no such event until the end.
10. `turnId` repeats across phases, so it cannot key rows.
11. Empty `assistant.message` bursts need filtering, live and at the commit, or their reasoning merged.
12. `abort` during a Tool leaves a staged `tool.execution_start` with no complete.
13. `disconnect` does not cancel: `s3-max` took 37s to disconnect, and the on-disk log grew `permission.requested` after
    `session.shutdown`. Only stopping the CLI ended it.
14. `session.model_change` into a Fusion tier comes at the end of the claiming Turn; into a non-Fusion tier, at its start.
15. A resumed Session without the assignment looks healthy: `get_current` reports the tier, and only a Prompt fails.

## Could not get to happen

- `degradedReason`, `outcome` other than `completed`, `status: "failed"`, `degradedToPhaseId`: no phase failed on its own, and I
  found no cheap way to force one, since a narrowed Model list fails the route as a whole. The CLI's bundle names `critic_invalid` and
  `revision_failed`, and its strings name `primary_unavailable` and `review_unavailable` (unobserved).
- `kind: "terminal_tool"`, `stagedTerminal`, `awaitingBackgroundWork`, `followUp`, `selectedRuleId`, `projectionMode: "append"`,
  several critics, a compaction Turn. None came up.
- A committed repair on Max: the first run's rejection was cut off by a bug in the capture tool's wait limit, and the second was accepted. Not repeated, at
  100-250 credits a try; `s6i` shows a repair commit on High, with no sign Max differs.
- Abort during a repair: `s5d` aborted a revision instead.
- An unanswered approval in a phase. The denied one is the live run above, which also shows a phase failing on its own and `outcome: degraded`.
