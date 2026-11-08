# Count a Claude Turn's native Subagents in its Usage

A Claude Turn's Usage was read from `result.usage`, which covers only the main loop's own API calls, while its Cost was read from `total_cost_usd`, which includes the native Subagents Claude ran inside its process (`docs/claude-metering-contract.md`). Splitting a Turn into Model Shares needs Usage and Cost to cover the same work, and the only per-Model figures Claude gives — `modelUsage`, cumulative over the conversation's reporting lifetime and keyed by canonical Model id — include those Subagents in both tokens and dollars. We decided a Claude Turn's Usage is the increment of `modelUsage` over the highest reported before it in that lifetime, summed across Models, so it includes its native Subagents as its Cost already does, and each Model's increment is that Turn's Share for it. Claude reports no split by Subagent, so none is invented: the tokens stay on the Turn that spawned them, as the Cost does.

## Considered Options

- **Split only Cost by `modelUsage`, keeping Usage from `result.usage`.** Rejected: a Subagent's Model would hold a Share of Cost with no tokens, and the Model breakdown's token figures would not add up to the Turns' Usage.
- **Keep Usage as it was and accept that the Actual view's tokens differ from the Selected view's.** Rejected: the Provider and Workspace breakdowns, and the two Model views, must total the same work.

## Consequences

- Every Claude Session's token figures rise wherever it ran native Subagents, on every surface that shows Usage, not only in Usage Metrics. Context Fill is unaffected.
- A `modelUsage` total that goes back within a lifetime, such as after a killed process, is not believed, as for Cost: that Turn's Usage falls back to its `result.usage`, held under its Agent Selection with no Share, so its native Subagents' tokens go uncounted rather than guessed.
