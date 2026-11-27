//! Fake data for where a Session begins: what a Server's Workspace Registry would list, one Repository's Worktrees,
//! and the directories a Directory Browser would read. Nothing here is read from anywhere.

/// A Workspace as the Workspace Picker lists it. The first four stand where `sessions::WORKSPACES` has them.
pub struct Place {
    pub name: &'static str,
    pub icon: Option<&'static str>,
    /// Its presented root, beneath the directory a machine keeps its work in.
    pub root: &'static str,
    pub description: &'static str,
    /// Whether it is a Repository; one that is not has no Worktree to choose.
    pub repository: bool,
    /// A bare Repository owes a Worktree choice before a Session can begin.
    pub bare: bool,
    /// Why its Server cannot read it now, where it cannot.
    pub unavailable: Option<&'static str>,
    /// Whether a Remote lists it too.
    pub remote: bool,
}

pub const SIDEKICK: usize = 3;

pub static PLACES: [Place; 7] = [
    Place {
        name: "shaman",
        icon: Some("md-flask"),
        root: "shaman",
        description: "The Server, the TUI, the Relay and the phone's core. Rust, one workspace of crates.",
        repository: true,
        bare: false,
        unavailable: None,
        remote: true,
    },
    Place {
        name: "rune-experiments",
        icon: Some("md-test_tube"),
        root: "rune-experiments",
        description: "Scratch crates for trying Rune's renderer and kit.",
        repository: true,
        bare: false,
        unavailable: None,
        remote: true,
    },
    Place {
        name: "dotfiles",
        icon: None,
        root: "dotfiles",
        description: "",
        repository: false,
        bare: false,
        unavailable: None,
        remote: false,
    },
    Place {
        name: "Sidekick",
        icon: Some("md-robot"),
        root: "",
        description: "",
        repository: false,
        bare: false,
        unavailable: None,
        remote: false,
    },
    Place {
        name: "copilot-sdk",
        icon: None,
        root: "references/copilot-sdk",
        description: "GitHub's Copilot SDK, kept for reading.",
        repository: true,
        bare: false,
        unavailable: None,
        remote: true,
    },
    Place {
        name: "notes.git",
        icon: None,
        root: "notes.git",
        description: "",
        repository: true,
        bare: true,
        unavailable: None,
        remote: false,
    },
    Place {
        name: "thesis",
        icon: Some("md-book_open_variant"),
        root: "/Volumes/Archive/thesis",
        description: "",
        repository: true,
        bare: false,
        unavailable: Some("/Volumes/Archive is not mounted"),
        remote: false,
    },
];

/// The directory a machine keeps its work in, as a path field names it.
pub fn home(outlook: Option<usize>) -> &'static str {
    match outlook {
        None => "~/Projects",
        Some(_) => "/home/jake/src",
    }
}

/// A Workspace's presented root on the machine the Outlook is turned toward.
pub fn root(at: usize, outlook: Option<usize>) -> String {
    let p = &PLACES[at];
    if p.root.starts_with('/') {
        p.root.to_owned()
    } else if p.name == "dotfiles" {
        "~/dotfiles".to_owned()
    } else {
        format!("{}/{}", home(outlook), p.root)
    }
}

/// The Workspaces a Workspace Picker offers under an Outlook, narrowed to names carrying `query`: never the Sidekick
/// Workspace, save as the reader's current one, which stands first as any current Workspace does.
pub fn offered(outlook: Option<usize>, current: usize, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut all: Vec<usize> = (0..PLACES.len())
        .filter(|i| *i != SIDEKICK || current == SIDEKICK)
        .filter(|i| outlook.is_none() || PLACES[*i].remote)
        .filter(|i| query.is_empty() || PLACES[*i].name.to_lowercase().contains(&query))
        .collect();
    all.sort_by_key(|i| *i != current);
    all
}

/// One of a Repository's Worktrees, as the Worktree Selector lists it.
pub struct Worktree {
    /// The branch, or the commit where it is detached.
    pub checkout: &'static str,
    pub linked: bool,
    pub detached: bool,
    /// Where it is, as the Selector says it: the leaf name alone for one Shaman manages, the path beneath the
    /// Workspace's main root for one within it, and the whole path otherwise.
    pub location: &'static str,
    pub managed: bool,
    /// Its Checkout State cannot be read.
    pub unavailable: bool,
}

/// The Worktrees of the one Repository that has several.
pub static WORKTREES: [Worktree; 6] = [
    Worktree {
        checkout: "main",
        linked: false,
        detached: false,
        location: "main working copy",
        managed: false,
        unavailable: false,
    },
    Worktree {
        checkout: "feature/desktop-app-full-client",
        linked: true,
        detached: false,
        location: ".worktrees/desktop-app-full-client",
        managed: false,
        unavailable: false,
    },
    Worktree {
        checkout: "shaman/relay-sqlite",
        linked: true,
        detached: false,
        location: "relay-sqlite-4c1a",
        managed: true,
        unavailable: false,
    },
    Worktree {
        checkout: "release/0.4.3",
        linked: true,
        detached: false,
        location: "~/Projects/shaman-release",
        managed: false,
        unavailable: false,
    },
    Worktree {
        checkout: "a7b6fe3",
        linked: true,
        detached: true,
        location: "keymap-chords-9d2e",
        managed: true,
        unavailable: false,
    },
    Worktree {
        checkout: "windows-paths",
        linked: true,
        detached: false,
        location: "/Volumes/Scratch/shaman-windows",
        managed: false,
        unavailable: true,
    },
];

/// Every other Repository has its main working copy alone.
static MAIN_ONLY: [Worktree; 1] = [Worktree {
    checkout: "main",
    linked: false,
    detached: false,
    location: "main working copy",
    managed: false,
    unavailable: false,
}];

pub fn worktrees(place: usize) -> &'static [Worktree] {
    let p = &PLACES[place];
    if !p.repository || p.bare {
        &[]
    } else if place == 0 {
        &WORKTREES
    } else {
        &MAIN_ONLY
    }
}

/// A directory a Directory Browser lists, by its path beneath the machine's work directory.
pub struct Dir {
    pub path: &'static str,
    /// The Workspace it is the root of, where it is one.
    pub place: Option<usize>,
    /// Dot-named, so left out until the reader asks for hidden directories.
    pub hidden: bool,
}

const fn dir(path: &'static str) -> Dir {
    Dir {
        path,
        place: None,
        hidden: false,
    }
}

/// Every directory there is to browse, parents before children.
pub static DIRS: [Dir; 21] = [
    Dir {
        path: "notes.git",
        place: Some(5),
        hidden: false,
    },
    Dir {
        path: "rune-experiments",
        place: Some(1),
        hidden: false,
    },
    dir("rune-experiments/atlas"),
    dir("rune-experiments/upload"),
    Dir {
        path: "shaman",
        place: Some(0),
        hidden: false,
    },
    Dir {
        path: "shaman/.github",
        place: None,
        hidden: true,
    },
    dir("shaman/crates"),
    dir("shaman/crates/shaman-client"),
    dir("shaman/crates/shaman-desktop"),
    dir("shaman/crates/shaman-desktop/src"),
    dir("shaman/crates/shaman-icons"),
    dir("shaman/crates/shaman-protocol"),
    dir("shaman/crates/shaman-thin-client"),
    dir("shaman/docs"),
    dir("shaman/docs/adr"),
    dir("shaman/docs/agents"),
    dir("shaman/ios"),
    dir("shaman/scripts"),
    dir("shaman/src"),
    dir("shaman/src/tui"),
    dir("scratch"),
];

pub fn leaf(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, leaf)| leaf)
}

pub fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// Whether `path` has children to open onto.
pub fn has_children(path: &str, hidden: bool) -> bool {
    DIRS.iter()
        .any(|d| parent(d.path) == path && (hidden || !d.hidden))
}

/// The rows a tree rooted at `root` draws beneath it: each directory whose every ancestor below the root is open,
/// with how deep it stands. `partial` narrows the root's own children to those whose name begins with it.
pub fn rows(
    root: &str,
    open: &std::collections::HashSet<&'static str>,
    hidden: bool,
    partial: &str,
) -> Vec<(&'static Dir, usize)> {
    let partial = partial.to_lowercase();
    DIRS.iter()
        .filter(|d| hidden || !d.hidden)
        .filter_map(|d| {
            let below = if root.is_empty() {
                d.path
            } else {
                d.path.strip_prefix(root)?.strip_prefix('/')?
            };
            let depth = below.matches('/').count();
            if depth == 0 && !leaf(d.path).to_lowercase().starts_with(&partial) {
                return None;
            }
            // Every ancestor between the root and this directory must stand open.
            let mut at = parent(d.path);
            while at != root {
                if !open.contains(at) {
                    return None;
                }
                at = parent(at);
            }
            Some((d, depth))
        })
        .collect()
}

/// The Workspace a directory belongs to and its path beneath that Workspace's root, where it has one.
pub fn belongs(path: &str) -> Option<(usize, Option<&str>)> {
    DIRS.iter()
        .filter_map(|d| d.place.map(|p| (d.path, p)))
        .find_map(|(root, place)| {
            if path == root {
                Some((place, None))
            } else {
                path.strip_prefix(root)?
                    .strip_prefix('/')
                    .map(|below| (place, Some(below)))
            }
        })
}

/// What a Prompt gives a Session for a Title before any agent has named it: its first line, cut short.
pub fn title_of(prompt: &str) -> String {
    let line = prompt.lines().next().unwrap_or("").trim();
    let mut title: String = line.chars().take(48).collect();
    if line.chars().count() > 48 {
        title.push('…');
    }
    title
}

/// The branch Shaman would name a new Worktree's after a Prompt.
pub fn branch_of(prompt: &str) -> String {
    let words: Vec<String> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(3)
        .map(str::to_lowercase)
        .collect();
    format!("shaman/{}", words.join("-"))
}
