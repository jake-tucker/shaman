# Copilot offers Fusion's routing tiers only to a Session assigned them

Recorded 2026-11-04 against GitHub Copilot CLI 1.0.105 on macOS, driven through
github-copilot-sdk 1.0.29-preview.5, on an `individual_pro` account. ADR 0060 records what Suru
decided from it.

## The symptom

Copilot's Fusion is the Model `auto` with routing tiers: `balance` (Standard), `hydrafusion` (High),
`hydrafusion-max` (Max) and a hidden `fast`. `session.model.list` lists them under `auto.tiers`. A
Session Suru opens lists none, and asking for `hydrafusion` is refused with "requires dynamic Auto
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

## Why Suru does not fetch

No request identifying itself honestly got the parameter:

| `userParams` | Variants | `copilot_cli_*` parameters |
| --- | --- | --- |
| `extensionname=suru`, an invented tracking ID | 6 | 0 |
| plus the real plan and CLI version | 6 | 0 |
| plus the account's real tracking ID and the remaining CLI facts | 6 | 0 |
| the same with no `extensionname` | 6 | 0 |
| `extensionname=suru` with a first-launch date | 6 | 0 |
| `extensionname=CopilotSDK` | 6 | 0 |
| The CLI's own request, as its cache recorded the answer | 66 | present |

A Session handed the converted form of one of those six-variant answers listed no tiers.

Not proven: that `extensionname=CopilotCLI` is what unlocks the parameter. That value was never sent,
because Suru claiming to be the Copilot CLI was ruled out before the work began, and a single request
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

So Suru asserts exactly those two parameters, both `true`, under the Config `default`, with
`Features`, `Flights` and `AssignmentContext` empty.

## What Suru does

The `provider.copilot.fusionTiers` Setting, in the Experimental group and off by default, governs
whether a conversation Session carries that assignment on `session.create` and `session.resume`.
With it off, neither carries the field. Errands and Skill discovery never carry it.

An assignment reaches the CLI only as a Session opens, so changing the Setting leaves an open Session
as it opened. It applies to the next Session to open and the next resume. Copilot Sessions share one
process that outlives idleness (ADR 0058), so an open Session usually resumes only after the Server
restarts, the process crashes, or its tree is evicted.

Checked against the real CLI, with an empty CLI cache and under the logger, using the bytes Suru's
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
  `copilot_cli_multi_turn_context_routing`, and Suru's does not. Their effect is untested.
- **Other plans.** One `individual_pro` account was checked, on which every tier reported itself
  enabled. What the assignment does on an account with no Fusion entitlement, or one the service
  would not have assigned it, is unknown.
- **Other CLI versions.** Nothing says these parameter names outlast 1.0.105. A name the CLI no longer
  reads would presumably just offer no tiers.
- **Turns through Suru.** The one live Turn ran through the SDK with Suru's bytes rather than through
  Suru's own Session. The transcript of a Fusion Turn still has a known defect: a committed phase
  shows its rows twice, which is why the Setting is off by default.
- **Windows and Linux.** Every observation above is from macOS.
