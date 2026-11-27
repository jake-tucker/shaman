//! Fake data: the tree of Sessions an open Session belongs to. One a top-level Session heads, with Subagents two
//! deep, and one a Sidekick's Session heads, with its Subsessions and the Sessions it acted on. Nothing here is read
//! from anywhere.

use crate::talk::{Activity, Kind, Outcome};

#[derive(Clone, Copy, PartialEq)]
pub enum Sort {
    /// The top-level Session that heads the tree.
    Top,
    /// A Subagent's Session.
    Subagent,
    /// A Session a Sidekick began or acted on.
    Session,
}

pub struct Node {
    pub id: &'static str,
    pub parent: Option<&'static str>,
    pub sort: Sort,
    pub title: &'static str,
    /// A Subagent's name.
    pub name: &'static str,
    /// The Model the Provider confirmed for a Subagent, or the Model a Session's Agent Selection names.
    pub model: &'static str,
    /// A top-level Session's Provider.
    pub provider: &'static str,
    /// Where a Session works: a Remote, where its Origin is one, and its Workspace among `sessions::WORKSPACES`.
    pub remote: Option<&'static str>,
    pub workspace: usize,
    pub outcome: Outcome,
    pub time: &'static str,
    /// Whether it may be stopped on its own: a brokered Subagent always, a native one where its Provider allows.
    pub stoppable: bool,
    /// What its entry says in place of everything current, where its Remote does not answer.
    pub lost: bool,
    /// A Subsession, which the Sidekick began, as against a Session it only acted on.
    pub begun: bool,
    /// What it was asked: a Subagent's Delegation, a Subsession's first Prompt.
    pub asked: &'static str,
    /// Its latest Message.
    pub said: &'static str,
    pub work: &'static [Activity],
}

const fn act(id: &'static str, kind: Kind, head: &'static str, slot: &'static str) -> Activity {
    Activity {
        id,
        kind,
        head,
        slot,
        outcome: Outcome::Worked,
        body: "",
        after: "",
    }
}

const fn sub(
    id: &'static str,
    parent: &'static str,
    name: &'static str,
    model: &'static str,
    title: &'static str,
    outcome: Outcome,
    time: &'static str,
) -> Node {
    Node {
        id,
        parent: Some(parent),
        sort: Sort::Subagent,
        title,
        name,
        model,
        provider: "",
        remote: None,
        workspace: 0,
        outcome,
        time,
        stoppable: true,
        lost: false,
        begun: false,
        asked: "",
        said: "",
        work: &[],
    }
}

/// The top-level Session every tree here hangs from but the Sidekick's.
pub const TOP: &str = "a1";
/// The Sidekick's Session.
pub const SIDEKICK: &str = "a7";
/// The Subagent that owes an Approval in the scenes that show one.
pub const ASKING: &str = "s4";

/// The tree `a1` heads, in the order the Subagents Section lists it: a branch with work in it ahead of the settled
/// ones, and within each the most recently spawned first.
pub static TREE: [Node; 6] = [
    Node {
        id: TOP,
        parent: None,
        sort: Sort::Top,
        title: "Draw the Transcript's virtual rows",
        name: "",
        model: "Fable 5.2",
        provider: "Claude",
        remote: None,
        workspace: 0,
        outcome: Outcome::Working,
        time: "2m 06s",
        stoppable: false,
        lost: false,
        begun: false,
        asked: "",
        said: "",
        work: &[],
    },
    Node {
        asked: "Measure how long a reflow of the Transcript takes at 400 Messages, in a release build, with the window 760 pt and 1,200 pt wide. Report p50 and p95 per frame.",
        said: "The harness is in `benches/reflow.rs`. At 760 pt the first run is in; I've handed the glyph atlas upload to a Subagent, since it dominates the first frame, and am running the 1,200 pt pass now.",
        work: &[
            act(
                "s2-a",
                Kind::Read,
                "crates/shaman-desktop/benches/reflow.rs",
                "1–64",
            ),
            act(
                "s2-b",
                Kind::Edit,
                "crates/shaman-desktop/benches/reflow.rs",
                "+41 −3",
            ),
            Activity {
                outcome: Outcome::Working,
                ..act(
                    "s2-c",
                    Kind::Command,
                    "cargo bench -p shaman-desktop --bench reflow -- --width 1200",
                    "21s",
                )
            },
        ],
        ..sub(
            "s2",
            TOP,
            "bench",
            "Sonnet 5.5",
            "Measure reflow at 400 Messages",
            Outcome::Working,
            "58s",
        )
    },
    Node {
        asked: "Profile the glyph atlas upload on the first frame of the reflow bench and say whether it can be warmed before the window opens.",
        said: "The upload is one 2,048 px texture written whole. I want to run the bench with the atlas warmed to compare.",
        work: &[
            act(
                "s4-a",
                Kind::Read,
                "crates/shaman-desktop/src/atlas.rs",
                "1–120",
            ),
            act(
                "s4-b",
                Kind::Tool,
                "grep  write_texture  crates/",
                "6 matches",
            ),
        ],
        ..sub(
            "s4",
            "s2",
            "profile",
            "GPT-6 Codex",
            "Profile the glyph atlas upload",
            Outcome::Working,
            "21s",
        )
    },
    Node {
        asked: "Read how Rune's `virtual_rows` decides which rows to build, and what it does when a row's height is not what was estimated. Report the file and the lines.",
        said: "`virtual_rows` builds the rows whose estimated span meets the viewport plus an overscan, then corrects the estimate from each built row's measured height (`el/virtual_list.rs:412`). A row that measures taller shifts the rows after it, and with `append_only` the anchor is the foot, so nothing in view moves.",
        work: &[
            act(
                "s1-a",
                Kind::Tool,
                "grep  virtual_rows  ~/.cargo/git/checkouts/rune-*/",
                "14 matches",
            ),
            act("s1-b", Kind::Read, "rune/src/el/virtual_list.rs", "380–470"),
            act("s1-c", Kind::Read, "rune/src/el/virtual_list.rs", "700–760"),
        ],
        ..sub(
            "s1",
            TOP,
            "explore",
            "Haiku 5.5",
            "Read Rune's virtual_rows",
            Outcome::Worked,
            "1m 40s",
        )
    },
    Node {
        asked: "Find what Rune's virtual list uses for overscan when none is given.",
        said: "Half a viewport on each side: `OVERSCAN_RATIO` is `0.5` at `el/virtual_list.rs:31`.",
        work: &[act(
            "s3-a",
            Kind::Tool,
            "grep  OVERSCAN  rune/src/el/",
            "3 matches",
        )],
        ..sub(
            "s3",
            "s1",
            "explore",
            "Haiku 5.5",
            "Find the overscan defaults",
            Outcome::Worked,
            "22s",
        )
    },
    Node {
        asked: "Review the stick-to-bottom rule in `State::landed` for a reader who scrolls back while a Message streams.",
        said: "",
        work: &[act(
            "s5-a",
            Kind::Read,
            "crates/shaman-desktop/src/state.rs",
            "200–260",
        )],
        stoppable: false,
        ..sub(
            "s5",
            TOP,
            "review",
            "Opus 5.5",
            "Review the stick-to-bottom rule",
            Outcome::Failed,
            "12s",
        )
    },
];

const fn session(
    id: &'static str,
    title: &'static str,
    provider: &'static str,
    model: &'static str,
    workspace: usize,
    outcome: Outcome,
    time: &'static str,
) -> Node {
    Node {
        id,
        parent: Some(SIDEKICK),
        sort: Sort::Session,
        title,
        name: "",
        model,
        provider,
        remote: None,
        workspace,
        outcome,
        time,
        stoppable: true,
        lost: false,
        begun: false,
        asked: "",
        said: "",
        work: &[],
    }
}

/// The tree a Sidekick's Session heads: its own Subagents, then its Subsessions and every other Session it acted on
/// in one order, those working ahead of those not.
pub static SIDEKICK_TREE: [Node; 8] = [
    Node {
        id: SIDEKICK,
        parent: None,
        sort: Sort::Top,
        title: "Summarise yesterday's Sessions",
        name: "",
        model: "Fable 5.2",
        provider: "Claude",
        remote: None,
        workspace: 3,
        outcome: Outcome::Worked,
        time: "1m 12s",
        stoppable: false,
        lost: false,
        begun: false,
        asked: "",
        said: "",
        work: &[],
    },
    Node {
        asked: "List every Session that settled yesterday on this Server and on studio, with its Workspace and how it ended.",
        said: "Eleven Sessions settled yesterday: nine here and two on studio. One failed, in `shaman`.",
        work: &[act(
            "k1-a",
            Kind::Tool,
            "list_sessions  since=yesterday",
            "11 sessions",
        )],
        ..sub(
            "k1",
            SIDEKICK,
            "explore",
            "Haiku 5.5",
            "Read the Session index",
            Outcome::Worked,
            "14s",
        )
    },
    Node {
        begun: true,
        asked: "Write the release notes for 0.4.3 from the pull requests merged since v0.4.2, grouped as the last notes were.",
        ..session(
            "k2",
            "Write the release notes for 0.4.3",
            "Claude",
            "Opus 5.5",
            0,
            Outcome::Working,
            "2m 10s",
        )
    },
    Node {
        asked: "Collect the titles of every pull request merged since v0.4.2.",
        said: "Twenty-three pull requests, listed by area.",
        work: &[act(
            "k2a-a",
            Kind::Command,
            "gh pr list --state merged --search 'merged:>2026-11-12'",
            "2s",
        )],
        ..sub(
            "k2a",
            "k2",
            "explore",
            "Haiku 5.5",
            "Collect merged pull request titles",
            Outcome::Worked,
            "31s",
        )
    },
    Node {
        remote: Some("studio"),
        ..session(
            "a4",
            "Benchmark wgpu upload paths",
            "Claude",
            "Sonnet 5.5",
            1,
            Outcome::Working,
            "41s",
        )
    },
    // A Session the Sidekick only acted on: it sent the Prompt this Session now waits on its reader over.
    session(
        "a2",
        "Migrate relay storage to SQLite",
        "Codex",
        "GPT-6 Codex",
        0,
        Outcome::Working,
        "Needs intervention",
    ),
    Node {
        begun: true,
        ..session(
            "k6",
            "Triage the overnight CI failures",
            "Copilot",
            "GPT-6",
            0,
            Outcome::Worked,
            "6m 02s",
        )
    },
    Node {
        remote: Some("mini"),
        lost: true,
        stoppable: false,
        ..session(
            "k5",
            "Rebuild the docs index",
            "",
            "",
            2,
            Outcome::Worked,
            "",
        )
    },
];

/// The tree the Session `id` belongs to; empty for a fake Session that has spawned nothing, which heads a tree of
/// its own alone.
pub fn tree(id: &str) -> &'static [Node] {
    // A Session a Sidekick only acted on heads its own tree, with no Sidekick above it.
    let under_sidekick = |n: &Node| n.id == id && (n.sort != Sort::Session || n.begun);
    if TREE.iter().any(|n| n.id == id) {
        &TREE
    } else if SIDEKICK_TREE.iter().any(under_sidekick) {
        &SIDEKICK_TREE
    } else {
        &[]
    }
}

pub fn node(id: &str) -> Option<&'static Node> {
    TREE.iter().chain(SIDEKICK_TREE.iter()).find(|n| n.id == id)
}

/// How many Sessions stand between `id` and the head of its tree.
pub fn depth(id: &str) -> usize {
    let mut depth = 0;
    let mut at = node(id);
    while let Some(parent) = at.and_then(|n| n.parent) {
        depth += 1;
        at = node(parent);
    }
    depth
}

/// The way down to `id` from the head of its tree, both ends included.
pub fn path(id: &str) -> Vec<&'static Node> {
    let mut path = Vec::new();
    let mut at = node(id);
    while let Some(n) = at {
        path.push(n);
        at = n.parent.and_then(node);
    }
    path.reverse();
    path
}
