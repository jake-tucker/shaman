# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root, or
- **`CONTEXT-MAP.md`** at the repo root if it exists — it points at one `CONTEXT.md` per context. Read each one relevant to the topic.
- **`docs/adr/`** — read ADRs that touch the area you're about to work in. In multi-context repos, also check `src/<context>/docs/adr/` for context-scoped decisions.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily when terms or decisions actually get resolved.

## File structure

Single-context repo (most repos):

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-event-sourced-orders.md
│   └── 0002-postgres-for-write-model.md
└── src/
```

Multi-context repo (presence of `CONTEXT-MAP.md` at the root):

```
/
├── CONTEXT-MAP.md
├── docs/adr/                          ← system-wide decisions
└── src/
    ├── ordering/
    │   ├── CONTEXT.md
    │   └── docs/adr/                  ← context-specific decisions
    └── billing/
        ├── CONTEXT.md
        └── docs/adr/
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal — either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Words for what a Client presents

The TUI and a thin Client present the same work differently. `CONTEXT.md` handles that by one rule, which holds when reading a presentation entry and when writing one:

- **A term named for what a presentation is for covers every Client.** Landing, Provisional Session, Workspace Picker, Worktree Selector, Working Indicator, Intervention and Turn Fold are such terms. The entry's opening sentence holds for every Client, the detail after it describes the TUI unless it says otherwise, and a trailing "A thin Client…" clause says what differs.
- **A term named for a shape or a place on the TUI's screen stays the TUI's.** Sidebar, Aside, Section, Subagents Section and Subagent Picker are such terms. Where a thin Client shows something in its place, the entry gains a clause saying so in ordinary words: a thin Client's "listing of Sessions", the "sheet" it offers a tree in. Do not coin a term for the thin Client's counterpart.
- **A thin Client clause is short.** One or two sentences, stating only a difference that follows from being a thin Client: no keys, no Server of its own, one screen at a time. What a release leaves out for now belongs in its spec, not in the glossary.

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders) — but worth reopening because…_
