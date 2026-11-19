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
    AgentSelection, CheckoutKind, CheckoutRevision, CheckoutSummary, CreateSessionRequest,
    ExecutionDirectory, ExecutionDirectoryStatus, Health, InitialPrompt, ModelCatalog,
    ModelOptionKind, ModelOptionValue, NewSessionAgent, PreparationId, PreparationPrompt,
    PrepareCheckoutRequest, ProviderId, RepositoryLocation, ResolvedWorkspace, SessionId,
    SessionSnapshot, SourceControlAvailability, SourceControlCapability, Workspace,
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
    /// The offer to begin in another Workspace, of any Remote, which pushes
    /// the Workspace Picker; none while the phone holds no Remote.
    pub elsewhere: Option<String>,
    /// The Worktree it will begin on, where its Workspace is a Repository
    /// a new Worktree can be made in: the main one, or a new one. Not kept;
    /// the screen starts on the main one every time.
    pub worktree: Option<WorktreeChoice>,
    /// Said where its Workspace is a bare Repository, which has no main
    /// Worktree to begin on: send is not offered.
    pub working_copy: Option<String>,
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

/// The two Worktrees a Session begun from the Landing may begin on, as the
/// screen offers them, the first chosen as it opens.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WorktreeChoice {
    /// The main Worktree, by its Checkout State: "On main", "On detached
    /// 1a2b3c4d".
    pub main: String,
    /// "In a new worktree", which the Server makes before the Session begins.
    pub new: String,
}

/// The Workspace a Session begun from the Landing begins in, as the middle
/// of the screen names it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LandingWorkspace {
    /// Which Workspace it is, by its Remote and its path there, so Swift
    /// tells one from another however they are named.
    pub id: String,
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
    /// What it may begin on there.
    pub worktrees: Worktrees,
}

/// What a Session begun in a Workspace may begin on, as its Server says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Worktrees {
    /// Outside source control, or not yet known: its directory alone, and no
    /// choice.
    #[default]
    Directory,
    /// A Repository with a main Worktree, standing on the Checkout State
    /// `main` names where it is known, in which a new Worktree can be made.
    Repository { main: Option<String> },
    /// A bare Repository, which has no main Worktree to begin on.
    Bare,
}

/// What a Session begun in `listed` may begin on, by what its Remote lists
/// of it, and by `resolved`, how it last resolved it, where it has.
pub(crate) fn worktrees(
    listed: Option<&Workspace>,
    resolved: Option<&ResolvedWorkspace>,
) -> Worktrees {
    let workspace = resolved.map(|resolved| &resolved.workspace).or(listed);
    let Some(repository) = workspace.and_then(|workspace| workspace.repository.as_deref()) else {
        return Worktrees::Directory;
    };
    let bare = match resolved {
        Some(resolved) => {
            resolved.execution_status == ExecutionDirectoryStatus::RequiresWorkingCopy
        }
        None => matches!(repository.location, RepositoryLocation::Bare { .. }),
    };
    if bare {
        return Worktrees::Bare;
    }
    let available = matches!(
        repository.availability,
        SourceControlAvailability::Available
    ) && matches!(
        repository.capabilities.create_checkout,
        SourceControlCapability::Available
    ) && resolved.is_none_or(|resolved| {
        matches!(
            resolved.execution_status,
            ExecutionDirectoryStatus::Available
        )
    });
    if !available {
        return Worktrees::Directory;
    }
    let main = resolved.and_then(|resolved| {
        resolved
            .checkouts
            .iter()
            .find(|checkout| checkout.association.kind == CheckoutKind::Main)
            .and_then(checkout_state)
    });
    Worktrees::Repository { main }
}

/// The main Worktree's Checkout State as the Worktree Selector labels it,
/// where it was read.
fn checkout_state(checkout: &CheckoutSummary) -> Option<String> {
    if checkout.availability != SourceControlAvailability::Available {
        return None;
    }
    match checkout.revision.as_ref()? {
        CheckoutRevision::Branch { name, commit: None } => Some(format!("{name} (unborn)")),
        CheckoutRevision::Branch { name, .. } => Some(name.clone()),
        CheckoutRevision::Detached { commit } => Some(format!(
            "detached {}",
            commit.chars().take(8).collect::<String>()
        )),
        CheckoutRevision::Unknown => None,
    }
}

/// What the Worktree choice offers first: the main Worktree, by its Checkout
/// State where it is known.
fn on_main(main: Option<&str>) -> String {
    main.map_or_else(
        || "On the main worktree".to_owned(),
        |main| format!("On {main}"),
    )
}

/// What the Worktree choice offers second.
const NEW_WORKTREE: &str = "In a new worktree";

/// What the Landing offers to begin elsewhere by.
const ELSEWHERE: &str = "Begin somewhere else";

/// What the Landing says where its Workspace is a bare Repository.
pub(crate) const WORKING_COPY: &str = "This workspace is a bare repository. A session needs a \
     working copy of it to begin in, which this phone cannot choose yet.";

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
            elsewhere: None,
            worktree: None,
            working_copy: None,
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
    let (worktree, working_copy) = match (workspace, &known.worktrees) {
        (None, _) | (_, Worktrees::Directory) => (None, None),
        (Some(_), Worktrees::Repository { main }) => (
            Some(WorktreeChoice {
                main: on_main(main.as_deref()),
                new: NEW_WORKTREE.to_owned(),
            }),
            None,
        ),
        (Some(_), Worktrees::Bare) => (None, Some(WORKING_COPY.to_owned())),
    };
    Landing {
        title: TITLE.to_owned(),
        place,
        workspace: workspace.map(|workspace| LandingWorkspace {
            id: format!("{remote}/{}", workspace.path.display()),
            glyph: known.glyph.clone(),
            name: workspace.name.clone(),
            origin: remote.to_owned(),
            description: known.description.clone(),
        }),
        asks: workspace
            .is_none()
            .then(|| asks(known.origin_named.then_some(remote))),
        elsewhere: Some(ELSEWHERE.to_owned()),
        agent: known.agent.clone(),
        can_send: workspace.is_some() && unreachable.is_none() && working_copy.is_none(),
        worktree,
        working_copy,
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
    format!("Choose a workspace{on} to begin in.")
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
    /// The new Worktree it begins in, where one was chosen.
    pub worktree: Option<NewWorktree>,
    /// The Session its Server holds, once it is seen, which replaces it.
    pub created: Option<Box<SessionSnapshot>>,
}

/// A new Worktree a Session being begun is to begin in, which its Server
/// prepares first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewWorktree {
    /// The identity of its preparation, the same however often it is asked,
    /// so a preparation the Server made already resumes.
    pub preparation: PreparationId,
    /// Where the Server made it, once it answered that it stands ready.
    pub prepared: Option<PathBuf>,
}

impl Provisional {
    /// The Session `session_id` begun at `directory` with the Prompt `text`,
    /// standing at `place`, on the Workspace's main Worktree.
    pub fn new(session_id: SessionId, directory: PathBuf, text: String, place: String) -> Self {
        Self {
            session_id,
            directory,
            prompt: prompt(text),
            place,
            progress: Progress::Starting,
            worktree: None,
            created: None,
        }
    }

    /// The same, in a new Worktree of the Repository at `directory`.
    pub fn in_new_worktree(mut self) -> Self {
        self.worktree = Some(NewWorktree {
            preparation: PreparationId::default(),
            prepared: None,
        });
        self
    }

    /// The request that prepares its new Worktree, for the Provider
    /// `provider` the Session will run on, where one is to be prepared yet.
    /// Its preparation id makes it the same preparation however often it is
    /// asked.
    pub fn preparation(&self, provider: ProviderId) -> Option<PrepareCheckoutRequest> {
        let worktree = self.worktree.as_ref()?;
        if worktree.prepared.is_some() {
            return None;
        }
        Some(PrepareCheckoutRequest {
            id: worktree.preparation,
            source: ExecutionDirectory {
                path: self.directory.clone(),
            },
            prompt: PreparationPrompt {
                text: self.prompt.text.clone(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
            provider,
            intended_session: Some(self.session_id),
        })
    }

    /// Whether a new Worktree is still to be prepared for it.
    pub fn preparing(&self) -> bool {
        self.worktree
            .as_ref()
            .is_some_and(|worktree| worktree.prepared.is_none())
    }

    /// Takes the new Worktree as prepared at `destination`.
    pub fn prepared(&mut self, destination: PathBuf) {
        if let Some(worktree) = self.worktree.as_mut() {
            worktree.prepared = Some(destination);
        }
    }

    /// The request that begins it, in the new Worktree once it is prepared.
    /// Its Session id and its Prompt id make it the same creation however
    /// often it is asked.
    pub fn request(&self) -> CreateSessionRequest {
        let prepared = self.worktree.as_ref().and_then(|worktree| {
            worktree
                .prepared
                .as_ref()
                .map(|destination| (worktree.preparation, destination))
        });
        CreateSessionRequest {
            preparation_id: prepared.map(|(preparation, _)| preparation),
            // The Server's own Agent Selection: a phone chooses none.
            agent_selection: None,
            execution_directory: ExecutionDirectory {
                path: prepared.map_or_else(|| self.directory.clone(), |(_, at)| at.clone()),
            },
            prompt: self.prompt.clone(),
            session_id: Some(self.session_id),
        }
    }

    /// What its bar reads while its Server has yet to answer: "Creating
    /// worktree" while a new one is prepared, and "Starting" after.
    pub fn bar_words(&self) -> &'static str {
        if self.preparing() {
            CREATING_WORKTREE
        } else {
            crate::frame::STARTING
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

/// The Provider a new Worktree is prepared for, as `health` says it: that of
/// the Agent a Session begun with none would get, or, from a Server too old
/// to say, that of the Agent Selection its Landing has stored; none where the
/// Server names no Agent.
pub(crate) fn worktree_provider(health: &Health) -> Option<ProviderId> {
    match health.new_session_agent() {
        NewSessionAgent::Agent(selection) => Some(selection.provider),
        NewSessionAgent::NoAgent => None,
        NewSessionAgent::NotSaid => health
            .landing_agent_selection
            .as_ref()
            .map(|selection| selection.provider.clone()),
    }
}

/// What the bar of a Session being begun reads while its new Worktree is
/// prepared.
pub(crate) const CREATING_WORKTREE: &str = "Creating worktree";

/// Why a beginning in a new Worktree is refused where its Server names no
/// Agent a Session would get, whose Provider the Worktree is prepared for.
pub(crate) const NO_AGENT_FOR_WORKTREE: &str =
    "the server names no agent to prepare a new worktree for";

/// Why a beginning in a new Worktree stands refused where the preparation's
/// answer never came: nothing was begun, and trying again resumes it.
pub(crate) const PREPARATION_UNANSWERED: &str =
    "the server did not answer while the worktree was being prepared";

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
            worktrees: Worktrees::Directory,
        };
        let landing = landing(Some(("desk", Some(&workspace))), &known, None);
        assert_eq!(landing.title, "New session");
        assert_eq!(landing.place, "desk · shaman");
        assert_eq!(
            landing.workspace,
            Some(LandingWorkspace {
                id: "desk/shaman".to_owned(),
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

    fn repository(location: RepositoryLocation) -> Workspace {
        serde_json::from_value(serde_json::json!({
            "id": "git:shaman",
            "path": "shaman",
            "repository": {
                "id": "git:shaman",
                "system": "git",
                "metadata_directory": ".git",
                "location": location,
                "availability": {"status": "available"},
                "capabilities": {
                    "list_checkouts": {"status": "available"},
                    "create_checkout": {"status": "available"},
                    "recover_checkout": {"status": "available"},
                    "remove_checkout": {"status": "available"},
                },
            },
            "source_control": {"status": "available"},
            "icon": null,
            "description": null,
        }))
        .expect("a Workspace")
    }

    fn resolved(
        workspace: Workspace,
        status: &str,
        revision: serde_json::Value,
    ) -> ResolvedWorkspace {
        serde_json::from_value(serde_json::json!({
            "execution_status": {"status": status},
            "workspace": workspace,
            "execution_directory": {"path": "shaman"},
            "checkout": null,
            "checkouts": [{
                "association": {
                    "recovery_revision": null,
                    "reclaim": null,
                    "id": "main",
                    "repository": "git:shaman",
                    "root": "shaman",
                    "kind": "main",
                },
                "revision": revision,
                "availability": {"status": "available"},
            }],
        }))
        .expect("a resolved Workspace")
    }

    fn main_root() -> RepositoryLocation {
        RepositoryLocation::Main {
            root: PathBuf::from("shaman"),
        }
    }

    #[test]
    fn a_repository_offers_its_main_worktree_by_its_checkout_state_or_a_new_one() {
        let listed = repository(main_root());
        assert_eq!(
            worktrees(Some(&listed), None),
            Worktrees::Repository { main: None },
            "offered as listed, before it resolves"
        );
        let branch =
            serde_json::json!({"kind": "branch", "name": "develop", "commit": "1a2b3c4d5e"});
        assert_eq!(
            worktrees(
                Some(&listed),
                Some(&resolved(listed.clone(), "available", branch))
            ),
            Worktrees::Repository {
                main: Some("develop".to_owned())
            }
        );
        let detached = serde_json::json!({"kind": "detached", "commit": "1a2b3c4d5e6f"});
        assert_eq!(
            worktrees(None, Some(&resolved(listed.clone(), "available", detached))),
            Worktrees::Repository {
                main: Some("detached 1a2b3c4d".to_owned())
            }
        );
        let unborn = serde_json::json!({"kind": "branch", "name": "main", "commit": null});
        assert_eq!(
            worktrees(None, Some(&resolved(listed, "available", unborn))),
            Worktrees::Repository {
                main: Some("main (unborn)".to_owned())
            }
        );

        let workspace = remembered("shaman");
        let at = |worktrees| {
            landing(
                Some(("desk", Some(&workspace))),
                &Known {
                    worktrees,
                    ..Known::default()
                },
                None,
            )
        };
        let offered = at(Worktrees::Repository {
            main: Some("develop".to_owned()),
        });
        assert_eq!(
            offered.worktree,
            Some(WorktreeChoice {
                main: "On develop".to_owned(),
                new: "In a new worktree".to_owned(),
            })
        );
        assert!(offered.can_send);
        assert_eq!(
            at(Worktrees::Repository { main: None })
                .worktree
                .map(|choice| choice.main),
            Some("On the main worktree".to_owned())
        );
        let outside = at(Worktrees::Directory);
        assert_eq!(outside.worktree, None, "outside source control");
        assert!(outside.can_send);
    }

    #[test]
    fn a_bare_repository_offers_no_worktree_and_no_send() {
        let bare = repository(RepositoryLocation::Bare {
            root: PathBuf::from("mirror.git"),
        });
        assert_eq!(worktrees(Some(&bare), None), Worktrees::Bare);
        let branch = serde_json::json!({"kind": "branch", "name": "main", "commit": "1a2b"});
        assert_eq!(
            worktrees(
                Some(&bare),
                Some(&resolved(bare.clone(), "requires_working_copy", branch))
            ),
            Worktrees::Bare
        );
        let workspace = remembered("mirror.git");
        let landing = landing(
            Some(("desk", Some(&workspace))),
            &Known {
                worktrees: Worktrees::Bare,
                ..Known::default()
            },
            None,
        );
        assert_eq!(landing.worktree, None);
        assert_eq!(landing.working_copy.as_deref(), Some(WORKING_COPY));
        assert!(!landing.can_send, "a working copy must be chosen");
        assert_eq!(
            worktrees(Some(&Workspace::directory(PathBuf::from("notes"))), None),
            Worktrees::Directory
        );
    }

    #[test]
    fn a_session_begun_in_a_new_worktree_prepares_it_first_and_begins_there() {
        let mut provisional = Provisional::new(
            SessionId::new(),
            PathBuf::from("shaman"),
            "Tidy the build".to_owned(),
            "shaman".to_owned(),
        )
        .in_new_worktree();
        assert_eq!(provisional.bar_words(), "Creating worktree");
        let preparation = provisional
            .preparation(ProviderId::new("claude"))
            .expect("a preparation to ask");
        assert_eq!(preparation.source.path, PathBuf::from("shaman"));
        assert_eq!(preparation.prompt.text, "Tidy the build");
        assert_eq!(preparation.intended_session, Some(provisional.session_id));
        assert_eq!(
            provisional
                .preparation(ProviderId::new("claude"))
                .unwrap()
                .id,
            preparation.id,
            "the same preparation however often it is asked"
        );

        provisional.prepared(PathBuf::from("shaman/.worktrees/tidy"));
        assert_eq!(provisional.bar_words(), "Starting");
        assert_eq!(provisional.preparation(ProviderId::new("claude")), None);
        let request = provisional.request();
        assert_eq!(request.preparation_id, Some(preparation.id));
        assert_eq!(
            request.execution_directory.path,
            PathBuf::from("shaman/.worktrees/tidy")
        );
        assert_eq!(request.agent_selection, None, "a phone names no Agent");
    }

    fn health(protocol_version: u32) -> Health {
        Health::new(
            shaman_protocol::ServerIdentity {
                instance_id: Default::default(),
                pid: 1,
                protocol_version,
                build_identity: String::new(),
            },
            shaman_protocol::LifecycleState::Ready,
        )
    }

    #[test]
    fn a_new_worktree_is_prepared_for_the_provider_of_the_agent_a_session_would_get() {
        let current = shaman_protocol::PROTOCOL_VERSION;
        assert_eq!(
            worktree_provider(
                &health(current)
                    .with_new_session_agent_selection(Some(fable("high")))
                    .with_landing_agent_selection(Some(AgentSelection {
                        provider: ProviderId::new("codex"),
                        ..fable("high")
                    }))
            ),
            Some(ProviderId::new("claude"))
        );
        assert_eq!(
            worktree_provider(&health(current).with_landing_agent_selection(Some(fable("high")))),
            None,
            "a Server that says none would serve names no Provider"
        );
        assert_eq!(
            worktree_provider(&health(0).with_landing_agent_selection(Some(fable("high")))),
            Some(ProviderId::new("claude")),
            "a Server too old to say gives what its Landing has stored"
        );
        assert_eq!(worktree_provider(&health(0)), None);
    }
}
