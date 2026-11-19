//! The Workspace Picker a phone pushes from the Landing's "Begin somewhere
//! else": one searchable list of every Remote's Workspaces, under a header
//! for each Remote where the phone holds more than one.
//!
//! A row is a Workspace's Icon, name and Description, ticked where the
//! Outlook stands in it. One its Server cannot read is dimmed with the
//! wire's reason and cannot be chosen; an Unreachable Remote's header says
//! so, its Workspaces still choosable, since choosing only turns the Outlook.
//! A Server's Sidekick Workspace is never offered, and there is no Browse
//! row. Choosing a Workspace turns the Outlook toward it, and leaves the
//! listing's scope where it was.

use std::path::Path;

use crate::listing::{Answered, Scope, icon_glyph};

/// What the picker's navigation bar reads.
const TITLE: &str = "Begin in";

/// What its search field says while it is empty.
const SEARCH_PROMPT: &str = "Search workspaces";

/// Said under a Remote whose Workspaces have yet to be listed.
pub(crate) const LOADING: &str = "Loading workspaces…";

/// Said under a Remote that lists none.
pub(crate) const NONE: &str = "No workspaces";

/// Said where a search finds nothing.
pub(crate) const FOUND_NOTHING: &str = "No workspaces found";

/// The Workspace Picker, whole on every change.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WorkspacePicker {
    pub title: String,
    pub search_prompt: String,
    /// Each Remote's Workspaces, in the order the Remotes were paired; while a
    /// search is typed, only those it finds, and only the Remotes they are on
    /// or that have yet to list theirs.
    pub sections: Vec<WorkspaceSection>,
    /// Said in place of every section where a search finds nothing, once
    /// every Remote has listed its Workspaces.
    pub empty: Option<String>,
}

/// One Remote's Workspaces, as the picker lists them.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WorkspaceSection {
    /// The Remote, where the phone holds more than one.
    pub header: Option<String>,
    /// Said beside the header where the Remote reads Unreachable, or in its
    /// place where there is none.
    pub unreachable: Option<String>,
    pub rows: Vec<WorkspaceRow>,
    /// Said where the Remote has no rows to show: its Workspaces have yet to
    /// be listed, or it lists none.
    pub line: Option<String>,
}

/// One Workspace the picker offers.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WorkspaceRow {
    /// The Workspace, as the core is handed it back to choose it.
    pub workspace: String,
    /// The glyph of its Icon, where it has one this build knows.
    pub glyph: Option<String>,
    pub name: String,
    /// Its Description, where it has one.
    pub description: Option<String>,
    /// Whether the Outlook stands in it.
    pub chosen: bool,
    /// Why it cannot be chosen, where its Server cannot read it: the row is
    /// dimmed and takes no tap.
    pub unavailable: Option<String>,
}

/// The picker over `remotes`, the Outlook standing at the Remote and in the
/// Workspace `standing` names, searched for `query`.
pub(crate) fn present(
    remotes: &[Answered],
    standing: Option<(&str, &Path)>,
    query: &str,
) -> WorkspacePicker {
    let origin_named = remotes.len() > 1;
    let query = query.trim().to_lowercase();
    let searching = !query.is_empty();
    let sections = remotes
        .iter()
        .filter_map(|remote| {
            let rows = remote
                .offered_workspaces()
                .into_iter()
                .filter(|workspace| {
                    !searching
                        || remote
                            .paths
                            .workspace_name(workspace)
                            .to_lowercase()
                            .contains(&query)
                })
                .map(|workspace| WorkspaceRow {
                    workspace: Scope::Workspace {
                        fingerprint: remote.fingerprint.clone(),
                        workspace: workspace.id.clone(),
                    }
                    .reference(),
                    glyph: icon_glyph(workspace.icon.as_deref()),
                    name: remote.paths.workspace_name(workspace),
                    description: workspace
                        .description
                        .as_ref()
                        .map(|description| description.text.trim().to_owned())
                        .filter(|text| !text.is_empty()),
                    chosen: standing
                        .is_some_and(|(name, path)| name == remote.name && workspace.path == path),
                    unavailable: workspace
                        .unavailable
                        .map(|why| format!("Unavailable: {}", why.reason())),
                })
                .collect::<Vec<_>>();
            // A search goes on saying which Remotes have yet to list theirs,
            // so it says it found nothing only once every one has.
            let loading = remote.workspaces.is_none();
            if searching && rows.is_empty() && !loading {
                return None;
            }
            let line = if loading && (rows.is_empty() || searching) {
                Some(LOADING.to_owned())
            } else {
                rows.is_empty().then(|| NONE.to_owned())
            };
            Some(WorkspaceSection {
                header: origin_named.then(|| remote.name.clone()),
                unreachable: remote.unreached.is_some().then(|| {
                    if origin_named {
                        "Unreachable"
                    } else {
                        "The server is unreachable"
                    }
                    .to_owned()
                }),
                rows,
                line,
            })
        })
        .collect::<Vec<_>>();
    WorkspacePicker {
        title: TITLE.to_owned(),
        search_prompt: SEARCH_PROMPT.to_owned(),
        empty: (searching && sections.is_empty()).then(|| FOUND_NOTHING.to_owned()),
        sections,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use shaman_protocol::{Workspace, WorkspaceDescription, WorkspaceUnavailability};

    use super::*;
    use crate::listing::tests::{remote, session};

    fn root() -> PathBuf {
        Path::new(if cfg!(windows) { r"C:\" } else { "/" }).to_owned()
    }

    fn listed(name: &str) -> Workspace {
        Workspace::directory(root().join("work").join(name))
    }

    fn described(name: &str, description: &str) -> Workspace {
        let mut workspace = listed(name);
        workspace.description = Some(WorkspaceDescription {
            text: description.to_owned(),
            set: true,
        });
        workspace
    }

    fn listing(name: &str, workspaces: Vec<Workspace>) -> Answered {
        let mut answered = remote(name, Vec::new());
        answered.workspaces = Some(workspaces);
        answered
    }

    fn names(section: &WorkspaceSection) -> Vec<&str> {
        section.rows.iter().map(|row| row.name.as_str()).collect()
    }

    #[test]
    fn every_remotes_workspaces_stand_under_its_header_the_outlooks_ticked() {
        let desk = listing(
            "desk",
            vec![
                described("shaman", "A terminal client for coding agents"),
                listed("dotfiles"),
            ],
        );
        let lab = listing("lab", vec![listed("home-assistant")]);
        let shaman = root().join("work").join("shaman");
        let picker = present(&[desk, lab], Some(("desk", &shaman)), "");
        assert_eq!(picker.title, "Begin in");
        assert_eq!(picker.search_prompt, "Search workspaces");
        assert_eq!(picker.empty, None);
        let [desk, lab] = picker.sections.as_slice() else {
            panic!("a section for each Remote: {picker:?}");
        };
        assert_eq!(desk.header.as_deref(), Some("desk"));
        assert_eq!(names(desk), ["shaman", "dotfiles"]);
        assert_eq!(
            desk.rows[0].description.as_deref(),
            Some("A terminal client for coding agents")
        );
        assert!(desk.rows[0].chosen, "the Outlook's Workspace is ticked");
        assert!(!desk.rows[1].chosen);
        assert_eq!(lab.header.as_deref(), Some("lab"));
        assert!(!lab.rows[0].chosen, "the same name elsewhere is not ticked");
        assert_eq!(
            Scope::of(&desk.rows[1].workspace),
            Scope::Workspace {
                fingerprint: "desk-key".to_owned(),
                workspace: listed("dotfiles").id,
            },
            "the row carries the Workspace to choose"
        );
    }

    #[test]
    fn with_one_remote_no_header_stands() {
        let picker = present(&[listing("desk", vec![listed("shaman")])], None, "");
        assert_eq!(picker.sections.len(), 1);
        assert_eq!(picker.sections[0].header, None);
        assert_eq!(picker.sections[0].unreachable, None);
    }

    #[test]
    fn a_workspace_its_server_cannot_read_is_dimmed_with_the_reason() {
        let mut gone = listed("photo-archive");
        gone.unavailable = Some(WorkspaceUnavailability::Gone);
        let picker = present(&[listing("desk", vec![gone])], None, "");
        assert_eq!(
            picker.sections[0].rows[0].unavailable.as_deref(),
            Some("Unavailable: its directory is gone")
        );
    }

    #[test]
    fn the_sidekick_workspace_is_never_offered_and_sessions_workspaces_are() {
        let mut desk = listing("desk", vec![listed("shaman")]);
        let sidekick = Workspace::directory(root().join("sidekick").join("workspace"));
        desk.paths = desk.paths.clone().with_sidekick_workspace(&sidekick.path);
        desk.workspaces.as_mut().unwrap().push(sidekick);
        desk.sessions = Some(vec![crate::listing::tests::listed_summary(session(
            "Tidy", "dotfiles", 1, 1,
        ))]);
        let picker = present(&[desk], None, "");
        assert_eq!(names(&picker.sections[0]), ["shaman", "dotfiles"]);
    }

    #[test]
    fn an_unreachable_remotes_header_says_so_and_its_workspaces_stay() {
        let mut lab = listing("lab", vec![listed("home-assistant")]);
        lab.unreached = Some(crate::reach::Unreached::NoWay);
        let picker = present(&[listing("desk", Vec::new()), lab.clone()], None, "");
        assert_eq!(
            picker.sections[1].unreachable.as_deref(),
            Some("Unreachable")
        );
        assert_eq!(names(&picker.sections[1]), ["home-assistant"]);
        let alone = present(&[lab], None, "");
        assert_eq!(
            alone.sections[0].unreachable.as_deref(),
            Some("The server is unreachable")
        );
    }

    #[test]
    fn a_remote_yet_to_list_its_workspaces_is_loading_and_one_with_none_says_so() {
        let unread = remote("lab", Vec::new());
        let picker = present(&[listing("desk", Vec::new()), unread], None, "");
        assert_eq!(picker.sections[0].line.as_deref(), Some("No workspaces"));
        assert_eq!(
            picker.sections[1].line.as_deref(),
            Some("Loading workspaces…")
        );
        assert!(
            picker
                .sections
                .iter()
                .all(|section| section.rows.is_empty())
        );
    }

    #[test]
    fn a_search_narrows_by_name_and_says_where_it_finds_nothing() {
        let remotes = [
            listing("desk", vec![listed("shaman"), listed("dotfiles")]),
            listing("lab", vec![listed("home-assistant")]),
        ];
        let picker = present(&remotes, None, " DOT ");
        assert_eq!(picker.sections.len(), 1, "only where it finds any");
        assert_eq!(names(&picker.sections[0]), ["dotfiles"]);
        assert_eq!(picker.empty, None);

        let nothing = present(&remotes, None, "kernel");
        assert!(nothing.sections.is_empty());
        assert_eq!(nothing.empty.as_deref(), Some("No workspaces found"));
    }

    #[test]
    fn a_search_says_which_remotes_have_yet_to_list_theirs_and_never_that_it_found_nothing() {
        let remotes = [
            listing("desk", vec![listed("shaman"), listed("dotfiles")]),
            remote("laptop", Vec::new()),
        ];
        let picker = present(&remotes, None, "kernel");
        assert_eq!(picker.empty, None, "the laptop may yet list one");
        let [laptop] = picker.sections.as_slice() else {
            panic!("the laptop stands, loading: {picker:?}");
        };
        assert_eq!(laptop.header.as_deref(), Some("laptop"));
        assert_eq!(laptop.line.as_deref(), Some("Loading workspaces…"));

        let found = present(&remotes, None, "dot");
        assert_eq!(names(&found.sections[0]), ["dotfiles"]);
        assert_eq!(
            found.sections[1].line.as_deref(),
            Some("Loading workspaces…")
        );
    }
}
