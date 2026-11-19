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
//! however soon it is left.
//!
//! The Server answers a repeat of the same creation — the same Session id
//! and the same Prompt id — with the Session it made, so a creation whose
//! answer was lost is asked again; and whatever else becomes of one, the
//! Server is asked whether it holds the Session after all, and the screen
//! turns into it where it does. One whose answer never came stands as having
//! maybe begun, the screen watching for the Session meanwhile, and is never
//! said to have been refused. A refusal stands as a row where the work would
//! be, with "Try again", and a Prompt written instead replaces the refused
//! one; where the Session turns out to have begun with the first, the one
//! written instead goes back to the composer, saying so. Leaving either
//! discards it and keeps its text as the Landing's draft, the Landing saying
//! where the Session may have begun all the same.

use std::path::PathBuf;

use shaman_protocol::{
    AgentSelection, CreateSessionRequest, ExecutionDirectory, InitialPrompt, ModelCatalog,
    ModelOptionKind, ModelOptionValue, NewSessionAgent, SessionId, SessionSnapshot,
};

use crate::device::RememberedWorkspace;

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
    /// Where it will begin, beneath the title: the Workspace, where one is
    /// remembered for the Outlook's Remote, preceded by that Remote where the
    /// phone holds more than one; empty where that says nothing.
    pub place: String,
    /// The Workspace it will begin in, as the middle of the screen names it,
    /// where one is remembered for the Outlook's Remote.
    pub workspace: Option<LandingWorkspace>,
    /// Said where no Workspace is remembered for the Outlook's Remote,
    /// asking for one.
    pub asks: Option<String>,
    /// The Agent a Session begun here would get, shown and not chosen, as its
    /// Server's health last said: "Agent unavailable" where it says none,
    /// and none where it has yet to say, or is too old to.
    pub agent: Option<String>,
    /// Said above the composer while the Outlook's Remote is Unreachable,
    /// when the composer takes a draft and nothing begins.
    pub unreachable: Option<String>,
    /// Said beneath the composer where a Session begun from here may have
    /// begun though its Server never answered, its Prompt back in the field.
    pub note: Option<String>,
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
    /// Whether the phone holds more than one Remote, and so names Origins.
    pub origin_named: bool,
    /// The Agent a Session begun there would get, in words.
    pub agent: Option<String>,
}

/// The Landing where the Outlook stands at the Remote `remote`, in the
/// Workspace `workspace` remembered for it, as the listing knows it, saying
/// `note` beneath its composer; or where it stands nowhere.
pub(crate) fn landing(
    at: Option<(&str, Option<&RememberedWorkspace>)>,
    known: &Known,
    note: Option<String>,
) -> Landing {
    let Some((remote, workspace)) = at else {
        return Landing {
            title: TITLE.to_owned(),
            place: String::new(),
            workspace: None,
            asks: Some("Pair with a server to begin a session there".to_owned()),
            agent: None,
            unreachable: None,
            note,
            placeholder: PLACEHOLDER.to_owned(),
            can_send: false,
            draft: LANDING_DRAFT.to_owned(),
        };
    };
    // With one Remote, naming it says nothing.
    let place = known
        .origin_named
        .then_some(remote)
        .into_iter()
        .chain(workspace.map(|workspace| workspace.name.as_str()))
        .collect::<Vec<_>>()
        .join(" · ");
    let named = if known.origin_named {
        remote.to_owned()
    } else {
        "The server".to_owned()
    };
    let unreachable = known
        .unreachable
        .then(|| format!("{named} is unreachable. Nothing begins there until it answers."));
    Landing {
        title: TITLE.to_owned(),
        place,
        workspace: workspace.map(|workspace| LandingWorkspace {
            glyph: known.glyph.clone(),
            name: workspace.name.clone(),
            origin: remote.to_owned(),
            description: known.description.clone(),
        }),
        asks: workspace
            .is_none()
            .then(|| asks(known.origin_named.then_some(remote))),
        agent: known.agent.clone(),
        can_send: workspace.is_some() && unreachable.is_none(),
        unreachable,
        note,
        placeholder: PLACEHOLDER.to_owned(),
        draft: LANDING_DRAFT.to_owned(),
    }
}

/// What the Landing says where no Workspace is remembered for the Outlook's
/// Remote, as after a first pairing, naming the Remote `named` where the
/// phone names Origins at all.
pub(crate) fn asks(named: Option<&str>) -> String {
    let on = named
        .map(|remote| format!(" on {remote}"))
        .unwrap_or_default();
    format!(
        "Choose a workspace{on} to begin in: narrow the listing to one, or open a session there."
    )
}

/// Said where a Server says a Session begun now would get no Agent: no
/// Provider can serve, or its Models are not known yet. Its refusal says
/// the rest.
pub(crate) const AGENT_UNAVAILABLE: &str = "Agent unavailable";

/// The Agent `agent` says a new Session would get, in words, named as
/// `models` names its Provider, its Model and each choice made of its
/// Options: none where its Server has yet to say, or is too old to.
pub(crate) fn agent_words(
    agent: &NewSessionAgent,
    models: Option<&ModelCatalog>,
) -> Option<String> {
    match agent {
        NewSessionAgent::NotSaid => None,
        NewSessionAgent::NoAgent => Some(AGENT_UNAVAILABLE.to_owned()),
        NewSessionAgent::Agent(selection) => Some(selection_words(selection, models)),
    }
}

/// `selection` as "Claude · Fable 5.2 · high", by the names `models` gives,
/// and by its identities where it gives none.
fn selection_words(selection: &AgentSelection, models: Option<&ModelCatalog>) -> String {
    let provider = models.and_then(|models| {
        models
            .providers
            .iter()
            .find(|provider| provider.provider == selection.provider)
    });
    let model = models.and_then(|models| models.model(&selection.provider, &selection.model));
    let choices = selection.options.iter().filter_map(|chosen| {
        let option = model?
            .options
            .iter()
            .find(|option| option.id == chosen.id)?;
        let (ModelOptionKind::Select { choices, .. }, ModelOptionValue::Select { choice }) =
            (&option.kind, &chosen.value)
        else {
            return None;
        };
        choices
            .iter()
            .find(|candidate| candidate.id == *choice)
            .map(|candidate| candidate.label.clone())
    });
    [
        provider.map_or_else(
            || selection.provider.as_str().to_owned(),
            |provider| provider.display_name.clone(),
        ),
        model.map_or_else(
            || selection.model.as_str().to_owned(),
            |model| model.display_name.clone(),
        ),
    ]
    .into_iter()
    .chain(choices)
    .collect::<Vec<_>>()
    .join(" · ")
}

/// How a Session being begun stands, until its Server is seen holding it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    /// Its Server has yet to answer.
    Starting,
    /// Its Server refused it, as it says, and holds no Session of it.
    Refused(String),
    /// Its Server never answered, so it may have begun: the screen watches
    /// for the Session, and never says it was refused.
    MayHaveBegun,
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
    pub progress: Progress,
    /// The Session its Server holds, once it is seen, which replaces it.
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
            progress: Progress::Starting,
            created: None,
        }
    }

    /// The request that begins it. Its Session id and its Prompt id make it
    /// the same creation however often it is asked.
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

    /// Whether its Server has yet to answer.
    pub fn starting(&self) -> bool {
        self.progress == Progress::Starting
    }

    /// Asks again, as "Try again" does: the same creation.
    pub fn ask_again(&mut self) {
        self.progress = Progress::Starting;
    }

    /// Replaces its Prompt with `text`, under a Prompt id of its own, and
    /// asks again: a creation of its own, unless the Server holds the
    /// Session already.
    pub fn replace(&mut self, text: String) {
        self.prompt = prompt(text);
        self.progress = Progress::Starting;
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

/// What the row a beginning whose answer never came stands as says.
pub(crate) const MAYBE_BEGUN: &str = "The server did not answer, so the session may have begun";

/// What the Landing says once a beginning whose answer never came is left,
/// its Prompt back in the field.
pub(crate) const MAYBE_BEGUN_LEFT: &str = "The server did not answer, so a session may have begun \
     with this prompt. Look for it in the listing before sending it again.";

/// What the composer says where a Prompt written instead of a refused one
/// found the Session begun with the first after all.
pub(crate) const BEGUN_WITH_THE_FIRST: &str =
    "The session had begun with its first prompt, so this one was not sent";

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
            origin_named: true,
            agent: None,
        };
        let landing = landing(Some(("desk", Some(&workspace))), &known, None);
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
        assert_eq!(landing.note, None);
        assert!(landing.can_send);
        assert_eq!(landing.draft, LANDING_DRAFT);
    }

    fn catalog() -> ModelCatalog {
        serde_json::from_value(serde_json::json!({
            "providers": [{
                "provider": "claude",
                "display_name": "Claude",
                "status": {"state": "fresh"},
                "models": [{
                    "provider": "claude",
                    "id": "claude-fable-5-2",
                    "display_name": "Fable 5.2",
                    "description": "",
                    "is_default": true,
                    "availability": "available",
                    "details": serde_json::to_value(shaman_protocol::ModelDetails::default()).unwrap(),
                    "options": [{
                        "id": "effort",
                        "label": "Effort",
                        "description": null,
                        "role": "reasoning_effort",
                        "kind": {
                            "type": "select",
                            "default": "medium",
                            "choices": [
                                {"id": "medium", "label": "medium", "description": null,
                                 "availability": "available"},
                                {"id": "high", "label": "high", "description": null,
                                 "availability": "available"}
                            ]
                        }
                    }]
                }]
            }]
        }))
        .expect("a Model Catalog")
    }

    fn fable(effort: &str) -> AgentSelection {
        serde_json::from_value(serde_json::json!({
            "provider": "claude",
            "model": "claude-fable-5-2",
            "options": [{"id": "effort", "value": {"type": "select", "choice": effort}}]
        }))
        .expect("an Agent Selection")
    }

    #[test]
    fn the_agent_is_named_as_the_model_catalog_names_it_and_unavailable_where_there_is_none() {
        let models = catalog();
        assert_eq!(
            agent_words(&NewSessionAgent::Agent(fable("high")), Some(&models)).as_deref(),
            Some("Claude · Fable 5.2 · high")
        );
        assert_eq!(
            agent_words(&NewSessionAgent::Agent(fable("high")), None).as_deref(),
            Some("claude · claude-fable-5-2"),
            "by its identities before the catalog names it"
        );
        assert_eq!(
            agent_words(&NewSessionAgent::NoAgent, Some(&models)).as_deref(),
            Some("Agent unavailable")
        );
        assert_eq!(
            agent_words(&NewSessionAgent::NotSaid, Some(&models)),
            None,
            "a Server too old to say shows no line"
        );
        let landing = landing(
            Some(("desk", Some(&remembered("shaman")))),
            &Known {
                agent: Some("Agent unavailable".to_owned()),
                ..Known::default()
            },
            None,
        );
        assert_eq!(landing.agent.as_deref(), Some("Agent unavailable"));
    }

    #[test]
    fn with_one_remote_the_landing_names_no_origin_beneath_its_title() {
        let workspace = remembered("shaman");
        let one = Known::default();
        assert_eq!(
            landing(Some(("desk", Some(&workspace))), &one, None).place,
            "shaman"
        );
        let nowhere = landing(Some(("desk", None)), &one, None);
        assert_eq!(nowhere.place, "");
        assert_eq!(nowhere.asks, Some(asks(None)));
        assert!(!nowhere.asks.unwrap().contains("desk"));
    }

    #[test]
    fn on_an_unreachable_remote_nothing_begins_and_send_is_not_offered() {
        let workspace = remembered("shaman");
        let unreachable = |origin_named| {
            landing(
                Some(("desk", Some(&workspace))),
                &Known {
                    unreachable: true,
                    origin_named,
                    ..Known::default()
                },
                None,
            )
        };
        let landing = unreachable(true);
        assert_eq!(
            landing.unreachable.as_deref(),
            Some("desk is unreachable. Nothing begins there until it answers.")
        );
        assert!(!landing.can_send);
        assert!(landing.workspace.is_some(), "it still says where");
        assert_eq!(
            unreachable(false).unreachable.as_deref(),
            Some("The server is unreachable. Nothing begins there until it answers.")
        );
    }

    #[test]
    fn where_no_workspace_is_remembered_the_landing_asks_for_one_and_offers_no_send() {
        let landing = landing(
            Some(("desk", None)),
            &Known {
                origin_named: true,
                ..Known::default()
            },
            None,
        );
        assert_eq!(landing.place, "desk");
        assert_eq!(landing.workspace, None);
        assert_eq!(landing.asks, Some(asks(Some("desk"))));
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
        assert_eq!(request.prompt.id, provisional.prompt.id);
        assert_eq!(request.agent_selection, None, "a phone chooses no Agent");
        assert_eq!(request.preparation_id, None, "on the main Worktree");
        assert_eq!(request.execution_directory.path, PathBuf::from("shaman"));
    }

    #[test]
    fn asked_again_it_is_the_same_creation_and_written_instead_a_prompt_of_its_own() {
        let mut provisional = Provisional::new(
            SessionId::new(),
            PathBuf::from("shaman"),
            "Draw the invite".to_owned(),
            "shaman".to_owned(),
        );
        let first = provisional.request();
        provisional.progress = Progress::MayHaveBegun;
        provisional.ask_again();
        assert!(provisional.starting());
        assert_eq!(provisional.request(), first, "the same creation");

        provisional.progress = Progress::Refused("no".to_owned());
        provisional.replace("Trim the quiet zone".to_owned());
        assert_eq!(provisional.prompt.text, "Trim the quiet zone");
        assert_ne!(provisional.prompt.id, first.prompt.id);
        assert_eq!(provisional.session_id, first.session_id.unwrap());
        assert!(provisional.starting());
    }
}
