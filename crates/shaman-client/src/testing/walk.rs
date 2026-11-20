//! A walk of every shared command, over a transport that keeps what it is
//! asked.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use http::Method;
use tokio::sync::watch;

use super::open_stream_of;
use crate::{
    SessionCommandTimings, SessionCommands,
    protocol::{
        AdmitPromptRequest, AgentSelection, AgentSelectionOperationId, ApprovalId, AttachmentId,
        CheckoutAssociation, CheckoutBranchOutcome, CheckoutId, CheckoutKind,
        CheckoutRemovalInspection, CheckoutRemovalPreview, CheckoutRemovalTarget, CheckoutSummary,
        CompactSessionRequest, CreateSessionRequest, Decision, ExecutionDirectory, InitialPrompt,
        InsightsQuery, ListDirectoryRequest, ModelId, MoveDestination, MoveSessionRequest,
        PreparationId, PrepareCheckoutRequest, PromptDelivery, PromptId, ProviderId,
        QuestionnaireId, QuestionnaireSubmission, RemoveCheckoutRequest, Repository, RepositoryId,
        RepositoryLocation, ResolveWorkspaceRequest, SessionError, SessionId, SessionTimestamp,
        SkillCatalogRequest, SourceControlAvailability, SourceControlCapabilities,
        UpdateAgentSelectionRequest, UpdateApprovalPostureRequest, UsageMetricsBreakdown,
        UsageMetricsInterval, UsageMetricsQuery, ViewSessionOperationId, ViewSessionRequest,
        WorkspaceId,
    },
    transport::{Transport, TransportResponse, TryNow, full_body},
};

/// A transport that answers every request with an empty success, and
/// keeps what it was asked.
pub struct Recorder {
    asked: Mutex<Vec<(Method, http::Uri)>>,
    count: watch::Sender<usize>,
}

impl Recorder {
    /// A recorder that has been asked nothing.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            count: watch::Sender::new(0),
        })
    }

    /// What it has been asked since last taken.
    pub fn take(&self) -> Vec<(Method, http::Uri)> {
        std::mem::take(&mut *self.asked.lock().expect("the record lock"))
    }

    /// Waits until it has been asked something since last taken.
    pub async fn asked_something(&self) {
        let asked = || !self.asked.lock().expect("the record lock").is_empty();
        let mut count = self.count.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !asked() {
                count.changed().await.expect("the recorder stands");
            }
        })
        .await
        .expect("a reader asks as it opens");
    }
}

#[async_trait::async_trait]
impl Transport for Recorder {
    async fn send(&self, request: http::Request<Bytes>) -> Result<TransportResponse, SessionError> {
        self.asked
            .lock()
            .expect("the record lock")
            .push((request.method().clone(), request.uri().clone()));
        self.count.send_modify(|count| *count += 1);
        // A stream is held open, so its reader asks nothing more of its
        // own accord; anything else is answered at once.
        let path = request.uri().path();
        if path.ends_with("events") || path.ends_with("subagent-tree") {
            return Ok(open_stream_of(&[]));
        }
        Ok(http::Response::new(full_body(Bytes::new())))
    }

    fn try_now(&self) -> TryNow {
        TryNow::never()
    }

    fn attached(self: Arc<Self>) -> Arc<dyn Transport> {
        self
    }
}

/// Asks every shared command and reader once, with what it takes, over a
/// [`Recorder`], and has `judge` judge what each asked under its name. A
/// command added to [`SessionCommands`] is added here, so the walk judges
/// every route the shared commands send: the Serving side's tests judge them
/// by its own route check, which this crate does not hold. This crate's own
/// test fails while one is missing.
#[allow(clippy::too_many_lines)]
pub async fn walk_every_command(judge: impl Fn(&'static str, Vec<(Method, http::Uri)>)) {
    let recorder = Recorder::new();
    let commands = SessionCommands::new(
        recorder.clone(),
        SessionCommandTimings {
            initial_recovery_backoff: Duration::from_millis(1),
            max_recovery_backoff: Duration::from_millis(1),
            attachment_fetch_timeout: Duration::from_secs(5),
        },
    );
    let session = SessionId::new();
    let root = PathBuf::from("worktree");
    let directory = ExecutionDirectory { path: root.clone() };
    let provider = ProviderId::new("codex");
    let selection = AgentSelection {
        provider: provider.clone(),
        model: ModelId::new("model"),
        options: Vec::new(),
    };
    let prompt = InitialPrompt {
        id: PromptId::new(),
        text: "Hello".to_owned(),
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
    };
    let repository = Repository {
        id: RepositoryId::from_metadata("git", &root),
        system: "git".to_owned(),
        metadata_directory: root.join(".git"),
        location: RepositoryLocation::Main { root: root.clone() },
        availability: SourceControlAvailability::Available,
        capabilities: SourceControlCapabilities::discovery_only(),
    };
    let checkout = CheckoutAssociation {
        id: CheckoutId::from_root(&repository.id, &root),
        repository: repository.id.clone(),
        root: root.clone(),
        kind: CheckoutKind::Linked,
        recovery_revision: None,
        reclaim: None,
    };
    let target = CheckoutRemovalTarget {
        repository,
        checkout: checkout.clone(),
    };
    let preview = CheckoutRemovalPreview {
        target: target.clone(),
        inspection: CheckoutRemovalInspection {
            checkout: CheckoutSummary {
                association: checkout,
                revision: None,
                availability: SourceControlAvailability::Available,
            },
            tracked: Vec::new(),
            untracked: Vec::new(),
            ignored: Vec::new(),
            lock: None,
            initialized_submodules: Vec::new(),
        },
        branch_outcome: CheckoutBranchOutcome::Retained,
        affected_sessions: 0,
        working_sessions: 0,
        pending_moves: 0,
    };
    let interval = UsageMetricsInterval {
        start: SessionTimestamp(0),
        end: SessionTimestamp(1),
    };
    let skills = SkillCatalogRequest {
        provider: provider.clone(),
        execution_directory: directory.clone(),
    };
    let workspace = WorkspaceId("workspace".to_owned());
    let attachment = AttachmentId::new("attachment");

    macro_rules! walk {
        ($($name:literal => $asked:expr),* $(,)?) => {
            $(
                let _ = $asked;
                judge($name, recorder.take());
            )*
        };
    }
    walk![
        "subscribe_catalog" => {
            let _catalog = commands.subscribe_catalog();
            recorder.asked_something().await;
        },
        "subscribe_subagent_tree" => {
            let _tree = commands.subscribe_subagent_tree(session);
            recorder.asked_something().await;
        },
        "subscribe_session" => commands.subscribe_session(session).await.map(drop),
        "attach_session" => commands.attach_session(session).await.map(drop),
        "resolve_workspace" => commands.resolve_workspace(ResolveWorkspaceRequest {
            checkout_id: None,
            remembered_execution_directory: None,
            workspace_id: None,
            base: None,
            path: root.clone(),
        }).await,
        "list_directory" => commands.list_directory(ListDirectoryRequest {
            path: root.clone(),
            base: Some(root.clone()),
        }).await,
        "usage_metrics" => commands.usage_metrics(&UsageMetricsQuery {
            buckets: vec![interval],
            breakdown: UsageMetricsBreakdown::Model,
            model_view: Default::default(),
            comparison: None,
            top_sessions: 0,
        }).await,
        "insights" => commands.insights(&InsightsQuery { interval }).await,
        "sidekick_workspace" => commands.sidekick_workspace().await,
        "list_skills" => commands.list_skills(skills.clone()).await,
        "refresh_skills" => commands.refresh_skills(skills).await,
        "health" => commands.health().await,
        "list_models" => commands.list_models().await,
        "refresh_models" => commands.refresh_models().await,
        "warm_models" => commands.warm_models().await,
        "preview_checkout_removal" => commands.preview_checkout_removal(target).await,
        "remove_checkout" => commands.remove_checkout(
            RemoveCheckoutRequest { preview, force: false },
        ).await,
        "prepare_checkout" => commands.prepare_checkout(PrepareCheckoutRequest {
            id: PreparationId::default(),
            source: directory.clone(),
            prompt: Default::default(),
            provider,
            intended_session: None,
        }).await,
        "create_session" => commands.create_session(CreateSessionRequest {
            preparation_id: None,
            agent_selection: Some(selection.clone()),
            execution_directory: directory,
            prompt: prompt.clone(),
            session_id: None,
        }).await,
        "admit_prompt" => commands.admit_prompt(session, AdmitPromptRequest {
            prompt,
            delivery: PromptDelivery::Queue,
        }).await,
        "upload_attachment" => commands.upload_attachment(vec![0_u8; 8]).await,
        "fetch_attachment" => commands.fetch_attachment(&attachment).await,
        "attachment_exists" => commands.attachment_exists(&attachment).await,
        "update_agent_selection" => commands.update_agent_selection(
            session,
            UpdateAgentSelectionRequest {
                operation_id: AgentSelectionOperationId::new(),
                selection: selection.clone(),
            },
        ).await,
        "update_approval_posture" => commands.update_approval_posture(
            session,
            UpdateApprovalPostureRequest { posture: None },
        ).await,
        "confirm_landing_agent_selection" => {
            commands.confirm_landing_agent_selection(selection).await
        },
        "promote_prompt" => commands.promote_prompt(session, PromptId::new()).await,
        "cancel_prompt" => commands.cancel_prompt(session, PromptId::new()).await,
        "submit_questionnaire" => commands.submit_questionnaire(
            session,
            QuestionnaireId::new(),
            QuestionnaireSubmission::Decline,
        ).await,
        "submit_decision" => {
            commands.submit_decision(session, ApprovalId::new(), Decision::Accept).await
        },
        "interrupt_session" => commands.interrupt_session(session).await,
        "interrupt_session_reporting_outcome" => {
            commands.interrupt_session_reporting_outcome(session).await
        },
        "compact_session" => {
            commands.compact_session(session, CompactSessionRequest::default()).await
        },
        "read_session" => commands.read_session(session).await,
        "context_breakdown" => commands.context_breakdown(session).await,
        "delete_session" => commands.delete_session(session).await,
        "settle_session" => commands.settle_session(session, true).await,
        "set_session_icon" => commands.set_session_icon(session, "star").await,
        "set_session_title" => commands.set_session_title(session, "Title").await,
        "move_session" => commands.move_session(session, &MoveSessionRequest {
            destination: MoveDestination::Directory { path: root.clone() },
        }).await,
        "session_title_proposal_offered" => {
            commands.session_title_proposal_offered(session).await
        },
        "propose_session_title" => commands.propose_session_title(session).await,
        "set_workspace_icon" => commands.set_workspace_icon(&workspace, "star").await,
        "set_workspace_description" => {
            commands.set_workspace_description(&workspace, Some(&root), "Notes").await
        },
        "view_session" => commands.view_session(session, ViewSessionRequest {
            operation_id: ViewSessionOperationId::new(),
        }).await,
        "list_workspace_sessions" => commands.list_workspace_sessions(Some(&workspace)).await,
        "list_sessions" => commands.list_sessions(Some(&root)).await,
        "list_workspaces" => commands.list_workspaces().await,
    ];
}

/// What [`SessionCommands`] offers that asks its Server nothing, and so is
/// not walked.
#[cfg(test)]
const ASKING_NOTHING: &[&str] = &["new", "recovery_backoff"];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    /// The name of each `pub fn` declared in `impl SessionCommands`, as
    /// `source` declares it, however its lines end: a checkout on Windows
    /// ends them with CRLF.
    fn declared(source: &str) -> BTreeSet<String> {
        source
            .lines()
            .skip_while(|line| *line != "impl SessionCommands {")
            .skip(1)
            .take_while(|line| *line != "}")
            .filter_map(|line| {
                let line = line.strip_prefix("    pub ")?;
                let line = line.strip_prefix("async ").unwrap_or(line);
                let name = line.strip_prefix("fn ")?;
                let end = name.find(['(', '<'])?;
                Some(name[..end].to_owned())
            })
            .collect()
    }

    const SOURCE: &str = include_str!("../session_commands.rs");

    #[test]
    fn the_commands_are_read_alike_whatever_their_lines_end_with() {
        let unix = SOURCE.replace("\r\n", "\n");
        let windows = unix.replace('\n', "\r\n");
        let read = declared(&unix);
        assert!(read.contains("health"), "{read:?}");
        assert!(read.contains("list_workspaces"), "{read:?}");
        assert_eq!(declared(&windows), read);
    }

    #[tokio::test]
    async fn every_command_that_asks_its_server_is_walked() {
        let walked = std::sync::Mutex::new(BTreeSet::new());
        super::walk_every_command(|name, _| {
            assert!(
                walked
                    .lock()
                    .expect("the walked lock")
                    .insert(name.to_owned()),
                "{name} is walked twice"
            );
        })
        .await;
        let walked = walked.into_inner().expect("the walked lock");
        let declared = declared(SOURCE);
        assert!(declared.contains("list_workspaces"), "{declared:?}");
        let unwalked = declared
            .iter()
            .filter(|name| {
                !walked.contains(*name) && !super::ASKING_NOTHING.contains(&name.as_str())
            })
            .collect::<Vec<_>>();
        assert!(unwalked.is_empty(), "not walked: {unwalked:?}");
        let undeclared = walked.difference(&declared).collect::<Vec<_>>();
        assert!(
            undeclared.is_empty(),
            "walked, and no command: {undeclared:?}"
        );
    }
}
