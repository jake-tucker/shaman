# Copilot offers Fusion's routing tiers only to a Session assigned them

_No longer so on Copilot CLI 1.0.114: see [What changed by 1.0.114](#what-changed-by-10114) at the end, and ADR 0082, which supersedes ADR 0060._

Recorded 2026-11-04 against GitHub Copilot CLI 1.0.105 on macOS, driven through
github-copilot-sdk 1.0.29-preview.5, on an `individual_pro` account. ADR 0060 records what Shaman
decided from it.

## The symptom

Copilot's Fusion is the Model `auto` with routing tiers: `balance` (Standard), `hydrafusion` (High),
`hydrafusion-max` (Max) and a hidden `fast`. `session.model.list` lists them under `auto.tiers`. A
Session Shaman opens lists none, and asking for `hydrafusion` is refused with "requires dynamic Auto
tiers". Feature flags, `--experimental` and `enable_experimental_mode` change nothing.

The gate is the experiment parameter `copilot_cli_dynamic_auto_tiers`. The interactive CLI receives
it in the assignments it fetches from Copilot's experimentation service (ExP). A Session driven over
the SDK receives no assignments: `copilot --server --stdio` never contacted the service in any run
here, even with an empty CLI cache. The SDK instead lets an integrator inject assignments on create
and on resume through `exp_assignments`, "in the same JSON shape the Copilot CLI fetches".

## What the service is

| Request | Answer |
| --- | --- |
| `GET /api/v1/assignments` | 405, `Allow: POST` |
| `GET /api/v1/not-a-route` | 404 |
| `POST /api/v1/assignments` with `{}` | 400, `userParams must not be empty` |
| `POST /api/v1/assignments` with `{"userParams": {...}}` | 200 |

- The host seen is `exp.individual.githubcopilot.com`, contacted only when the CLI's cache was empty.
  It matches `endpoints.exp` in the account's `/copilot_internal/user` response, whose sibling
  endpoints name the same plan, so the host is presumably per plan. Only this plan was seen.
- The path sits beside `ExP request failed with status` in the CLI's `runtime.node`. The names of
  `userParams` sit beside each other in it too: `extensionname`, `copilottrackingid`,
  `gh_core_ghmsftorexternal`, and `github_copilotcli_` followed by `cliversion`, `audience`,
  `experimentationoptin`, `prerelease`, `firstlaunchat` and `copilotplan`.
- The host was read from a logger that records `CONNECT host:port` and tunnels without decrypting.
  No CA, no TLS termination, and no change to system trust.
- All the 200s above came with no credential. Whether the CLI sends one is not known.

The service answers in a raw form the CLI converts before handing it on:

| Raw field | `CopilotExpAssignmentResponse` |
| --- | --- |
| `assignedVariants[].name` | `Features` |
| `assignedVariants[].numberline` → `.name` | `Flights` |
| `dataVersion` | `FlightingVersion` |
| `assignmentContext` | `AssignmentContext` |
| `featureVariables["/<config>/<parameter>"]` | `Configs[].Parameters`, parsed to bool, integer or string |

## Why Shaman does not fetch

No request identifying itself honestly got the parameter:

| `userParams` | Variants | `copilot_cli_*` parameters |
| --- | --- | --- |
| `extensionname=shaman`, an invented tracking ID | 6 | 0 |
| plus the real plan and CLI version | 6 | 0 |
| plus the account's real tracking ID and the remaining CLI facts | 6 | 0 |
| the same with no `extensionname` | 6 | 0 |
| `extensionname=shaman` with a first-launch date | 6 | 0 |
| `extensionname=CopilotSDK` | 6 | 0 |
| The CLI's own request, as its cache recorded the answer | 66 | present |

A Session handed the converted form of one of those six-variant answers listed no tiers.

Not proven: that `extensionname=CopilotCLI` is what unlocks the parameter. That value was never sent,
because Shaman claiming to be the Copilot CLI was ruled out before the work began, and a single request
would settle it. The other differences from the CLI's request are headers that could not be seen,
including any `Authorization`, the values guessed for audience and opt-in, and any parameter not
found. A logged-out CLI makes no ExP request at all, so it could not separate these.

## What the assignment has to say

Listing the tiers is not the same as routing as the CLI does. The same one-word Prompt on
`hydrafusion`, all three with identical router scores:

| Assignments | planVersion | policyVersion / ruleId | Route | totalNanoAiu |
| --- | --- | --- | --- | --- |
| `dynamic_auto_tiers` only | 1 | none | `critique`, claude-sonnet-5 + gpt-5.6-sol | 4,547,725,000 |
| CLI-issued (control) | 2 | `hydrafusion_v3_2026-10-27` / `solo_luna6_simple_low_reasoning` | `single`, gpt-6-luna | 128,304,750 |
| `dynamic_auto_tiers` + `copilot_cli_hydrafusion_plan_v2` | 2 | same as control | `single`, gpt-6-luna | 125,977,250 |

So Shaman asserts exactly those two parameters, both `true`, under the Config `default`, with
`Features`, `Flights` and `AssignmentContext` empty.

## What Shaman does

The `provider.copilot.fusionTiers` Setting, in the Experimental group and off by default, governs
whether a conversation Session carries that assignment on `session.create` and `session.resume`.
With it off, neither carries the field. Errands and Skill discovery never carry it.

An assignment reaches the CLI only as a Session opens, so changing the Setting leaves an open Session
as it opened. It applies to the next Session to open and the next resume. Copilot Sessions share one
process that outlives idleness (ADR 0058), so an open Session usually resumes only after the Server
restarts, the process crashes, or its tree is evicted.

Checked against the real CLI, with an empty CLI cache and under the logger, using the bytes Shaman's
`CopilotRuntime` put on the wire with the Setting on:

- With the Setting off, `session.create` carried no assignment. With it on, it carried exactly the
  assignment above.
- That assignment listed `balance`, `hydrafusion`, `hydrafusion-max` and `fast`.
- One one-word Turn on plain `auto` with no tier answered `OK`, routed to `gpt-6-luna`, reported a
  cost of 1.0, and emitted no Fusion or Auto-tier events. `session.model.getCurrent` then answered
  `auto`.
- A later CLI process resuming that Session listed no tiers when the resume carried no assignment,
  and all four when it did. The assignment does not persist with the Session.
- `exp.*` was contacted zero times, and no `exp-cache.json` appeared in the empty cache.

## Not verified

- **Harder work.** Only one-word Turns were compared, so a hard Prompt or a multi-turn Session could
  still differ from the CLI's routing. The CLI's issued assignment also carries
  `copilot_cli_auto_intent`, `copilot_cli_auto_v2_endpoint`, `copilot_cli_hydrafusion_rollout`,
  `copilot_cli_hydrafusion_sticky_root`, `copilot_cli_hydrafusion_interrupted_commit` and
  `copilot_cli_multi_turn_context_routing`, and Shaman's does not. Their effect is untested.
- **Other plans.** One `individual_pro` account was checked, on which every tier reported itself
  enabled. What the assignment does on an account with no Fusion entitlement, or one the service
  would not have assigned it, is unknown.
- **Other CLI versions.** Nothing says these parameter names outlast 1.0.105. A name the CLI no longer
  reads would presumably just offer no tiers.
- **Turns through Shaman.** The one live Turn ran through the SDK with Shaman's bytes rather than through
  Shaman's own Session. The transcript of a Fusion Turn still has a known defect: a committed phase
  shows its rows twice, which is why the Setting is off by default.
- **Windows and Linux.** Every observation above is from macOS.

## What changed by 1.0.114

Recorded 2026-11-23 against Copilot CLI 1.0.114 on macOS, driven through copilot-sdk 2.0.1 at
protocol 4, on the same `individual_pro` account. ADR 0082 records what Shaman decided from it.

**The tiers are listed with no assignment.** `models.list`, asked with no Session, no assignment and
no feature flags, answered with `auto.tiers` in full, and a Session opened with nothing extra listed
the same from `session.model.list`. Each tier now carries the Fusion name Copilot asks to be told
it by:

| `id` | `displayName` | `fusionTier` | `type` | `pickerVisible` |
| --- | --- | --- | --- | --- |
| `balance` (the default) | Standard | `standard` | `auto` | true |
| `hydrafusion` | High | `high` | `fusion` | true |
| `hydrafusion-max` | Max | `max` | `fusion` | true |
| `fast` | Fast | none | `auto` | false |

**A tier routes the same with no assignment.** The same one-word Prompt on `hydrafusion`, once with
nothing asserted and once with the two parameters above (`copilot_auto_tier_probe hydrafusion`, the
second under `PROBE_EXP_ASSIGNMENTS`):

| Assignments | planVersion | policyVersion / ruleId | Route | totalNanoAiu |
| --- | --- | --- | --- | --- |
| none | 2 | `hydrafusion_v4_2026-11-10` / `solo_luna6_simple_low_reasoning` | `single`, gpt-6-luna | 121,969,750 |
| `dynamic_auto_tiers` + `copilot_cli_hydrafusion_plan_v2` | 2 | same | `single`, gpt-6-luna | 46,042,150 |

Both sent 13,920 input tokens. The second read 9,432 of them from the Prompt Cache the first had
just written, which is the whole of the difference in cost. `session.model.getCurrent` answered
`{"modelId":"auto","autoTier":"hydrafusion","fusionTier":"high"}` after each.

**Through Shaman, asserting nothing.** `copilot_routing_tier_live` against a Server built without
the assignment: the catalog offered Standard, High and Max 2.8s after the Setting came on, with no
Session opened to learn them. A Turn on `hydrafusion`, asked for as Fusion tier `high`, completed
with a Route on gpt-6-luna. `not-a-real-tier` and `efficiency` were each refused at once, and their
Turns ran on `hydrafusion` with a Status row. The tier survived a Server restart and resume, going
back to the default cleared it, and with the Setting off the resumed Session ran on plain Auto.

### Not verified

- **Which CLI version first did this.** 1.0.105 needed the assignment and 1.0.114 does not. Nothing
  between was checked.
- **Other plans.** The same one account. What an account with no Fusion entitlement is listed is
  unknown.
- **Harder work.** One-word Turns again.
- **Windows and Linux.** Every observation is from macOS.
