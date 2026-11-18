//! Beginning a Session where the Outlook stands: the Landing a phone shows
//! as a screen pushed from the listing, and the Provisional Session a first
//! Prompt turns it into.
//!
//! The Landing names where a Session begun now begins, the Workspace the
//! Outlook stands in on its Remote, and whether send is offered there: not
//! where no Workspace is remembered for that Remote, and not while it is
//! Unreachable, when the composer takes a draft and nothing begins. Its
//! composer keeps a draft of its own.
//!
//! Sending begins the Session on that Workspace's main Worktree, under an
//! identity the core chose, and opens it at once as the Provisional Session:
//! the Prompt as a bubble, its first line as the Title, and a bar reading
//! "Starting" with no elapsed time and no Stop, until the Server answers with
//! the Session it made, which replaces it in place. The beginning goes on
//! whatever becomes of the screen, so the Session appears in the listing
//! however soon it is left. A refusal stands as a row where the work would
//! be, with "Try again", and a Prompt written instead replaces the refused
//! one; leaving it discards it and keeps its text as the Landing's draft.

use std::path::PathBuf;

use shaman_protocol::{
    CreateSessionRequest, ExecutionDirectory, InitialPrompt, SessionId, SessionSnapshot,
};

use crate::{device::RememberedWorkspace, remotes::BeginsIn};

/// The key the Landing's own draft is kept under, which no Session's
/// reference can be.
pub(crate) const LANDING_DRAFT: &str = "new-session";

/// What the Landing's navigation bar reads.
const TITLE: &str = "New session";

/// What the empty composer of the Landing says.
const PLACEHOLDER: &str = "Message";

/// The Landing: the screen a Session is begun from, as a phone pushes it from
/// the listing, whole on every change.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Landing {
    /// What the navigation bar reads: "New session".
    pub title: String,
    /// Where it will begin, beneath the title: the Remote, and the Workspace
    /// where one is remembered for it.
    pub place: String,
    /// The Workspace it will begin in, as the middle of the screen names it,
    /// where one is remembered for the Outlook's Remote.
    pub workspace: Option<LandingWorkspace>,
    /// Said where no Workspace is remembered for the Outlook's Remote,
    /// asking for one.
    pub asks: Option<String>,
    /// Said above the composer while the Outlook's Remote is Unreachable,
    /// when the composer takes a draft and nothing begins.
    pub unreachable: Option<String>,
    /// What the empty composer says.
    pub placeholder: String,
    /// Whether send is offered: only where a Workspace is remembered and its
    /// Remote answers.
    pub can_send: bool,
    /// The key the composer's draft is kept under, as Swift hands it back to
    /// read and write the draft.
    pub draft: String,
}

/// The Workspace a Session begun from the Landing begins in, as the middle
/// of the screen names it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LandingWorkspace {
    /// The glyph of its Icon, where it has one this build knows.
    pub glyph: Option<String>,
    pub name: String,
    /// The Remote it is on.
    pub origin: String,
    /// Its Description, where it has one.
    pub description: Option<String>,
}

/// What the listing knows of the Workspace the Outlook stands in on its
/// Remote, which the Landing is drawn from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Known {
    /// The glyph of its Icon, where its Remote lists one this build knows.
    pub glyph: Option<String>,
    /// Its Description, where its Remote lists one.
    pub description: Option<String>,
    /// Whether its Remote reads Unreachable.
    pub unreachable: bool,
}

/// The Landing where the Outlook stands at the Remote `remote`, in the
/// Workspace `workspace` remembered for it, as the listing knows it; or
/// where it stands nowhere.
pub(crate) fn landing(at: Option<(&str, Option<&RememberedWorkspace>)>, known: &Known) -> Landing {
    let Some((remote, workspace)) = at else {
        return Landing {
            title: TITLE.to_owned(),
            place: String::new(),
            workspace: None,
            asks: Some("Pair with a server to begin a session there".to_owned()),
            unreachable: None,
            placeholder: PLACEHOLDER.to_owned(),
            can_send: false,
            draft: LANDING_DRAFT.to_owned(),
        };
    };
    let place = BeginsIn::remote(remote, workspace.map(|workspace| workspace.name.as_str())).place;
    let unreachable = known
        .unreachable
        .then(|| format!("{remote} is unreachable. Nothing begins there until it answers."));
    Landing {
        title: TITLE.to_owned(),
        place,
        workspace: workspace.map(|workspace| LandingWorkspace {
            glyph: known.glyph.clone(),
            name: workspace.name.clone(),
            origin: remote.to_owned(),
            description: known.description.clone(),
        }),
        asks: workspace.is_none().then(|| asks(remote)),
        can_send: workspace.is_some() && unreachable.is_none(),
        unreachable,
        placeholder: PLACEHOLDER.to_owned(),
        draft: LANDING_DRAFT.to_owned(),
    }
}

/// What the Landing says where no Workspace is remembered for the Remote
/// `remote`, as after a first pairing.
pub(crate) fn asks(remote: &str) -> String {
    format!(
        "Choose a workspace on {remote} to begin in: narrow the listing to one, or open a \
         session there."
    )
}

/// A Session being begun from the Landing, until its Server answers with
/// the Session it made: what an open Session's screen holds of its own in
/// the meantime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Provisional {
    /// The identity the core chose for it.
    pub session_id: SessionId,
    /// Where it begins: the Workspace's main Worktree.
    pub directory: PathBuf,
    /// Its first Prompt.
    pub prompt: InitialPrompt,
    /// Where it stands, beneath its Title, as its frame names it.
    pub place: String,
    /// Why its Server refused it, once it has.
    pub refused: Option<String>,
    /// The Session its Server made, once it answers, which replaces it.
    pub created: Option<Box<SessionSnapshot>>,
}

impl Provisional {
    /// The Session `session_id` begun at `directory` with the Prompt `text`,
    /// standing at `place`.
    pub fn new(session_id: SessionId, directory: PathBuf, text: String, place: String) -> Self {
        Self {
            session_id,
            directory,
            prompt: prompt(text),
            place,
            refused: None,
            created: None,
        }
    }

    /// The request that begins it.
    pub fn request(&self) -> CreateSessionRequest {
        CreateSessionRequest {
            preparation_id: None,
            // The Server's own Agent Selection: a phone chooses none.
            agent_selection: None,
            execution_directory: ExecutionDirectory {
                path: self.directory.clone(),
            },
            prompt: self.prompt.clone(),
            session_id: Some(self.session_id),
        }
    }

    /// Its Title: the first line of its Prompt.
    pub fn title(&self) -> String {
        self.prompt
            .text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_owned()
    }

    /// Replaces its refused Prompt with `text`, and asks again.
    pub fn replace(&mut self, text: String) {
        self.prompt = prompt(text);
        self.refused = None;
    }
}

/// `text` as a first Prompt, under an identity of its own.
fn prompt(text: String) -> InitialPrompt {
    InitialPrompt {
        id: shaman_protocol::PromptId::new(),
        text,
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
    }
}

/// What the row a refusal stands as says: why, after the words the TUI
/// leads its row with.
pub(crate) fn refused_words(why: &str) -> String {
    format!("Could not create session: {why}")
}

/// Said where a beginning's answer never came.
pub(crate) const MAYBE_BEGUN: &str = "The server did not answer, so the session may have begun";

#[cfg(test)]
mod tests {
    use super::*;

    fn remembered(name: &str) -> RememberedWorkspace {
        RememberedWorkspace {
            path: PathBuf::from(name),
            name: name.to_owned(),
        }
    }

    #[test]
    fn the_landing_names_where_it_begins_and_offers_send_there() {
        let workspace = remembered("shaman");
        let known = Known {
            glyph: Some("\u{e7a8}".to_owned()),
            description: Some("A terminal client for coding agents".to_owned()),
            unreachable: false,
        };
        let landing = landing(Some(("desk", Some(&workspace))), &known);
        assert_eq!(landing.title, "New session");
        assert_eq!(landing.place, "desk · shaman");
        assert_eq!(
            landing.workspace,
            Some(LandingWorkspace {
                glyph: Some("\u{e7a8}".to_owned()),
                name: "shaman".to_owned(),
                origin: "desk".to_owned(),
                description: Some("A terminal client for coding agents".to_owned()),
            })
        );
        assert_eq!(landing.asks, None);
        assert_eq!(landing.unreachable, None);
        assert!(landing.can_send);
        assert_eq!(landing.draft, LANDING_DRAFT);
    }

    #[test]
    fn on_an_unreachable_remote_nothing_begins_and_send_is_not_offered() {
        let workspace = remembered("shaman");
        let landing = landing(
            Some(("desk", Some(&workspace))),
            &Known {
                unreachable: true,
                ..Known::default()
            },
        );
        assert_eq!(
            landing.unreachable.as_deref(),
            Some("desk is unreachable. Nothing begins there until it answers.")
        );
        assert!(!landing.can_send);
        assert!(landing.workspace.is_some(), "it still says where");
    }

    #[test]
    fn where_no_workspace_is_remembered_the_landing_asks_for_one_and_offers_no_send() {
        let landing = landing(Some(("desk", None)), &Known::default());
        assert_eq!(landing.place, "desk");
        assert_eq!(landing.workspace, None);
        assert_eq!(landing.asks, Some(asks("desk")));
        assert!(!landing.can_send);
    }

    #[test]
    fn a_provisional_session_is_titled_by_the_first_line_of_its_prompt() {
        let provisional = Provisional::new(
            SessionId::new(),
            PathBuf::from("shaman"),
            "\n  Draw the invite as a QR code  \nin half-blocks".to_owned(),
            "shaman".to_owned(),
        );
        assert_eq!(provisional.title(), "Draw the invite as a QR code");
        let request = provisional.request();
        assert_eq!(request.session_id, Some(provisional.session_id));
        assert_eq!(request.agent_selection, None, "a phone chooses no Agent");
        assert_eq!(request.preparation_id, None, "on the main Worktree");
        assert_eq!(request.execution_directory.path, PathBuf::from("shaman"));
    }

    #[test]
    fn a_prompt_written_instead_replaces_the_refused_one_under_an_identity_of_its_own() {
        let mut provisional = Provisional::new(
            SessionId::new(),
            PathBuf::from("shaman"),
            "Draw the invite".to_owned(),
            "shaman".to_owned(),
        );
        let first = provisional.prompt.id;
        provisional.refused = Some("no".to_owned());
        provisional.replace("Trim the quiet zone".to_owned());
        assert_eq!(provisional.prompt.text, "Trim the quiet zone");
        assert_ne!(provisional.prompt.id, first);
        assert_eq!(provisional.refused, None);
    }
}
