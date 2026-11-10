//! Deriving a Session's Title, and its Workspace's Icon, from its first
//! Prompt.
//!
//! A Session is titled with the verbatim text of its first Prompt, which is a
//! real Title rather than a placeholder but rarely a good one. The moment that
//! Prompt is admitted, Suru asks a Provider — through an Errand — for a short
//! line naming the subject and the outcome, with an Icon standing for the
//! work chosen alongside it, and replaces the Title with what comes back. On
//! the same task, right after, a second Errand asks for an Icon standing for
//! the Workspace itself and a Description of it, where it still lacks either
//! — see [`super::workspace_icon`] for what that Errand asks and how its
//! answer commits.
//!
//! The Session's own Icon travels with its Title but keeps its own rule: a
//! derived Icon lands only where the Session still carries none, so an Icon
//! derived earlier — or chosen by the user, once that lands — is never
//! overwritten by a later derivation. The Title keeps its own, separate rule
//! of replacing only the Title it was derived from. See the **Icon** and
//! **Icon Catalog** glossary entries and ADR 0028 for why an Icon is carried
//! as a Catalog name rather than a codepoint.
//!
//! Which Provider runs both Errands is the `derivation.errand` Setting's
//! answer, read once when derivation begins and shared by the Title and the
//! Workspace's Icon and Description alike. Left alone it is the Session's own, at that
//! Provider's declared Errand Selection resolved when each Errand runs, so
//! deriving either is paid for at the rate that Provider keeps for its own
//! work rather than at the rate of conversing. Turned off, neither Errand is
//! asked for. Pinned to an Agent Selection, that Provider and Model derive
//! every Session's Title and every Workspace's Icon and Description, whatever the Session
//! itself uses — including a Session using nothing.
//!
//! The two Errands run in a fixed order on the one spawned task: the Title
//! Errand first, all the way to its own commit or failure, and only then the
//! Workspace Errand — never concurrently, and never Workspace-first. A harness
//! answering Errands in the order it receives them can therefore always tell
//! the two apart by position, which is what keeps this ordering a documented
//! guarantee rather than an accident of scheduling.
//!
//! A new Managed Worktree is named by an Errand of its own, asked before the
//! Worktree is made and waited on as part of making it
//! ([`Derivation::worktree_name_for_prompt`],
//! [`Derivation::worktree_name_for_session`]): 2 to 5 plain words describing
//! the work, read from the Prompt a Session is about to begin there with, or
//! from the Prompts so far of a Session Moving into it. The Model reads a
//! Prompt exactly as the Title is asked about it, Skill names included, but is
//! asked not to make a Skill the subject. A proposal Suru can use is stripped
//! of any `refs/heads/` or `feature/` it was spelled with and shaped by the
//! one name-shaping function every Managed Worktree name goes through, and it
//! names the Worktree's location and its branch alike from the moment they are
//! made, so neither is ever renamed under an Agent at work there. The wait is
//! held to the naming timeout, which covers resolving the Model the Errand
//! runs at as well as the Errand: a Setting that derives nothing, an Errand
//! that fails or proposes nothing usable, and a wait that runs out all leave
//! the Worktree the name its Prompt — or, for a Move, the Session's Title —
//! gives it. The same Setting governs it as governs the Title, and it never
//! touches the Title: a Session's Title Errand asks for no branch at all.
//!
//! Renaming a Session is the user's act rather than Suru's: a Title they
//! write stands as written ([`SessionStore::set_title`]), and one they ask
//! Suru to propose ([`Derivation::propose`]) is derived anew from the
//! Session's Prompts so far, asks for the Title alone, and changes nothing
//! until they keep it. A proposal is governed by the same Setting, and is
//! never the second attempt the derivation below never makes of its own
//! accord.
//!
//! Everything here is best-effort by construction. A Title's derivation runs
//! in the background alongside the real first Turn and never blocks or gates
//! it, and is attempted once per Session and never again; a Worktree's name is
//! waited on, but only so long. A failure of any kind — a Provider that cannot
//! be reached, a reply that arrives too late, a reply that is not the shape
//! Suru asked for — leaves the Prompt-derived Title standing, the Workspace
//! without an Icon or a Description, or the Worktree with the name its Prompt
//! or Title gives it, and reaches the Log and nowhere else.

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{sync::watch, time::Duration};

use crate::{
    errands::{ErrandPurpose, ErrandRunner},
    icon_catalog,
    model_catalog::ModelCatalogService,
    protocol::{
        AgentSelection, AttachmentBinding, DerivationErrand, MAX_TITLE_CHARS, PreparationPrompt,
        Prompt, PromptStatus, ProviderId, SessionCatalogChange, SessionChange, SessionId,
        SessionSnapshot, SessionSummary, SettingsSnapshot, SkillInvocation, Workspace,
        one_line_title,
    },
    provider::ProviderErrand,
    source_control::naming,
};

use super::{SessionStore, workspace_icon};

/// The most of a first Prompt an Errand carries. The opening is taken rather
/// than the ending because a first Prompt states its intent up front and trails
/// off into detail, and titling should not cost a full-context call.
const MAX_ERRAND_PROMPT_CHARS: usize = 2_000;

/// The pairs a Model may wrap a Title in when it answers with one. Stripped in
/// matched pairs only, so a Title that legitimately opens with a quote keeps it.
const QUOTE_PAIRS: [(char, char); 6] = [
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
    ('\u{201C}', '\u{201D}'),
    ('\u{2018}', '\u{2019}'),
    ('\u{00AB}', '\u{00BB}'),
];

/// What Suru asks an Errand to answer with. One object carrying both the
/// Title and the Icon Catalog name, because a Model choosing them together
/// picks a better Icon than one retrofitting it onto a Title it had no say
/// in — and because two calls would cost twice as much to answer the same
/// question.
///
/// Suru's own validation tolerates extra properties and a missing or unusable
/// Icon: a Model that volunteers a field Suru did not ask for, or names an
/// Icon Suru's Catalog does not carry, has still answered the Title question,
/// and throwing a good Title away over it costs the user more than ignoring
/// it does. The requested schema still marks every property required and
/// rejects unasked ones because Codex sends output schemas in strict mode,
/// where every property must be required and no others allowed. Providers
/// that cannot enforce the schema may return outside it anyway, and Suru will
/// keep validating regardless. A reply missing the Title is what does not
/// deserialize at all, and it is discarded whole.
#[derive(Debug, Deserialize)]
struct DerivedTitleReply {
    title: String,
    #[serde(default)]
    icon: Option<String>,
}

/// What one derivation yields: the Title Suru will store, and the Icon
/// Catalog name that stands beside it when the reply named a usable one.
/// They travel together everywhere because they are derived together, in one
/// Errand, and land together in one change — though the Icon lands only
/// where the Session still has none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DerivedTitle {
    title: String,
    icon: Option<String>,
}

/// Derives Sessions' Titles and Icons, Workspaces' Icons, and new Managed
/// Worktrees' names. Cloned into the server's shared state, which is what
/// lets Session creation fork a derivation and return without waiting.
#[derive(Clone)]
pub(crate) struct Derivation {
    errands: ErrandRunner,
    /// The live Model catalog, which is what an Errand Selection is resolved
    /// against — every time one runs, rather than once when the server started.
    models: ModelCatalogService,
    sessions: SessionStore,
    /// The effective Settings in force, read when a derivation begins. A
    /// Setting the user changes governs the Sessions they start next rather
    /// than reaching back into a derivation already under way.
    settings: watch::Receiver<SettingsSnapshot>,
    /// How long making a new Managed Worktree waits on the Errand naming it.
    worktree_naming_timeout: Duration,
}

impl Derivation {
    pub(crate) fn new(
        errands: ErrandRunner,
        models: ModelCatalogService,
        sessions: SessionStore,
        settings: watch::Receiver<SettingsSnapshot>,
        worktree_naming_timeout: Duration,
    ) -> Self {
        Self {
            errands,
            models,
            sessions,
            settings,
            worktree_naming_timeout,
        }
    }

    /// Forks a derivation for a Session whose first Prompt has just been
    /// admitted, and returns at once. The derivation is independent of the
    /// first Turn's lifetime — interrupting or failing that Turn does not
    /// cancel it, because the Prompt was still written and still deserves a
    /// Title.
    ///
    /// Following the Session, a Session that has selected no Provider is
    /// skipped, permanently: Suru will not pick a Provider the user did not
    /// choose, and deferring the attempt would give derivation a second trigger
    /// point and pending state to carry. A pinned Selection is the user
    /// choosing one up front, so it titles that Session like any other. The
    /// same gate governs the Workspace Errand below: a Setting or a Session
    /// that skips the Title skips the Workspace's Icon and Description too, since all are one
    /// Provider call asked at one moment.
    ///
    /// Where the Session's own Provider runs the Errand, its Agent Selection
    /// decides the Provider and nothing else. The Model is the Provider's own
    /// business — its declared Errand Selection, resolved when the Errand runs
    /// — because the Model a user converses with is not the one that should be
    /// paid to write six words.
    ///
    /// `workspace` is read once, here, for whether it already carries an Icon
    /// and a Description: a Workspace that carries both skips the second
    /// Errand, and one lacking either gets exactly one attempt from this
    /// Session — a race with another Session created in the same Workspace
    /// before either commits is possible and harmless, because
    /// [`SessionStore::commit_workspace_icon`] and
    /// [`SessionStore::commit_workspace_description`] only ever fill an
    /// absence.
    pub(crate) fn derive(
        &self,
        session_id: SessionId,
        execution_directory: std::path::PathBuf,
        provider: Option<ProviderId>,
        prompt: &Prompt,
        workspace: &Workspace,
    ) {
        let Some(errand_at) = self.errand_at(session_id, provider) else {
            return;
        };
        // The Title as it stands right now is the one this derivation is
        // derived from, and the only one its answer may replace.
        let Some(derived_from) = self.sessions.title_standing(session_id) else {
            return;
        };
        let title_prompt =
            errand_prompt(&prompt.text, &prompt.skill_invocations, &prompt.attachments);
        let workspace_errand = self.workspace_errand(workspace);
        let errands = self.errands.clone();
        let models = self.models.clone();
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            let Some(selection) = errand_at
                .selection(&models, &session_id.to_string(), "Errand")
                .await
            else {
                return;
            };
            // The Title Errand runs first, to completion, before the
            // Workspace Errand is even built — see the module doc for why the
            // order is a guarantee rather than a scheduling accident.
            let title_errand = ProviderErrand {
                prompt: title_prompt,
                schema: reply_schema(),
                selection: selection.clone(),
                execution_directory: execution_directory.clone(),
            };
            match errands.run(ErrandPurpose::SessionTitle, title_errand).await {
                Ok(answer) => match derived_title(&answer) {
                    Some(title) => {
                        if !sessions.replace_derived_title(session_id, &derived_from, title) {
                            tracing::debug!(
                                %session_id,
                                "a derived Title was discarded because the Title it was derived from has since changed"
                            );
                        }
                    }
                    None => tracing::info!(
                        %session_id,
                        "Title Errand answered outside its schema, so the Prompt-derived Title stands"
                    ),
                },
                Err(failure) => {
                    tracing::info!(%session_id, "Title Errand produced no Title: {failure}");
                }
            }
            if let Some((workspace_id, prompt)) = workspace_errand {
                derive_workspace(
                    &errands,
                    &sessions,
                    workspace_id,
                    ProviderErrand {
                        prompt,
                        schema: workspace_icon::reply_schema(),
                        selection,
                        execution_directory,
                    },
                )
                .await;
            }
        });
    }

    /// Forks the Workspace Errand alone for `workspace`, which the Sidekick
    /// `on_behalf_of` names has just Cloned to `execution_directory`, and
    /// returns at once — so the new Workspace has its Icon and Description
    /// derived as the first Session begun there would have them: governed by
    /// the same Setting, which follows `provider` — the Sidekick's own, or
    /// for a Sidekick on a Peer the one a Session it began here would run on
    /// — unless it pins another or turns derivation off, run in the
    /// Workspace itself, and asked only where it still lacks either.
    pub(crate) fn derive_cloned_workspace(
        &self,
        on_behalf_of: impl std::fmt::Display,
        provider: Option<ProviderId>,
        workspace: &Workspace,
        execution_directory: std::path::PathBuf,
    ) {
        let Some(errand_at) = self.errand_at(on_behalf_of, provider) else {
            return;
        };
        if !self.wants_workspace_errand(workspace) {
            return;
        }
        let workspace = workspace.clone();
        let errands = self.errands.clone();
        let models = self.models.clone();
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            // The README the Errand carries is read off the Server's own
            // threads, where a clone on a mount gone quiet holds up no one.
            let workspace_id = workspace.id.clone();
            let Ok(prompt) =
                tokio::task::spawn_blocking(move || workspace_icon::errand_prompt(&workspace))
                    .await
            else {
                return;
            };
            let Some(selection) = errand_at
                .selection(&models, &format!("{workspace_id:?}"), "Workspace Errand")
                .await
            else {
                return;
            };
            derive_workspace(
                &errands,
                &sessions,
                workspace_id,
                ProviderErrand {
                    prompt,
                    schema: workspace_icon::reply_schema(),
                    selection,
                    execution_directory,
                },
            )
            .await;
        });
    }

    /// Whether a Title proposal would be derived for the Session at
    /// `session_id`, whose Agent Selection names `provider` where it names
    /// one: as the Setting in force decides, and for no Session it would not
    /// title of its own accord.
    pub(crate) fn offers_proposal(
        &self,
        session_id: SessionId,
        provider: Option<ProviderId>,
    ) -> bool {
        self.errand_at(session_id, provider).is_some()
    }

    /// The name an Errand proposes for the new Managed Worktree `asked_for`
    /// prepares for a Session about to begin on `provider` with `prompt`,
    /// waited on while the Worktree's making does and for no longer than the
    /// naming timeout. `None` leaves the Worktree the name its Prompt gives
    /// it: where the Setting derives nothing, the Errand fails or names
    /// nothing usable, or the wait ends first — said in the Log and nowhere
    /// else. The Errand runs in `source`, since the Worktree is yet to stand.
    pub(crate) async fn worktree_name_for_prompt(
        &self,
        asked_for: String,
        provider: ProviderId,
        prompt: &PreparationPrompt,
        source: &std::path::Path,
    ) -> Option<String> {
        self.worktree_name(
            asked_for,
            Some(provider),
            beginning_worktree_prompt(prompt),
            source.to_owned(),
        )
        .await
    }

    /// [`Self::worktree_name_for_prompt`] for the new Managed Worktree the
    /// Session of `snapshot` is Moving into, read from its Prompts so far.
    /// `None` leaves the Worktree the name the Session's Title gives it.
    pub(crate) async fn worktree_name_for_session(
        &self,
        snapshot: &SessionSnapshot,
        source: &std::path::Path,
    ) -> Option<String> {
        let session_id = snapshot.session.id;
        let prompts = admitted_prompts(snapshot);
        if prompts.is_empty() {
            tracing::debug!(%session_id, "no Errand: the Session has no Prompt to name a Worktree from");
            return None;
        }
        let provider = snapshot
            .session
            .agent_selection
            .as_ref()
            .map(|selection| selection.provider.clone());
        self.worktree_name(
            session_id.to_string(),
            provider,
            moving_worktree_prompt(&prompts),
            source.to_owned(),
        )
        .await
    }

    /// Runs the Errand naming a new Managed Worktree, the whole of it — the
    /// Model it runs at being resolved included — held to the naming timeout.
    async fn worktree_name(
        &self,
        asked_for: String,
        provider: Option<ProviderId>,
        prompt: String,
        execution_directory: std::path::PathBuf,
    ) -> Option<String> {
        let errand_at = self.errand_at(&asked_for, provider)?;
        let deadline = tokio::time::Instant::now() + self.worktree_naming_timeout;
        let resolved = tokio::time::timeout_at(
            deadline,
            errand_at.selection(&self.models, &asked_for, "Worktree name Errand"),
        )
        .await;
        let Ok(selection) = resolved else {
            tracing::info!(
                asked_for,
                "Worktree name Errand found no Model to run at in time, so the Worktree is named without it"
            );
            return None;
        };
        let errand = ProviderErrand {
            prompt,
            schema: worktree_name_schema(),
            selection: selection?,
            execution_directory,
        };
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match self
            .errands
            .within(remaining)
            .run(ErrandPurpose::WorktreeName, errand)
            .await
        {
            Ok(answer) => {
                let name = proposed_worktree_name(&answer);
                if name.is_none() {
                    tracing::info!(
                        asked_for,
                        "Worktree name Errand named nothing usable, so the Worktree is named without it"
                    );
                }
                name
            }
            Err(failure) => {
                tracing::info!(
                    asked_for,
                    "Worktree name Errand produced no name, so the Worktree is named without it: {failure}"
                );
                None
            }
        }
    }

    /// Derives a Title proposal from `snapshot`'s Prompts so far, at the
    /// user's asking, and answers it without storing anything: the user keeps
    /// it by renaming the Session to it. Runs while its caller waits, so a
    /// caller that stops waiting abandons the Errand with it. Asks for the
    /// Title alone — an Icon or a branch name is never proposed.
    pub(crate) async fn propose(
        &self,
        snapshot: &SessionSnapshot,
    ) -> Result<String, ProposalRefusal> {
        let session_id = snapshot.session.id;
        let provider = snapshot
            .session
            .agent_selection
            .as_ref()
            .map(|selection| selection.provider.clone());
        let Some(errand_at) = self.errand_at(session_id, provider) else {
            return Err(ProposalRefusal::Withheld);
        };
        let (provider, pinned) = match &errand_at {
            ErrandAt::Provider(provider) => (provider, None),
            ErrandAt::Selection(selection) => (&selection.provider, Some(selection)),
        };
        let Some(selection) = self
            .models
            .resolved_errand_selection(provider, pinned)
            .await
        else {
            return Err(ProposalRefusal::Failed(format!(
                "`{provider}` offers no Model to derive a Title at"
            )));
        };
        let errand = ProviderErrand {
            prompt: proposal_prompt(&admitted_prompts(snapshot)),
            schema: proposal_schema(),
            selection,
            execution_directory: snapshot.session.execution_directory.path.clone(),
        };
        let answer = self
            .errands
            .run(ErrandPurpose::TitleProposal, errand)
            .await
            .map_err(|failure| ProposalRefusal::Failed(failure.to_string()))?;
        serde_json::from_value::<DerivedTitleReply>(answer)
            .ok()
            .and_then(|reply| sanitized_title(&reply.title))
            .ok_or_else(|| {
                ProposalRefusal::Failed("the Provider answered with no usable Title".to_owned())
            })
    }

    /// The Workspace Errand `workspace` asks for — its identity and the
    /// Prompt asking for its Icon and Description — where it still lacks
    /// either. The Sidekick Workspace is Suru's own: it wears a standing Icon
    /// and is described only by whoever chooses to, so nothing of it is
    /// derived.
    fn workspace_errand(
        &self,
        workspace: &Workspace,
    ) -> Option<(crate::protocol::WorkspaceId, String)> {
        self.wants_workspace_errand(workspace).then(|| {
            (
                workspace.id.clone(),
                workspace_icon::errand_prompt(workspace),
            )
        })
    }

    /// Whether `workspace` asks for a Workspace Errand: where it lacks an
    /// Icon or a Description, and is not the Sidekick Workspace.
    fn wants_workspace_errand(&self, workspace: &Workspace) -> bool {
        (workspace.icon.is_none() || workspace.description.is_none())
            && !self.sessions.is_sidekick_workspace(&workspace.id)
    }

    /// Where this derivation's Errands go, as the Setting in force decides,
    /// and `None` where they go nowhere — for what `asked_for` names, the
    /// Session or the Sidekick it is derived for. Read before anything is
    /// spawned, so a Setting turning derivation off costs no task and no
    /// Provider call.
    fn errand_at(
        &self,
        asked_for: impl std::fmt::Display,
        provider: Option<ProviderId>,
    ) -> Option<ErrandAt> {
        match &self.settings.borrow().settings.derivation.errand {
            DerivationErrand::Off => {
                tracing::debug!(%asked_for, "no Errand: derivation is turned off");
                None
            }
            DerivationErrand::Pinned(selection) => Some(ErrandAt::Selection(selection.clone())),
            DerivationErrand::FollowSession => match provider {
                Some(provider) => Some(ErrandAt::Provider(provider)),
                None => {
                    tracing::debug!(%asked_for, "no Errand: no Provider is selected for it");
                    None
                }
            },
        }
    }
}

/// Runs a Workspace Errand and commits each half of its answer where the
/// Workspace still lacks it: the Icon where it carries none, the Description
/// where it carries none, neither waiting on the other.
async fn derive_workspace(
    errands: &ErrandRunner,
    sessions: &SessionStore,
    workspace_id: crate::protocol::WorkspaceId,
    errand: ProviderErrand,
) {
    let answer = match errands
        .run(ErrandPurpose::WorkspaceDescription, errand)
        .await
    {
        Ok(answer) => answer,
        Err(failure) => {
            tracing::info!(
                ?workspace_id,
                "Workspace Errand produced no Icon or Description: {failure}"
            );
            return;
        }
    };
    let derived = workspace_icon::derived_reply(&answer);
    // The Description is attempted ahead of the Icon, so whoever hears of the
    // Icon landing knows the whole reply has been read.
    match derived.description {
        Some(description) => {
            if !sessions.commit_workspace_description(&workspace_id, description) {
                tracing::debug!(
                    ?workspace_id,
                    "a derived Workspace Description was discarded because the Workspace already carries one"
                );
            }
        }
        None => tracing::info!(
            ?workspace_id,
            "Workspace Errand answered with no Description"
        ),
    }
    match derived.icon {
        Some(icon) => {
            if !sessions.commit_workspace_icon(&workspace_id, icon) {
                tracing::debug!(
                    ?workspace_id,
                    "a derived Workspace Icon was discarded because the Workspace already carries one"
                );
            }
        }
        None => tracing::info!(
            ?workspace_id,
            "Workspace Errand answered with no Icon the Icon Catalog carries"
        ),
    }
}

/// What one derivation was told to run its Errand at: a Provider whose own
/// declaration decides the Model, or a whole Selection the user pinned. Both
/// are resolved against the live catalog by the same rules when the Errand
/// runs; the difference is only whose choice is being resolved.
enum ErrandAt {
    Provider(ProviderId),
    Selection(AgentSelection),
}

impl ErrandAt {
    /// The Selection `errand`, run for `on_behalf_of`, runs at as the catalog
    /// stands now; `None`, saying so in the Log, where its Provider offers no
    /// Model to run one at.
    async fn selection(
        &self,
        models: &ModelCatalogService,
        on_behalf_of: &str,
        errand: &str,
    ) -> Option<AgentSelection> {
        let (provider, pinned) = match self {
            Self::Provider(provider) => (provider, None),
            Self::Selection(selection) => (&selection.provider, Some(selection)),
        };
        let selection = models.resolved_errand_selection(provider, pinned).await;
        if selection.is_none() {
            tracing::info!(
                on_behalf_of,
                "no {errand}: `{provider}` offers no Model to run one at"
            );
        }
        selection
    }
}

/// A Title as a derivation finds it: what it reads, and how many times the
/// Session had been renamed. A derivation replaces only the Title standing
/// as it found it, so a rename in between — even to the same words — stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TitleStanding {
    pub(crate) title: String,
    pub(crate) renames: u64,
}

/// Why no Title proposal was derived.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProposalRefusal {
    /// The Session does not exist on this Server.
    SessionNotFound,
    /// The Server's own storage could not read the Session; the Log says how.
    Storage,
    /// The Setting in force derives no Title for this Session: it is off, or
    /// follows a Session with no Provider.
    Withheld,
    /// The Errand failed, timed out, or answered with no usable Title, as the
    /// message says.
    Failed(String),
}

impl SessionStore {
    /// The Session's Title as it stands, or `None` for a Session this server
    /// does not hold.
    pub(crate) fn title(&self, session_id: SessionId) -> Option<String> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .sessions
            .get(&session_id)
            .map(|record| record.summary.title.clone())
    }

    /// The Session's Title as it stands, and how many times it has been
    /// renamed, or `None` for a Session this server does not hold.
    pub(crate) fn title_standing(&self, session_id: SessionId) -> Option<TitleStanding> {
        let state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        state.sessions.get(&session_id).map(|record| TitleStanding {
            title: record.summary.title.clone(),
            renames: state.title_renames.get(&session_id).copied().unwrap_or(0),
        })
    }

    /// Replaces a Session's Title and Icon with a derived pair, but only
    /// where each of their own guards allows it: the Title only when it
    /// still stands as the one the derivation was derived from — reading the
    /// same, and renamed no time since — and the Icon only where the Session
    /// still carries none.
    ///
    /// The Title guard is what makes a derivation that is still in flight
    /// safe, and what lets a rename command land later without a schema
    /// change: a Title set by other means while an Errand was outstanding is
    /// never overwritten by the answer to that Errand. The Icon guard is the
    /// same idea applied to derivation filling an absence rather than
    /// replacing a value: an Icon this Session already carries — derived
    /// earlier, or chosen by the user once that lands — stands regardless of
    /// what a later derivation offers. Answers `true` when the Title changed;
    /// a Title that changes without an Icon to fill is still a change.
    ///
    /// The change reaches both the open Session stream and the catalog, so
    /// attached clients see the same Title and Icon as readers of Session
    /// listings. [`SessionStore::set_icon`] shares this same
    /// commit-then-publish shape rather than duplicating it, differing only
    /// in that it writes the Icon unconditionally instead of filling an
    /// absence.
    pub(crate) fn replace_derived_title(
        &self,
        session_id: SessionId,
        derived_from: &TitleStanding,
        derived: DerivedTitle,
    ) -> bool {
        let DerivedTitle { title, icon } = derived;
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let renames = state.title_renames.get(&session_id).copied().unwrap_or(0);
        let icon = {
            let Some(record) = state.sessions.get_mut(&session_id) else {
                return false;
            };
            if record.summary.title != derived_from.title || renames != derived_from.renames {
                return false;
            }
            // A derived Icon only ever fills an absence.
            let icon = record.summary.icon.clone().or(icon);
            if let Err(error) = record.commit_derived(
                &self.storage,
                session_id,
                vec![SessionChange::TitleChanged {
                    title: title.clone(),
                    icon: icon.clone(),
                }],
            ) {
                tracing::warn!(%session_id, %error, "Could not commit derived Title");
                return false;
            }
            icon
        };
        state.publish_catalog_change(SessionCatalogChange::TitleChanged {
            session_id,
            title,
            icon,
        });
        state.follow_subsession_title(&self.storage, session_id);
        state.announce_subagent_tree(session_id);
        true
    }

    /// Sets a Session's Icon to the user's own choice from the Icon Catalog,
    /// replacing whatever it already carried — derived earlier, chosen
    /// before, or absent — because a user's choice always stands rather than
    /// only ever filling an absence the way derivation does. Refuses a name
    /// the Icon Catalog does not carry, so a Session never stores an Icon
    /// that would only ever draw as no Icon at all.
    ///
    /// Shares `replace_derived_title`'s commit-then-publish shape and its
    /// `TitleChanged` change, carrying the Session's current Title unchanged
    /// alongside the new Icon: every client's existing apply path already
    /// repaints from that one change kind, so a chosen Icon reaches it the
    /// same way a derived one does.
    pub(crate) fn set_icon(
        &self,
        session_id: SessionId,
        icon: &str,
    ) -> Result<SessionSummary, SetIconError> {
        if icon_catalog::glyph(icon).is_none() {
            return Err(SetIconError::UnknownIcon);
        }
        let icon = icon.to_owned();
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let (title, summary) = {
            let Some(record) = state.sessions.get_mut(&session_id) else {
                return Err(SetIconError::SessionNotFound);
            };
            let title = record.summary.title.clone();
            record
                .commit_derived(
                    &self.storage,
                    session_id,
                    vec![SessionChange::TitleChanged {
                        title: title.clone(),
                        icon: Some(icon.clone()),
                    }],
                )
                .map_err(|_| SetIconError::Storage)?;
            (title, record.summary.clone())
        };
        state.publish_catalog_change(SessionCatalogChange::TitleChanged {
            session_id,
            title,
            icon: Some(icon),
        });
        Ok(summary)
    }

    /// Renames a Session: sets its Title to a line the user wrote, folded to
    /// one line and trimmed, replacing whatever stood — the Prompt's text, a
    /// derived Title, or an earlier rename. A derivation still in flight was
    /// derived from the Title this replaces, so
    /// [`Self::replace_derived_title`]'s guard discards its answer.
    ///
    /// Publishes the same `TitleChanged` change a derived Title does, carrying
    /// the Session's Icon unchanged, and follows it to the trees and
    /// Sidekick listings a derived Title reaches.
    pub(crate) fn set_title(
        &self,
        session_id: SessionId,
        title: &str,
    ) -> Result<SessionSummary, SetTitleError> {
        let title = written_title(title)?;
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let state = &mut *state;
        let (icon, summary) = {
            let Some(record) = state.sessions.get_mut(&session_id) else {
                return Err(SetTitleError::SessionNotFound);
            };
            let icon = record.summary.icon.clone();
            record
                .commit_derived(
                    &self.storage,
                    session_id,
                    vec![SessionChange::TitleChanged {
                        title: title.clone(),
                        icon: icon.clone(),
                    }],
                )
                .map_err(|_| SetTitleError::Storage)?;
            *state.title_renames.entry(session_id).or_default() += 1;
            (icon, record.summary.clone())
        };
        state.publish_catalog_change(SessionCatalogChange::TitleChanged {
            session_id,
            title,
            icon,
        });
        state.follow_subsession_title(&self.storage, session_id);
        state.announce_subagent_tree(session_id);
        Ok(summary)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetTitleError {
    SessionNotFound,
    /// Nothing is left once the written Title is folded and trimmed.
    Blank,
    /// The written Title runs past [`MAX_TITLE_CHARS`], this many characters
    /// long.
    TooLong(usize),
    Storage,
}

/// A Title the user wrote, as it stands: kept as [`one_line_title`] keeps it.
/// Refused rather than cut short where it runs past the cap, since a user's
/// own words are theirs to shorten.
fn written_title(raw: &str) -> Result<String, SetTitleError> {
    let folded = one_line_title(raw);
    if folded.is_empty() {
        return Err(SetTitleError::Blank);
    }
    let chars = folded.chars().count();
    if chars > MAX_TITLE_CHARS {
        return Err(SetTitleError::TooLong(chars));
    }
    Ok(folded)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetIconError {
    SessionNotFound,
    /// The named Icon does not resolve in the Icon Catalog.
    UnknownIcon,
    Storage,
}

/// A Prompt's text as a title writer reads it: Skill sigils and Attachment
/// labels taken out.
fn titling_text(
    prompt: &str,
    skill_invocations: &[SkillInvocation],
    attachments: &[AttachmentBinding],
) -> String {
    // Admission already proved each bound `$skill-name` begins at its span and
    // each Attachment label reads as bound at its own. Removing only a Skill's
    // sigil keeps its visible name useful to the title writer without letting
    // a Provider interpret it as an invocation; a label such as `[Image 1]`
    // names nothing of the work, so it goes whole.
    let mut cuts = skill_invocations
        .iter()
        .map(|invocation| invocation.span.start as usize)
        .filter(|start| prompt.as_bytes().get(*start) == Some(&b'$'))
        .map(|start| start..start + 1)
        .chain(
            attachments
                .iter()
                .filter(|binding| prompt.get(binding.span.range()) == Some(&*binding.label))
                .map(|binding| binding.span.range()),
        )
        .collect::<Vec<_>>();
    cuts.sort_unstable_by_key(|cut| (cut.start, cut.end));
    cuts.dedup();
    let mut prompt = prompt.to_owned();
    let mut kept_from = prompt.len();
    for cut in cuts.into_iter().rev() {
        if cut.end <= kept_from {
            kept_from = cut.start;
            prompt.replace_range(cut, "");
        }
    }
    prompt
}

/// The Prompt one Title Errand carries: what to write, and the first Prompt to
/// write it about.
fn errand_prompt(
    prompt: &str,
    skill_invocations: &[SkillInvocation],
    attachments: &[AttachmentBinding],
) -> String {
    let prompt = titling_text(prompt, skill_invocations, attachments);
    let mut characters = prompt.chars();
    let opening = characters
        .by_ref()
        .take(MAX_ERRAND_PROMPT_CHARS)
        .collect::<String>();
    format!(
        "Name the piece of work the request below begins.\n\n\
         Answer with a title of 3 to 8 words, under 50 characters, naming the subject of \
         the work and what it is meant to achieve. Do not echo the wording of the request, \
         do not address the reader, and do not end with a full stop. Answer also with an \
         Icon standing for the work, chosen from the offered names alongside the title \
         rather than fitted to it afterwards.\n\n\
         The request:\n{opening}"
    )
}

/// The Prompts of `snapshot`'s Session so far, as an Errand reads them.
fn admitted_prompts(snapshot: &SessionSnapshot) -> Vec<&Prompt> {
    snapshot
        .prompts
        .iter()
        .filter(|prompt| prompt.status != PromptStatus::Cancelled)
        .collect()
}

/// A Session's Prompts so far, as an Errand carries them to write about, within
/// the one budget a first Prompt is held to. The first Prompt opens the work and
/// the latest say where it went, so where they cannot all be carried the first
/// keeps up to half the budget and the latest fill the rest, those between them
/// giving way.
fn carried_requests(prompts: &[&Prompt]) -> String {
    carried_texts(
        prompts
            .iter()
            .map(|prompt| {
                titling_text(&prompt.text, &prompt.skill_invocations, &prompt.attachments)
            })
            .collect(),
    )
}

/// [`carried_requests`] of Prompts already read as a title writer reads them.
fn carried_texts(texts: Vec<String>) -> String {
    let texts = texts
        .iter()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    let opening = |text: &str, budget: usize| text.chars().take(budget).collect::<String>();
    let mut carried = Vec::new();
    if let Some((first, later)) = texts.split_first() {
        let first_budget = if later.is_empty() {
            MAX_ERRAND_PROMPT_CHARS
        } else {
            MAX_ERRAND_PROMPT_CHARS / 2
        };
        let first = opening(first, first_budget);
        let mut remaining = MAX_ERRAND_PROMPT_CHARS - first.chars().count();
        let mut latest = Vec::new();
        for text in later.iter().rev() {
            if remaining == 0 {
                break;
            }
            let kept = opening(text, remaining);
            remaining -= kept.chars().count();
            latest.push(kept);
        }
        let skipped = later.len() - latest.len();
        carried.push(first);
        match skipped {
            0 => {}
            1 => carried.push("(1 more request)".to_owned()),
            _ => carried.push(format!("({skipped} more requests)")),
        }
        carried.extend(latest.into_iter().rev());
    }
    carried.join("\n\n---\n\n")
}

/// The Prompt a Title proposal's Errand carries: what to write, and the
/// Session's Prompts so far to write it about.
fn proposal_prompt(prompts: &[&Prompt]) -> String {
    format!(
        "Name the piece of work the requests below have carried out so far, in the order \
         they were made.\n\n\
         Answer with a title of 3 to 8 words, under 50 characters, naming the subject of \
         the work and what it is meant to achieve. Do not echo the wording of the requests, \
         do not address the reader, and do not end with a full stop.\n\n\
         The requests:\n{}",
        carried_requests(prompts)
    )
}

/// What every Worktree name Errand is asked to answer with.
const WORKTREE_NAME_ASKED: &str = "Answer with a branch name of 2 to 5 plain words describing the work, with no prefix \
     and no issue numbers, and never naming a Skill as its subject.";

/// The Prompt the Errand naming a new Session's Worktree carries: what to
/// write, and the opening of the Prompt the Session is about to begin with.
fn beginning_worktree_prompt(prompt: &PreparationPrompt) -> String {
    let requested = titling_text(&prompt.text, &prompt.skill_invocations, &prompt.attachments);
    format!(
        "Name the branch for the piece of work the request below begins.\n\n\
         {WORKTREE_NAME_ASKED}\n\n\
         The request:\n{}",
        carried_texts(vec![requested])
    )
}

/// The Prompt the Errand naming a Moving Session's Worktree carries: what to
/// write, and the Session's Prompts so far to write it about.
fn moving_worktree_prompt(prompts: &[&Prompt]) -> String {
    format!(
        "Name the branch for the piece of work the requests below have carried out so far, \
         in the order they were made.\n\n\
         {WORKTREE_NAME_ASKED}\n\n\
         The requests:\n{}",
        carried_requests(prompts)
    )
}

/// The shape a Worktree name Errand answers in: the branch alone.
fn worktree_name_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "branch": {
                "type": "string",
                "description": "2 to 5 plain words describing the work, with no prefix, no issue numbers, and no Skill's name as its subject",
            },
        },
        "required": ["branch"],
        "additionalProperties": false,
    })
}

/// The shape a Title proposal's Errand answers in: the Title alone.
fn proposal_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "description": "3 to 8 words under 50 characters naming the subject of the work and what it is meant to achieve",
            },
        },
        "required": ["title"],
        "additionalProperties": false,
    })
}

/// The shape Suru asks a Title Errand to answer in. A request rather than a
/// guarantee — every reply is validated here regardless of whether the harness
/// could enforce it.
fn reply_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "description": "3 to 8 words under 50 characters naming the subject of the work and what it is meant to achieve",
            },
            "icon": {
                "type": "string",
                "description": "the Icon Catalog name standing for the work",
                "enum": icon_catalog::names(),
            },
        },
        "required": ["title", "icon"],
        "additionalProperties": false,
    })
}

/// What a Title Errand's reply yields: its Title and Icon, or `None` when it
/// yields no Title, and so nothing at all. Sanitizing runs on every path,
/// whether or not the harness was able to enforce the schema, because no
/// Provider can be relied on to have done it.
fn derived_title(answer: &Value) -> Option<DerivedTitle> {
    let reply: DerivedTitleReply = serde_json::from_value(answer.clone()).ok()?;
    Some(DerivedTitle {
        title: sanitized_title(&reply.title)?,
        icon: reply
            .icon
            .filter(|name| icon_catalog::glyph(name).is_some()),
    })
}

/// The name a Worktree name Errand's reply proposes, shaped as every Managed
/// Worktree's name is, or `None` where it proposes nothing usable: read as
/// whatever JSON arrives, so a reply of the wrong shape is only no proposal.
fn proposed_worktree_name(answer: &Value) -> Option<String> {
    answer
        .get("branch")?
        .as_str()
        .and_then(naming::proposed_fragment)
}

/// A Model-authored Title as Suru stores it: its first non-empty line, unwrapped
/// from any quotes around it, with its whitespace collapsed and its length
/// capped. A Title that survives none of that is no Title.
fn sanitized_title(raw: &str) -> Option<String> {
    let line = raw.lines().find(|line| !line.trim().is_empty())?;
    let unquoted = unquoted(line.trim());
    let collapsed = unquoted.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= MAX_TITLE_CHARS {
        return Some(collapsed);
    }
    let mut capped = collapsed
        .chars()
        .take(MAX_TITLE_CHARS - 1)
        .collect::<String>();
    capped.push('\u{2026}');
    Some(capped)
}

/// The text inside one matched pair of quotes, or the text itself when it is
/// not wrapped in one.
fn unquoted(text: &str) -> &str {
    let mut characters = text.chars();
    let (Some(first), Some(last)) = (characters.next(), characters.next_back()) else {
        return text;
    };
    if QUOTE_PAIRS
        .iter()
        .any(|(open, close)| first == *open && last == *close)
    {
        characters.as_str().trim()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Title as a derivation found it, renamed no time since.
    fn found(title: &str) -> TitleStanding {
        TitleStanding {
            title: title.to_owned(),
            renames: 0,
        }
    }

    #[test]
    fn a_title_is_taken_from_the_first_non_empty_line() {
        assert_eq!(
            sanitized_title("\n \nFix the reasoning flicker\nand some rambling after"),
            Some("Fix the reasoning flicker".to_owned())
        );
    }

    #[test]
    fn a_title_loses_the_quotes_around_it_and_its_extra_whitespace() {
        assert_eq!(
            sanitized_title("  \"Fix   the\treasoning flicker\"  "),
            Some("Fix the reasoning flicker".to_owned())
        );
        assert_eq!(
            sanitized_title("\u{201C}Fix the reasoning flicker\u{201D}"),
            Some("Fix the reasoning flicker".to_owned())
        );
    }

    #[test]
    fn a_lone_quote_is_not_a_pair_and_stays() {
        assert_eq!(
            sanitized_title("\"Fix the reasoning flicker"),
            Some("\"Fix the reasoning flicker".to_owned())
        );
    }

    #[test]
    fn an_over_long_title_is_capped_and_marked() {
        let capped = sanitized_title(&"word ".repeat(60)).expect("a long Title still yields one");
        assert_eq!(capped.chars().count(), MAX_TITLE_CHARS);
        assert!(capped.ends_with('\u{2026}'));
    }

    #[test]
    fn a_title_that_exactly_fills_the_cap_is_left_whole() {
        let exact = "w".repeat(MAX_TITLE_CHARS);
        assert_eq!(sanitized_title(&exact), Some(exact));
    }

    #[test]
    fn a_title_of_nothing_is_no_title() {
        assert_eq!(sanitized_title("   \n  \t "), None);
        assert_eq!(sanitized_title("\"\""), None);
    }

    #[test]
    fn a_reply_without_a_title_yields_nothing_at_all() {
        assert!(derived_title(&json!({ "icon": "md-bug" })).is_none());
        assert!(derived_title(&json!("Fix the flicker")).is_none());
        assert!(derived_title(&json!({ "title": "   " })).is_none());
    }

    #[test]
    fn a_reply_with_an_unknown_icon_still_yields_its_title() {
        assert_eq!(
            derived_title(&json!({ "title": "Fix the flicker", "icon": "not-a-catalog-name" })),
            Some(DerivedTitle {
                title: "Fix the flicker".to_owned(),
                icon: None,
            })
        );
    }

    #[test]
    fn a_reply_missing_an_icon_still_yields_its_title() {
        assert_eq!(
            derived_title(&json!({ "title": "Fix the flicker" })),
            Some(DerivedTitle {
                title: "Fix the flicker".to_owned(),
                icon: None,
            })
        );
    }

    #[test]
    fn a_reply_carrying_more_than_suru_asked_for_still_yields_its_title() {
        assert_eq!(
            derived_title(&json!({
                "title": "Fix the flicker",
                "icon": "md-bug",
                "confidence": 0.9,
            })),
            Some(DerivedTitle {
                title: "Fix the flicker".to_owned(),
                icon: Some("md-bug".to_owned()),
            })
        );
    }

    #[test]
    fn a_worktree_name_is_only_what_a_reply_usably_proposes() {
        assert_eq!(
            proposed_worktree_name(&json!({ "branch": "feature/Fix reasoning flicker" })),
            Some("fix-reasoning-flicker".to_owned())
        );
        for unusable in [
            json!({ "branch": "" }),
            json!({ "branch": "please do it" }),
            json!({ "branch": null }),
            json!({ "branch": 42 }),
            json!({ "title": "Fix the flicker" }),
            json!("fix-flicker"),
        ] {
            assert_eq!(proposed_worktree_name(&unusable), None, "{unusable}");
        }
    }

    #[test]
    fn a_title_errand_asks_for_no_branch() {
        let schema = reply_schema();
        assert!(schema["properties"].get("branch").is_none());
        assert!(!errand_prompt("Fix the flicker", &[], &[]).contains("branch"));
    }

    #[test]
    fn a_worktree_name_errand_asks_for_the_branch_alone() {
        let schema = worktree_name_schema();
        assert_eq!(schema["properties"]["branch"]["type"], json!("string"));
        assert_eq!(schema["required"], json!(["branch"]));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn a_new_sessions_worktree_is_named_from_the_opening_of_its_prompt() {
        let text = format!("$review {}", "x".repeat(MAX_ERRAND_PROMPT_CHARS + 500));
        let skill = SkillInvocation {
            skill_id: crate::protocol::SkillId::new("review"),
            name: "review".to_owned(),
            scope: None,
            span: crate::protocol::TextSpan { start: 0, end: 7 },
        };
        let carried = beginning_worktree_prompt(&PreparationPrompt {
            text,
            skill_invocations: vec![skill],
            attachments: Vec::new(),
        });
        assert!(
            carried.contains("The request:\nreview x"),
            "the Skill keeps its name without its sigil: {carried}"
        );
        assert!(
            !carried.contains(&"x".repeat(MAX_ERRAND_PROMPT_CHARS)),
            "only the opening is carried"
        );
    }

    #[test]
    fn a_title_errand_schema_rejects_unasked_properties() {
        assert_eq!(reply_schema()["additionalProperties"], json!(false));
    }

    #[test]
    fn a_title_errand_schema_enumerates_the_icon_catalog() {
        let schema = reply_schema();
        assert_eq!(
            schema["properties"]["icon"]["enum"],
            json!(icon_catalog::names()),
            "a Model can only choose an Icon Suru's Catalog can resolve"
        );
        assert_eq!(schema["required"], json!(["title", "icon"]));
    }

    /// The Title guard is exercised here rather than at the server seam
    /// because nothing else in Suru writes a Title yet — a rename command is
    /// the caller this exists for, and it does not exist. The guard is built
    /// now anyway, because it is what lets that command land later without a
    /// schema change and what makes a derivation still in flight safe.
    #[tokio::test]
    async fn a_derived_title_replaces_only_the_title_it_was_derived_from() {
        use crate::{
            protocol::{CreateSessionRequest, InitialPrompt, PromptId},
            storage::{StorageRepository, StorageWriter},
        };

        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let execution_directory = tempfile::tempdir().expect("create valid Workspace");
        let repository = StorageRepository::open(data_dir.path())
            .await
            .expect("open Session repository");
        let (_writer, storage) = StorageWriter::spawn(repository);
        let store = SessionStore::new(Default::default(), storage, Vec::new(), Default::default());
        let created = store
            .create(CreateSessionRequest {
                session_id: None,
                preparation_id: None,
                agent_selection: None,
                execution_directory: crate::protocol::ExecutionDirectory {
                    path: execution_directory.path().to_owned(),
                },
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: "Explain the seam".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
            })
            .expect("create Session");
        let crate::sessions::StoreOutcome::Created(snapshot) = created else {
            panic!("a fresh Prompt creates a Session");
        };
        let session_id = snapshot.session.id;

        assert!(
            !store.replace_derived_title(
                session_id,
                &found("a Title this Session never had"),
                DerivedTitle {
                    title: "Derived from something else".to_owned(),
                    icon: None,
                },
            ),
            "a derivation cannot replace a Title it was not derived from"
        );
        assert_eq!(store.title(session_id).as_deref(), Some("Explain the seam"));

        assert!(store.replace_derived_title(
            session_id,
            &found("Explain the seam"),
            DerivedTitle {
                title: "Explain the Provider seam".to_owned(),
                icon: Some("md-bug".to_owned()),
            },
        ));
        assert_eq!(
            store.title(session_id).as_deref(),
            Some("Explain the Provider seam")
        );

        let before_rename = store
            .title_standing(session_id)
            .expect("the created Session is held");
        store
            .set_title(session_id, "Explain the Provider seam")
            .expect("rename to the very words standing");
        assert!(
            !store.replace_derived_title(
                session_id,
                &before_rename,
                DerivedTitle {
                    title: "Derived before the rename".to_owned(),
                    icon: None,
                },
            ),
            "a rename stands against a derivation even where it wrote the words it found"
        );
        assert_eq!(
            store.title(session_id).as_deref(),
            Some("Explain the Provider seam")
        );
    }

    /// The Icon's own guard: derivation fills an absence and never overwrites
    /// an Icon the Session already carries, exactly as a chosen Icon will
    /// stand against a later derivation once issue #360 lands a way to
    /// choose one.
    #[tokio::test]
    async fn a_derived_icon_fills_an_absence_but_never_overwrites_one() {
        use crate::{
            protocol::{CreateSessionRequest, InitialPrompt, PromptId},
            storage::{StorageRepository, StorageWriter},
        };

        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let execution_directory = tempfile::tempdir().expect("create valid Workspace");
        let repository = StorageRepository::open(data_dir.path())
            .await
            .expect("open Session repository");
        let (_writer, storage) = StorageWriter::spawn(repository);
        let store = SessionStore::new(Default::default(), storage, Vec::new(), Default::default());
        let created = store
            .create(CreateSessionRequest {
                session_id: None,
                preparation_id: None,
                agent_selection: None,
                execution_directory: crate::protocol::ExecutionDirectory {
                    path: execution_directory.path().to_owned(),
                },
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: "Explain the seam".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
            })
            .expect("create Session");
        let crate::sessions::StoreOutcome::Created(snapshot) = created else {
            panic!("a fresh Prompt creates a Session");
        };
        let session_id = snapshot.session.id;

        assert!(store.replace_derived_title(
            session_id,
            &found("Explain the seam"),
            DerivedTitle {
                title: "Explain the Provider seam".to_owned(),
                icon: Some("md-bug".to_owned()),
            },
        ));
        assert_eq!(
            store
                .state
                .lock()
                .expect("Session store lock is not poisoned")
                .sessions
                .get(&session_id)
                .expect("the created Session is held")
                .summary
                .icon
                .as_deref(),
            Some("md-bug"),
            "a first derivation fills the Session's absent Icon"
        );

        assert!(store.replace_derived_title(
            session_id,
            &found("Explain the Provider seam"),
            DerivedTitle {
                title: "Explain the Provider seam once more".to_owned(),
                icon: Some("dev-rust".to_owned()),
            },
        ));
        assert_eq!(
            store
                .state
                .lock()
                .expect("Session store lock is not poisoned")
                .sessions
                .get(&session_id)
                .expect("the created Session is held")
                .summary
                .icon
                .as_deref(),
            Some("md-bug"),
            "a second derivation never overwrites the Icon the Session already carries"
        );
    }

    #[test]
    fn an_errand_carries_only_the_opening_of_a_long_prompt() {
        let prompt = "x".repeat(MAX_ERRAND_PROMPT_CHARS + 500);
        let carried = errand_prompt(&prompt, &[], &[]);
        assert!(carried.contains(&"x".repeat(MAX_ERRAND_PROMPT_CHARS)));
        assert!(!carried.contains(&"x".repeat(MAX_ERRAND_PROMPT_CHARS + 1)));
    }

    #[test]
    fn an_errand_names_the_work_without_the_labels_of_its_attachments() {
        let text = "$review [Image 1] against [Image 2]";
        let skill = SkillInvocation {
            skill_id: crate::protocol::SkillId::new("review"),
            name: "review".to_owned(),
            scope: None,
            span: crate::protocol::TextSpan { start: 0, end: 7 },
        };
        let label = |label: &str| {
            let start = text.find(label).expect("the label stands in the text");
            AttachmentBinding {
                attachment_id: crate::protocol::AttachmentId::new(label),
                label: label.to_owned(),
                span: crate::protocol::TextSpan::from(start..start + label.len()),
            }
        };
        let carried = errand_prompt(text, &[skill], &[label("[Image 2]"), label("[Image 1]")]);
        assert!(
            carried.ends_with("The request:\nreview  against "),
            "the Skill keeps its name and each label goes whole: {carried}"
        );
    }
}
