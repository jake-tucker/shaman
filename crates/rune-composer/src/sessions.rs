//! Fake data for the Sidebar: what a Server and two Remotes would list. Nothing here is read from anywhere.

#[derive(Clone, Copy, PartialEq)]
pub enum Standing {
    NeedsIntervention,
    /// How long the work has been running.
    Working(&'static str),
    Failed,
    /// How long it has been waiting.
    Monitoring(&'static str),
    Done,
    /// Nothing to say: the slot holds the compact time since the Session last moved.
    Quiet,
}

pub struct Session {
    pub id: &'static str,
    pub title: &'static str,
    /// An Icon Catalog name, or none.
    pub icon: Option<&'static str>,
    pub workspace: usize,
    /// Its Origin where that is a Remote.
    pub remote: Option<usize>,
    /// The Checkout State's branch or commit; none for a directory outside source control.
    pub checkout: Option<&'static str>,
    pub linked: bool,
    pub detached: bool,
    pub standing: Standing,
    pub settled: bool,
    /// The compact time since it last moved.
    pub age: &'static str,
    pub unreadable: bool,
}

pub struct Workspace {
    pub name: &'static str,
    pub icon: Option<&'static str>,
}

pub struct Remote {
    pub name: &'static str,
    pub unreachable: bool,
}

pub const WORKSPACES: [Workspace; 4] = [
    Workspace {
        name: "shaman",
        icon: Some("md-flask"),
    },
    Workspace {
        name: "rune-experiments",
        icon: Some("md-test_tube"),
    },
    Workspace {
        name: "dotfiles",
        icon: None,
    },
    Workspace {
        name: "Sidekick",
        icon: Some("md-robot"),
    },
];

pub const REMOTES: [Remote; 2] = [
    Remote {
        name: "studio",
        unreachable: false,
    },
    Remote {
        name: "mini",
        unreachable: true,
    },
];

const fn s(
    id: &'static str,
    title: &'static str,
    icon: &'static str,
    workspace: usize,
    checkout: Option<&'static str>,
    standing: Standing,
    age: &'static str,
) -> Session {
    Session {
        id,
        title,
        icon: Some(icon),
        workspace,
        remote: None,
        checkout,
        linked: false,
        detached: false,
        standing,
        settled: false,
        age,
        unreadable: false,
    }
}

const fn settled(
    id: &'static str,
    title: &'static str,
    icon: &'static str,
    workspace: usize,
    age: &'static str,
) -> Session {
    Session {
        settled: true,
        ..s(
            id,
            title,
            icon,
            workspace,
            Some("main"),
            Standing::Quiet,
            age,
        )
    }
}

pub static SESSIONS: [Session; 24] = [
    Session {
        linked: true,
        ..s(
            "a1",
            "Draw the Transcript's virtual rows",
            "md-window_restore",
            0,
            Some("feature/desktop-app-full-client"),
            Standing::Working("4m 12s"),
            "now",
        )
    },
    Session {
        linked: true,
        ..s(
            "a2",
            "Migrate relay storage to SQLite",
            "md-database",
            0,
            Some("relay-sqlite"),
            Standing::NeedsIntervention,
            "2m",
        )
    },
    s(
        "a3",
        "Chase the flaky pairing test",
        "md-bug",
        0,
        Some("main"),
        Standing::Failed,
        "9m",
    ),
    Session {
        remote: Some(0),
        ..s(
            "a4",
            "Benchmark wgpu upload paths",
            "md-speedometer",
            1,
            Some("main"),
            Standing::Working("41s"),
            "now",
        )
    },
    s(
        "a5",
        "Watch CI on the release branch",
        "md-rocket_launch",
        0,
        Some("release/0.4.3"),
        Standing::Monitoring("18m"),
        "18m",
    ),
    s(
        "a6",
        "Sketch the app's icon set",
        "md-palette",
        2,
        None,
        Standing::Done,
        "31m",
    ),
    s(
        "a7",
        "Summarise yesterday's Sessions",
        "md-robot",
        3,
        None,
        Standing::Quiet,
        "2h",
    ),
    Session {
        detached: true,
        ..s(
            "a8",
            "Port the keymap to chords",
            "md-keyboard",
            0,
            Some("a7b6fe3"),
            Standing::Quiet,
            "5h",
        )
    },
    Session {
        remote: Some(1),
        linked: true,
        ..s(
            "a9",
            "Audit path fixtures for Windows",
            "md-shield_check",
            0,
            Some("windows-paths"),
            Standing::Quiet,
            "1d",
        )
    },
    Session {
        unreadable: true,
        icon: None,
        ..s(
            "a10",
            "Thread from an older Codex",
            "md-bug",
            0,
            Some("main"),
            Standing::Quiet,
            "3d",
        )
    },
    settled(
        "z1",
        "Offer routing tiers as Copilot lists them",
        "md-puzzle",
        0,
        "1d",
    ),
    settled(
        "z2",
        "Speak protocol 4 on copilot-sdk 2",
        "md-wrench",
        0,
        "1d",
    ),
    settled(
        "z3",
        "Say what a thin Client's Aside holds",
        "md-book_open_variant",
        0,
        "2d",
    ),
    settled("z4", "Try glyph atlases at 2x", "md-test_tube", 1, "2d"),
    settled("z5", "Cut release v0.4.2", "md-rocket_launch", 0, "3d"),
    settled(
        "z6",
        "Explain the Keeper's login item",
        "md-key_variant",
        0,
        "3d",
    ),
    settled("z7", "Tidy zsh startup files", "md-wrench", 2, "4d"),
    settled(
        "z8",
        "Record the two-executable decision",
        "md-file_document_edit",
        0,
        "4d",
    ),
    settled(
        "z9",
        "Find where notarization staples",
        "md-shield_lock",
        0,
        "5d",
    ),
    settled("z10", "Profile markdown reflow", "md-speedometer", 1, "6d"),
    settled(
        "z11",
        "Draft the pairing wire schema",
        "md-file_document",
        0,
        "1w",
    ),
    settled(
        "z12",
        "Shimmer for the Working Indicator",
        "md-brush",
        0,
        "1w",
    ),
    settled(
        "z13",
        "Questionnaire keyboard on the Duo",
        "md-cellphone",
        0,
        "2w",
    ),
    settled("z14", "Seed the Icon Catalog", "md-palette", 0, "3w"),
];

/// What a listing ranges over.
#[derive(Clone, Copy, PartialEq)]
pub enum Scope {
    Everywhere,
    /// Every Workspace of the Outlook.
    All,
    Workspace(usize),
}

/// The Sessions a scope lists while the Outlook is turned toward `outlook` (none: this machine's own Server), narrowed
/// to Titles carrying `query`.
pub fn listed(scope: Scope, outlook: Option<usize>, query: &str) -> Vec<&'static Session> {
    let query = query.trim().to_lowercase();
    SESSIONS
        .iter()
        .filter(|x| match scope {
            Scope::Everywhere => true,
            Scope::All => x.remote == outlook,
            Scope::Workspace(w) => x.remote == outlook && x.workspace == w,
        })
        .filter(|x| query.is_empty() || x.title.to_lowercase().contains(&query))
        .collect()
}

pub fn session(id: &str) -> Option<&'static Session> {
    SESSIONS.iter().find(|x| x.id == id)
}

/// The name a machine goes by: a Remote's, or this one's.
pub fn machine(outlook: Option<usize>) -> &'static str {
    outlook.map_or("This Mac", |r| REMOTES[r].name)
}
