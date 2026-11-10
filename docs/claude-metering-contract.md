# Claude metering contract

## Claude Code CLI 2.1.280

Re-verified on 2026-09-27 against the installed `2.1.280 (Claude Code)`, the suggested version, with live Haiku Turns in stream-json mode and the Agent SDK 0.3.280 type definitions. Where this section and the 2.1.237 record below disagree, this section is current.

- **A resumed conversation continues its running total.** One process ran two Turns (`total_cost_usd` 0.0121764, then 0.0160821) and exited when stdin closed. A fresh process launched with `--resume` for the same native Session ID reported **0.0192634** on its first Turn, and its `modelUsage` carried every earlier Turn's tokens as well. The 2.1.277 changelog describes the change ("headless sessions now save their totals at exit"), and the SDK's `total_cost_usd` documentation now reads "a resumed or forked session continues from the total its transcript saved". The `result_index` field restarts at 0 in each process.
- **The total is saved only when the process exits cleanly.** A process that ran one Turn (0.0218079) and was then killed with `SIGKILL` saved nothing. The next resume continued from the previous clean exit's 0.0192634, so its first result (0.0218171) lacked the killed process's spend. Suru stops a child by closing its stdin and forces it down only after the exit grace, so the normal path saves the total.
- **`/clear` now works in stream-json and begins a new conversation.** Sent as a user message, it produced a `conversation_reset` message, a `system/init` naming a different `session_id`, and a zero-usage result whose `total_cost_usd` was **0** under that new `session_id`.
- **Thinking is broken out of output.** `result.usage.output_tokens` counts thinking, and `result.usage.output_tokens_details.thinking_tokens` states how much of it was thinking (for example, 46 output tokens with 39 of thinking). `modelUsage[*].thinkingTokens` carries the cumulative figure.
- **Unchanged from 2.1.237:** `result.usage` covers only the loop's own API calls for that result, while `total_cost_usd` and `modelUsage` are cumulative and include Subagents. A spawned Subagent's (Haiku) tokens appeared only in `modelUsage` and the dollar total. Assistant snapshots still carry placeholder `output_tokens` (6 in this capture) and only establish Model identity.

A brokered Subagent runs in a Provider process of its own, so its Cost is never in the `total_cost_usd` of the Session that delegated to it, even when both run on Claude.

Suru reads these as follows. A Claude result's cumulative Cost is reported in the reporting lifetime `claude:<session_id>`, keyed by the conversation the result names. That lifetime runs through every process that resumes the conversation, including processes started for a Selection change or after a Suru restart, so earlier spend is never counted twice. A `/clear` starts a new lifetime from zero. A total that goes back within one lifetime, such as after a killed process, is not believed: the Turn's Cost is left partial rather than lowering the recorded total. Result thinking tokens become Usage `reasoning_tokens` and are subtracted from `output_tokens`. Suru's Resume State still names the conversation the Session was spawned under, so a `/clear` followed by a respawn resumes the pre-clear conversation. That is a known gap.

### How Suru reads `modelUsage`

Suru reads a result's `modelUsage` for the Turn's Usage and its Model Shares (ADR 0071). What rests on what:

- **Observed on 2.1.280** (above): `modelUsage` is cumulative over the conversation, continues across a clean resume, includes Subagents' tokens and dollars, and carries a cumulative `thinkingTokens`. A killed process saves nothing, so the next resume reports lower totals.
- **From the CLI's own schema** (the 2.1.310 bundle, not measured): each entry is keyed by the raw model string its calls ran on, and `canonicalModel` is the "canonical model id used for the pricing lookup", which "may differ from the raw model string this entry is keyed by (provider-specific ids, aliases)". The bundle's model ids include 1M-context forms such as `claude-opus-4-6[1m]`. How often a key differs from its canonical id in practice has not been measured.
- **Suru's reading rules**, below, follow from those two.

From each entry Suru reads `inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`, `thinkingTokens`, `costUSD` and `canonicalModel`. The figures are running totals in the reporting lifetime `claude:<session_id>`. Suru keeps the highest totals of each entry, by its raw key, in the Session's Resume State. That way a process resuming the conversation after a Selection change, an idle stop or a Suru restart is measured against the totals reported before it. Only the lifetimes a result can still report into are kept: the conversation the Session is filed under, and the one its process last reported in. Totals Suru cannot read back from the Resume State are read as none rather than failing the Session.

- **A Share is what a result adds, under the canonical Model.** Each entry's increase over the highest its lifetime reported before is work on the Model its `canonicalModel` names. Entries naming one canonical Model make one Share. Its Usage is what they added, and its Cost is their running `costUSD` summed. An entry from a CLI that states no `canonicalModel` is named by its key with the 1M-context tier set aside, as the CLI names Models in assistant messages. A Model none of whose entries moved holds no Share. Thinking becomes `reasoning_tokens` and is taken out of output, as for `result.usage`. An entry without `thinkingTokens` leaves reasoning unknown and output as stated.
- **The Turn's Usage is the sum of its Shares**, so it includes the native Subagents' calls, as `total_cost_usd` does. A Turn answered by several results sums what each adds.
- **A Share's Cost is that Model's running `costUSD`**, on the same terms as the Turn's running Cost: Cost Coverage takes each Share's increment within the lifetime. The Turn's Cost is still `total_cost_usd`.
- **A total gone back is not believed.** The whole `modelUsage` of a result is not believed if any entry falls below the highest reported before it in the lifetime, in any figure or by leaving out an entry reported before. This holds even where other entries rose. A lower entry means the totals were restored from an earlier save, such as on the resume after a killed process, and the rises are then measured against figures the restored totals never held. A result with no `modelUsage`, or one Suru cannot read whole, is not believed either, and Suru traces the unreadable one.
  - Such a result's Usage is its own `result.usage`, with no Share. It stays under the Turn's Agent Selection, and its native Subagents' tokens go uncounted. A result closing a `/compact` loop is the exception: its `result.usage` is zero because the summarising is the CLI's own call, so it states no Usage at all.
  - The highest totals are still raised to whatever it reported, as Cost Coverage keeps a lifetime's highest Cost. A later result is believed again once every entry is back at or above them, and it counts only what lies past them.
  - A CLI that does not restore the totals on resume, like 2.1.237 below, reads the same way: each resumed process falls back to `result.usage` until its totals pass the highest reported before.
- **Per-Model Cost can skew after an unbelieved result.** The unbelieved result's Turn holds no Share, so Cost Coverage takes the next believed Share's Cost increment over the last *recorded* Share of that Model. That record may predate the unbelieved Turn, so the increment can include Cost the unbelieved Turn spent, while that Turn's Cost increment sits in its unattributed remainder. The Turn and lifetime totals, and the non-negativity of every part, still hold. Only the split between that Model and the Selection's remainder is off.
- **A `/clear` starts afresh.** The cleared conversation is a new lifetime, whose totals start from zero, so its first entries are its own work rather than totals gone back.
- **Context Fill is unaffected.** It is read from the CLI's `get_context_usage` answers, not from `modelUsage`.

Suru reads an assistant message's `model` for Message attribution, not for Cost. It names the concrete Model that wrote the message, which is how Suru learns what an alias such as `default` resolved to, or what the CLI fell back to mid-Turn. Suru reads it from the loop's own `message_start` where the stream carries it there, and otherwise from the snapshot restating each block. An Agent Message of the loop's own conversation written on a Model other than its Agent Selection's carries that Model as its Message attribution; one written on the Selection's own Model carries none. The CLI names the Model without the 1M-context tier, so a Selection such as `claude-opus-5-5[1m]` that ran `claude-opus-5-5` ran its own Model. The evidence holds for its assistant message alone, so a Message streamed before its snapshot arrives is attributed once the snapshot names the Model. The CLI's `<synthetic>` placeholder, written on messages no Model produced, such as an API error, is not a Model. A native Subagent's `model` keeps its own handling: it names the Subagent's row and Turn.

### How Suru reads the Prompt Cache

Claude states no Cache Lifetime, so Suru infers it from the Session's own requests (#624). What rests on what:

- **Observed on 2.1.283.** The captures behind docs/validation/0407-claude-folded-steer.md ran with `--include-partial-messages`, as Suru does, and saw a `stream_event` `message_start` open each request of the loop's own conversation. The committed fixtures (`tests/claude_integration/fixtures/steer-mid-run.jsonl`, `steer-late-restart.jsonl`) keep the assistant snapshots and drop the stream events. Each snapshot's `usage` carries `cache_creation_input_tokens` and its breakdown, `cache_creation.ephemeral_5m_input_tokens` and `cache_creation.ephemeral_1h_input_tokens`, beside the placeholder `output_tokens: 1` that a `message_start` opens with. On that claude.ai account the writes were all for one hour (`ephemeral_1h_input_tokens: 5487`, `ephemeral_5m_input_tokens: 0`).
- **From the API's documented shape, not measured on the CLI.** A `message_start` carries the request's `usage` with the same cache fields, which the snapshots restate. No capture records a `message_start`'s `usage` itself.
- **Suru's reading rules**, below, follow from those two.

From each `message_start` of the loop's own conversation (`parent_tool_use_id` null), Suru reads:

- **What the request wrote, and for how long.** The request wrote for the shortest lifetime it wrote any tokens for, since that part lapses first. Tokens for five minutes are written at the default write price, and tokens for an hour at the hour-long one. The price is read off the field the tokens are in, never off how long the lifetime is. Tokens counted in `cache_creation_input_tokens` that no lifetime in the breakdown claims, where the breakdown is absent or names none, were written for five minutes, the only lifetime an API breaking out none offers.
- **A request that wrote nothing.** A follow-up served from the cache says nothing of the lifetime. Suru keeps the write last observed for the Session, its lifetime and its price, which is stored with the lapse moment. That write outlives an idle stop and a Server restart. A Session where no moment stands assumes five minutes.
- **A `message_start` with no `usage`, or one Suru cannot read.** This is no evidence of a request, and moves nothing.
- **A Turn a stopped Server left open.** What its requests stated died with the process that read them, so the next start settles it by Claude's declaration alone: it stated no request Suru still knows of, so the earlier moment stands, whatever other Models its Usage names. A completed Compaction it requested still leaves no moment standing.
- **The Model.** The request runs on the Model the loop's `system/init` names unless its `message.model` names another one, a fallback. Only then is the cache recorded as written by that Model rather than the Selection's. An `init` naming no Model leaves every request on the Selection, since an alias could not be told from a fallback.
- **When the lifetime starts.** Anthropic measures a cache entry's lifetime from the start of the request that writes or reads it, so generation time counts against it, and a read refreshes the timer. Suru stamps the moment when the Provider actor reads the request's `message_start`, on the Server's clock. That stamp is later than the request's start by the API's time to first token and whatever the stamp waited for, so the lapse Suru publishes errs late by that much (seconds on a large context), never by the time spent generating.

What this cannot tell apart, and why it does not need to:

- **A native Subagent's requests.** Its conversation never streams, so no `message_start` of its own reaches Suru. Its cache is its own and moves nothing of the Session's. Its Models still appear in the Turn's Model Shares (ADR 0071), so for Claude those Shares are not read as the conversation running elsewhere.
- **`result.usage` and `modelUsage`.** These are totals over the loop and the conversation, including the Subagents. They do not say which request wrote for which lifetime, so neither is read for the cache.
- **A `/compact`.** The summarising call is the CLI's own. The 2.1.283 capture (docs/validation/0462-claude-manual-compaction.md) shows no assistant message for it, but it ran without partial messages, so whether that call streams a `message_start` is unverified. Either way, a completed Compaction with no request after it leaves no moment standing, because the cache any request before it wrote holds the old context.

**The hour-long write price.** models.dev lists one `cache_write` rate for Anthropic's Models: the five-minute one, 1.25× input. It lists no hour-long rate (checked 2026-11-11). Claude's runtime prices a write for an hour at twice the Model's fresh input rate, the rule Anthropic publishes in its prompt-caching pricing (read 2026-11-11: one-hour writes at 2× base input, five-minute ones at 1.25×). The rule applies only to a Model the table gives a five-minute write rate. An hour-long rate the table comes to list for a Model wins over the rule. Nothing else is derived: the five-minute write and the read are exactly what the table says, and a Model the table does not price stays unpriced.

## Claude Code CLI 2.1.237

Verified against the then-suggested Claude Code CLI **2.1.237** while implementing issue [#324](https://github.com/suru-ai/suru/issues/324). The published `@anthropic-ai/claude-code-linux-x64@2.1.237` binary ran in stream-json input/output mode against a local HTTP fixture serving deterministic Anthropic message streams. These measurements exercise the real CLI's accounting with synthetic API usage; they do not measure billing.

### Successive results and process lifetimes

Two user messages were sent sequentially through one running CLI process. The fixture supplied uncached Haiku input/output counts, with no tool calls:

| API response | Input tokens | Output tokens | CLI `result.usage` input/output | CLI `total_cost_usd` |
| --- | ---: | ---: | --- | ---: |
| First | 100 | 5 | 100 / 5 | 0.000125 |
| Second | 200 | 7 | 200 / 7 | 0.000360 |

The second `modelUsage` carried 300 input and 12 output tokens. The dollar total is cumulative within the running process; adding both result totals would count the first call twice. A fresh process launched with `--resume` for the same native Session ID reported 0.000125 and then 0.000360 for the same synthetic responses: the resumed conversation does not restore the previous process's metering total.

Sending `/clear` in this tested stream-json environment returned an unavailable-command result with zero Turn usage and the unchanged cumulative Cost. It did not establish a new accounting lifetime. Current documentation describes reset messages in environments that support these commands; a repeated or lower amount alone must not be mistaken for proof of a reset.

### Descendants and Model identity

The fixture made the first parent response invoke `Agent`, targeting a custom Subagent configured with a different Model. The CLI emitted `task_started` with the spawning `tool_use_id`, followed by a child assistant snapshot carrying that identity as `parent_tool_use_id` and the actual child Model in `message.model`.

| Work | Model | Input tokens | Output tokens | CLI model Cost |
| --- | --- | ---: | ---: | ---: |
| Parent, two API responses | claude-haiku-4-5-20251001 | 400 | 12 | 0.000460 |
| Child, one API response | claude-sonnet-5 | 200 | 7 | 0.000705 |

The result's `usage` contained only the parent's 400 input and 12 output tokens. Its `total_cost_usd` was **0.001165**, including the child, and `modelUsage` contained both Models. Both parent and child assistant snapshots carried `output_tokens: 1`, the message-start placeholder, despite the final API stream reporting larger counts. Such snapshots establish Model identity but cannot establish complete child Usage or Cost.

The observed CLI tool was named `Agent`; older Suru fixtures used `Task`. The task lifecycle's `tool_use_id` is the stable attribution link, not the tool's display name.

### Source and limits

These observations agree with the official [streaming-input cost documentation](https://code.claude.com/docs/en/agent-sdk/cost-tracking#track-costs-in-streaming-input-mode) and its discussion of [output-token placeholders](https://code.claude.com/docs/en/agent-sdk/cost-tracking#read-output-tokens-from-the-result-message). A direct inference probe was blocked by the account's session limit; the deterministic local fixture allowed the pinned CLI's metering behavior to be verified without inference. Other CLI versions and supported reset notifications remain separate contracts to validate when the suggested version changes.
