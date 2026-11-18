//! The commands a Peer may ask of a Server's work, and the readers of its
//! Sessions, catalog and Subagent trees, all sent over a transport: the
//! machine's own Server on loopback, turned toward an Outlook, or later a
//! Remote reached over a Pairing. Nothing here learns which.
//!
//! These are exactly the commands whose routes the Serving side lets a Peer
//! ask. What only a Client's own Server answers — its Settings, its
//! Onboarding, its Pairings and Relays — is its administration, sent on the
//! local Server's descriptor instead.

use std::{fmt::Display, future::Future, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use http::{Method, header::CONTENT_TYPE};
use http_body_util::BodyExt;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Serialize, de::DeserializeOwned};

use crate::protocol::{
    AdmitPromptRequest, AgentSelection, AttachmentDescriptor, AttachmentId, CreateSessionRequest,
    InterruptOutcome, ModelCatalog, MoveOutcome, MoveSessionRequest, Prompt, PromptId,
    ResolveWorkspaceRequest, SESSION_ERROR_CODE_HEADER, SessionApprovalPosture, SessionError,
    SessionErrorCode, SessionId, SessionListItem, SessionSnapshot, SessionSummary,
    SessionTitleProposal, SessionTitleProposalOffer, SetSessionIconRequest, SetSessionTitleRequest,
    SetWorkspaceDescriptionRequest, SetWorkspaceIconRequest, SettleSessionRequest, SkillCatalog,
    SkillCatalogRequest, UpdateAgentSelectionRequest, UpdateApprovalPostureRequest,
    ViewSessionRequest, WorkspaceId, WorkspaceListing,
};

use super::{
    SessionCatalogSubscription, SessionSubscription, SubagentTreeSubscription,
    transport::{Transport, TransportResponse},
};

/// The commands a Peer may ask of the one Server's work a transport reaches.
#[derive(Clone)]
pub(crate) struct SessionCommands {
    transport: Arc<dyn Transport>,
    timings: SessionCommandTimings,
}

/// How long the commands and readers wait: a reader's backoff between
/// reconnections, and the deadline a whole Attachment request is held to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionCommandTimings {
    pub(crate) initial_recovery_backoff: Duration,
    pub(crate) max_recovery_backoff: Duration,
    pub(crate) attachment_fetch_timeout: Duration,
}

impl SessionCommands {
    pub(crate) fn new(transport: Arc<dyn Transport>, timings: SessionCommandTimings) -> Self {
        Self { transport, timings }
    }

    pub(crate) fn recovery_backoff(&self) -> (Duration, Duration) {
        (
            self.timings.initial_recovery_backoff,
            self.timings.max_recovery_backoff,
        )
    }

    pub(crate) fn subscribe_catalog(&self) -> SessionCatalogSubscription {
        SessionCatalogSubscription::open_attached(
            self.transport.clone(),
            self.timings.initial_recovery_backoff,
            self.timings.max_recovery_backoff,
        )
    }

    pub(crate) fn subscribe_subagent_tree(
        &self,
        session_id: SessionId,
    ) -> SubagentTreeSubscription {
        SubagentTreeSubscription::open(
            self.transport.clone(),
            session_id,
            self.timings.initial_recovery_backoff,
            self.timings.max_recovery_backoff,
        )
    }

    /// The Session's stream, read to its end and not reopened.
    pub(crate) async fn subscribe_session(
        &self,
        session_id: SessionId,
    ) -> Result<SessionSubscription> {
        SessionSubscription::open(self.transport.as_ref(), session_id).await
    }

    /// The Session's stream, reopened whenever it drops, over the transport
    /// an attached stream runs over: one bound to the Server as it stands,
    /// where the transport can tell, so the stream ends once that Server is
    /// replaced and its reader attaches afresh.
    pub(crate) async fn attach_session(
        &self,
        session_id: SessionId,
    ) -> Result<SessionSubscription> {
        SessionSubscription::open_attached(
            self.transport.clone().attached(),
            session_id,
            self.timings.initial_recovery_backoff,
            self.timings.max_recovery_backoff,
        )
        .await
    }

    pub(crate) async fn resolve_workspace(
        &self,
        request: ResolveWorkspaceRequest,
    ) -> Result<crate::protocol::ResolvedWorkspace> {
        self.post_json("/v1/workspaces/resolve", &request, "Workspace resolution")
            .await
    }

    pub(crate) async fn list_directory(
        &self,
        request: crate::protocol::ListDirectoryRequest,
    ) -> Result<crate::protocol::DirectoryListing> {
        let path = with_query("/v1/workspaces/directory", &request)?;
        let response = self
            .send(bodiless(Method::GET, &path)?, "directory listing")
            .await?;
        decode_api_response(response, "Directory listing").await
    }

    /// The Usage Metrics of the Server this transport reaches. The query
    /// rides the body, as the Server's route asks.
    pub(crate) async fn usage_metrics(
        &self,
        query: &crate::protocol::UsageMetricsQuery,
    ) -> Result<crate::protocol::UsageMetricsReport> {
        self.post_json("/v1/usage-metrics", query, "Usage Metrics read")
            .await
    }

    /// The Insights of the Server this transport reaches, asked as its Usage
    /// Metrics are.
    pub(crate) async fn insights(
        &self,
        query: &crate::protocol::InsightsQuery,
    ) -> Result<crate::protocol::InsightsReport> {
        self.post_json("/v1/usage-metrics/insights", query, "Insights read")
            .await
    }

    /// The Sidekick Workspace of the Server this transport reaches, which
    /// that Server makes the first time it is asked for.
    pub(crate) async fn sidekick_workspace(&self) -> Result<crate::protocol::ResolvedWorkspace> {
        self.post_without_body("/v1/workspaces/sidekick", "Sidekick Workspace")
            .await
    }

    pub(crate) async fn list_skills(&self, request: SkillCatalogRequest) -> Result<SkillCatalog> {
        self.post_json("/v1/skills", &request, "Skill Catalog listing")
            .await
    }

    pub(crate) async fn refresh_skills(
        &self,
        request: SkillCatalogRequest,
    ) -> Result<SkillCatalog> {
        self.post_json("/v1/skills/refresh", &request, "Skill Catalog refresh")
            .await
    }

    pub(crate) async fn list_models(&self) -> Result<ModelCatalog> {
        self.get_json("/v1/models", "Model listing").await
    }

    pub(crate) async fn refresh_models(&self) -> Result<ModelCatalog> {
        self.post_without_body("/v1/models/refresh", "Model catalog refresh")
            .await
    }

    pub(crate) async fn warm_models(&self) -> Result<()> {
        let response = self
            .send(
                bodiless(Method::POST, "/v1/models/warm")?,
                "Model catalog warm-up",
            )
            .await?;
        decode_empty_api_response(response, "Model catalog warm-up").await
    }

    pub(crate) async fn preview_checkout_removal(
        &self,
        target: crate::protocol::CheckoutRemovalTarget,
    ) -> Result<crate::protocol::CheckoutRemovalPreview> {
        self.post_json(
            "/v1/checkouts/removal-preview",
            &target,
            "Worktree removal preview",
        )
        .await
    }

    pub(crate) async fn remove_checkout(
        &self,
        request: crate::protocol::RemoveCheckoutRequest,
    ) -> Result<crate::protocol::RemoveCheckoutResult> {
        self.post_json("/v1/checkouts/remove", &request, "Worktree removal")
            .await
    }

    pub(crate) async fn prepare_checkout(
        &self,
        request: crate::protocol::PrepareCheckoutRequest,
    ) -> Result<crate::protocol::PrepareCheckoutResult> {
        self.post_json("/v1/checkouts/prepare", &request, "Worktree preparation")
            .await
    }

    pub(crate) async fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<SessionSnapshot> {
        self.post_json("/v1/sessions", &request, "Session creation")
            .await
    }

    pub(crate) async fn admit_prompt(
        &self,
        session_id: SessionId,
        request: AdmitPromptRequest,
    ) -> Result<Prompt> {
        self.post_json(
            &session_path(session_id, "/prompts"),
            &request,
            "Prompt admission",
        )
        .await
    }

    /// Runs one Attachment request, `action` naming it, within the
    /// Attachment deadline. The deadline covers the whole request — its
    /// headers and all of its body — so a Server that takes it and then
    /// stalls fails it rather than leaving it pending for good.
    async fn within_attachment_deadline<T>(
        &self,
        action: &str,
        request: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let deadline = self.timings.attachment_fetch_timeout;
        tokio::time::timeout(deadline, request)
            .await
            .unwrap_or_else(|_| Err(anyhow!("{action} timed out after {deadline:?}")))
    }

    /// Uploads an image's PNG bytes as an Attachment on the Server this
    /// transport reaches, answering with what it was stored as. A refusal is
    /// the Server's own `SessionError`, whose message a client can show as it
    /// stands.
    pub(crate) async fn upload_attachment(&self, png: Vec<u8>) -> Result<AttachmentDescriptor> {
        self.within_attachment_deadline("Attachment upload", async {
            let request = http::Request::post("/v1/attachments")
                .header(CONTENT_TYPE, "image/png")
                .body(Bytes::from(png))
                .context("build Attachment upload")?;
            let response = self.send(request, "Attachment upload").await?;
            decode_api_response(response, "Attachment upload").await
        })
        .await
    }

    /// Fetches a stored Attachment's bytes, with the type they were stored as.
    /// A body past the 5 MiB an Attachment may be is refused as it arrives,
    /// rather than read to its end: nothing the Server admitted is that large,
    /// so nothing that large is taken for an Attachment.
    pub(crate) async fn fetch_attachment(
        &self,
        attachment_id: &AttachmentId,
    ) -> Result<(String, Vec<u8>)> {
        self.within_attachment_deadline("Attachment fetch", self.read_attachment(attachment_id))
            .await
    }

    async fn read_attachment(&self, attachment_id: &AttachmentId) -> Result<(String, Vec<u8>)> {
        let response = self
            .send(
                bodiless(Method::GET, &attachment_path(attachment_id))?,
                "Attachment fetch",
            )
            .await?;
        if !response.status().is_success() {
            return Err(decode_api_error(response, "Attachment fetch").await);
        }
        let mime_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| anyhow!("Attachment fetch answered without a content type"))?
            .to_owned();
        let limit = crate::attachments::MAX_ATTACHMENT_BYTES;
        if content_length(&response).is_some_and(|length| length > limit as u64) {
            bail!("the fetched Attachment is larger than an Attachment may be");
        }
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame
                .map_err(|error| anyhow!(error))
                .context("read the fetched Attachment")?;
            let Ok(chunk) = frame.into_data() else {
                continue;
            };
            if bytes.len() + chunk.len() > limit {
                bail!("the fetched Attachment is larger than an Attachment may be");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((mime_type, bytes))
    }

    /// Whether the Server stores an Attachment under `attachment_id`, asked
    /// with a `HEAD` of its fetch route so none of its bytes are sent. Only a
    /// Not Found whose code header says `attachment_not_found` means it is
    /// not stored there: any other refusal, a Remote this Server no longer
    /// knows among them, is an error, so the Attachment is not taken for gone.
    pub(crate) async fn attachment_exists(&self, attachment_id: &AttachmentId) -> Result<bool> {
        let response = self
            .within_attachment_deadline(
                "Attachment check",
                self.send(
                    bodiless(Method::HEAD, &attachment_path(attachment_id))?,
                    "Attachment check",
                ),
            )
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(true);
        }
        let code = response
            .headers()
            .get(SESSION_ERROR_CODE_HEADER)
            .and_then(|code| code.to_str().ok())
            .unwrap_or("none");
        if status == http::StatusCode::NOT_FOUND
            && code == SessionErrorCode::AttachmentNotFound.wire_name()
        {
            return Ok(false);
        }
        Err(anyhow!(
            "Attachment check failed with HTTP {status} (error code {code})"
        ))
    }

    pub(crate) async fn update_agent_selection(
        &self,
        session_id: SessionId,
        request: UpdateAgentSelectionRequest,
    ) -> Result<AgentSelection> {
        self.post_json(
            &session_path(session_id, "/agent-selection"),
            &request,
            "Agent Selection update",
        )
        .await
    }

    pub(crate) async fn update_approval_posture(
        &self,
        session_id: SessionId,
        request: UpdateApprovalPostureRequest,
    ) -> Result<SessionApprovalPosture> {
        self.post_json(
            &session_path(session_id, "/approval-posture"),
            &request,
            "Approval Posture update",
        )
        .await
    }

    pub(crate) async fn confirm_landing_agent_selection(
        &self,
        selection: AgentSelection,
    ) -> Result<AgentSelection> {
        let response = self
            .send(
                json(Method::PUT, "/v1/landing-agent-selection", &selection)?,
                "landing Agent Selection update command",
            )
            .await?;
        decode_api_response(response, "Landing Agent Selection update").await
    }

    pub(crate) async fn promote_prompt(
        &self,
        session_id: SessionId,
        prompt_id: PromptId,
    ) -> Result<Prompt> {
        self.post_without_body(
            &session_path(
                session_id,
                &format!("/prompts/{}/promote", segment(prompt_id)),
            ),
            "Prompt promotion",
        )
        .await
    }

    pub(crate) async fn cancel_prompt(
        &self,
        session_id: SessionId,
        prompt_id: PromptId,
    ) -> Result<Prompt> {
        self.post_without_body(
            &session_path(
                session_id,
                &format!("/prompts/{}/cancel", segment(prompt_id)),
            ),
            "Prompt cancellation",
        )
        .await
    }

    pub(crate) async fn submit_questionnaire(
        &self,
        session_id: SessionId,
        id: crate::protocol::QuestionnaireId,
        submission: crate::protocol::QuestionnaireSubmission,
    ) -> Result<()> {
        self.post_json_answering_nothing(
            &session_path(session_id, &format!("/questionnaires/{}", segment(id))),
            &submission,
            "Questionnaire submission",
        )
        .await
    }

    pub(crate) async fn submit_decision(
        &self,
        session_id: SessionId,
        id: crate::protocol::ApprovalId,
        decision: crate::protocol::Decision,
    ) -> Result<()> {
        self.post_json_answering_nothing(
            &session_path(session_id, &format!("/approvals/{}/decision", segment(id))),
            &decision,
            "Approval Decision",
        )
        .await
    }

    pub(crate) async fn interrupt_session(&self, session_id: SessionId) -> Result<()> {
        self.interrupt_session_reporting_outcome(session_id)
            .await
            .map(|_| ())
    }

    pub(crate) async fn compact_session(
        &self,
        session_id: SessionId,
        request: crate::protocol::CompactSessionRequest,
    ) -> Result<()> {
        self.post_json_answering_nothing(
            &session_path(session_id, "/compact"),
            &request,
            "Session compaction",
        )
        .await
    }

    /// Interrupts the Session and reports what the interrupt did, so a client
    /// that withdrew an undelivered Prompt can return its text to the composer
    /// it was submitted from (ADR 0024). Stopping work answers with no body at
    /// all, which is [`InterruptOutcome::StoppedWork`].
    pub(crate) async fn interrupt_session_reporting_outcome(
        &self,
        session_id: SessionId,
    ) -> Result<InterruptOutcome> {
        let response = self
            .send(
                bodiless(Method::POST, &session_path(session_id, "/interrupt"))?,
                "Session interruption command",
            )
            .await?;
        if !response.status().is_success() {
            return Err(decode_api_error(response, "Session interruption").await);
        }
        let body = read_body(response)
            .await
            .context("read Session interruption outcome")?;
        if body.is_empty() {
            return Ok(InterruptOutcome::StoppedWork);
        }
        serde_json::from_slice(&body).context("decode Session interruption outcome")
    }

    pub(crate) async fn read_session(&self, session_id: SessionId) -> Result<SessionSnapshot> {
        self.get_json(&session_path(session_id, ""), "Session read")
            .await
    }

    pub(crate) async fn context_breakdown(
        &self,
        session_id: SessionId,
    ) -> Result<crate::protocol::ContextBreakdown> {
        self.get_json(&session_path(session_id, "/context"), "Context Breakdown")
            .await
    }

    pub(crate) async fn delete_session(&self, session_id: SessionId) -> Result<()> {
        let response = self
            .send(
                bodiless(Method::DELETE, &session_path(session_id, ""))?,
                "Session deletion",
            )
            .await?;
        decode_empty_api_response(response, "Session deletion").await
    }

    /// Sets a Session aside as done for now, or brings it back. Which of the
    /// two is stated rather than toggled, so a client acting on a listing that
    /// has moved on cannot flip a Session it meant to leave alone.
    pub(crate) async fn settle_session(
        &self,
        session_id: SessionId,
        settled: bool,
    ) -> Result<SessionSummary> {
        self.post_json(
            &session_path(session_id, "/settlement"),
            &SettleSessionRequest { settled },
            "Session settlement",
        )
        .await
    }

    /// Sets a Session's Icon to a user's own choice from the Icon Catalog by
    /// name, on the Server this transport reaches: the Session's Origin,
    /// wherever a caller turned it toward that.
    pub(crate) async fn set_session_icon(
        &self,
        session_id: SessionId,
        icon: &str,
    ) -> Result<SessionSummary> {
        self.post_json(
            &session_path(session_id, "/icon"),
            &SetSessionIconRequest {
                icon: icon.to_owned(),
            },
            "Session Icon",
        )
        .await
    }

    /// Renames a Session to a Title the user wrote, on its Origin.
    pub(crate) async fn set_session_title(
        &self,
        session_id: SessionId,
        title: &str,
    ) -> Result<SessionSummary> {
        self.post_json(
            &session_path(session_id, "/title"),
            &SetSessionTitleRequest {
                title: title.to_owned(),
            },
            "Session Title",
        )
        .await
    }

    /// Moves a Session to another directory of its Origin, which reads the
    /// destination against its own directories.
    pub(crate) async fn move_session(
        &self,
        session_id: SessionId,
        request: &MoveSessionRequest,
    ) -> Result<MoveOutcome> {
        self.post_json(&session_path(session_id, "/move"), request, "Session Move")
            .await
    }

    /// Asks the Session's Origin whether it would derive a Title proposal
    /// for it, as the derivation Setting in force there decides.
    pub(crate) async fn session_title_proposal_offered(
        &self,
        session_id: SessionId,
    ) -> Result<bool> {
        self.get_json::<SessionTitleProposalOffer>(
            &session_path(session_id, "/title/proposal"),
            "Title proposal offer",
        )
        .await
        .map(|offer| offer.offered)
    }

    /// Asks the Session's Origin to derive a Title proposal from its Prompts
    /// so far. Dropping the future abandons the request, and the Errand
    /// behind it with it.
    pub(crate) async fn propose_session_title(&self, session_id: SessionId) -> Result<String> {
        self.post_without_body::<SessionTitleProposal>(
            &session_path(session_id, "/title/proposal"),
            "Title proposal",
        )
        .await
        .map(|proposal| proposal.title)
    }

    /// Sets a Workspace's Icon to a user's own choice from the Icon Catalog by
    /// name, on the Server this transport reaches: the Workspace's Origin,
    /// wherever a caller turned it toward that. The Workspace's identity
    /// travels in the request body rather than a path segment, since a
    /// `WorkspaceId`'s inner string may itself contain path characters.
    pub(crate) async fn set_workspace_icon(
        &self,
        workspace_id: &WorkspaceId,
        icon: &str,
    ) -> Result<()> {
        self.post_json_answering_nothing(
            "/v1/workspaces/icon",
            &SetWorkspaceIconRequest {
                workspace_id: workspace_id.clone(),
                icon: icon.to_owned(),
            },
            "Workspace Icon",
        )
        .await
    }

    /// Sets a Workspace's Description, or clears it where the text is blank,
    /// on the Workspace's Origin, exactly as [`Self::set_workspace_icon`]
    /// sets its Icon.
    pub(crate) async fn set_workspace_description(
        &self,
        workspace_id: &WorkspaceId,
        path: Option<&Path>,
        description: &str,
    ) -> Result<()> {
        self.post_json_answering_nothing(
            "/v1/workspaces/description",
            &SetWorkspaceDescriptionRequest {
                workspace_id: workspace_id.clone(),
                path: path.map(Path::to_path_buf),
                description: description.to_owned(),
            },
            "Workspace Description",
        )
        .await
    }

    /// Reports that this Client has the Session open in its main view.
    pub(crate) async fn view_session(
        &self,
        session_id: SessionId,
        request: ViewSessionRequest,
    ) -> Result<SessionSummary> {
        self.post_json(
            &session_path(session_id, "/viewed"),
            &request,
            "Session viewed",
        )
        .await
    }

    pub(crate) async fn list_workspace_sessions(
        &self,
        workspace: Option<&WorkspaceId>,
    ) -> Result<Vec<SessionListItem>> {
        let path = match workspace {
            Some(workspace) => with_query("/v1/sessions", &[("workspace_id", &workspace.0)])?,
            None => "/v1/sessions".to_owned(),
        };
        self.get_json(&path, "Session listing").await
    }

    pub(crate) async fn list_sessions(
        &self,
        workspace: Option<&Path>,
    ) -> Result<Vec<SessionListItem>> {
        let path = match workspace {
            Some(workspace) => with_query("/v1/sessions", &[("workspace", workspace)])?,
            None => "/v1/sessions".to_owned(),
        };
        self.get_json(&path, "Session listing").await
    }

    pub(crate) async fn list_workspaces(&self) -> Result<WorkspaceListing> {
        self.get_json("/v1/workspaces", "Workspace listing").await
    }

    /// Sends `request` over the transport, a failure to reach the Server at
    /// all said to be one of sending `operation`.
    async fn send(
        &self,
        request: http::Request<Bytes>,
        operation: &str,
    ) -> Result<TransportResponse> {
        self.transport
            .send(request)
            .await
            .map_err(|failure| unanswered(failure).context(format!("send {operation}")))
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str, operation: &str) -> Result<T> {
        let response = self.send(bodiless(Method::GET, path)?, operation).await?;
        decode_api_response(response, operation).await
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &(impl Serialize + ?Sized),
        operation: &str,
    ) -> Result<T> {
        let response = self
            .send(
                json(Method::POST, path, body)?,
                &format!("{operation} command"),
            )
            .await?;
        decode_api_response(response, operation).await
    }

    async fn post_json_answering_nothing(
        &self,
        path: &str,
        body: &(impl Serialize + ?Sized),
        operation: &str,
    ) -> Result<()> {
        let response = self
            .send(json(Method::POST, path, body)?, operation)
            .await?;
        decode_empty_api_response(response, operation).await
    }

    async fn post_without_body<T: DeserializeOwned>(
        &self,
        path: &str,
        operation: &str,
    ) -> Result<T> {
        let response = self
            .send(
                bodiless(Method::POST, path)?,
                &format!("{operation} command"),
            )
            .await?;
        decode_api_response(response, operation).await
    }
}

/// A failure the transport answers in a Server's place: no Server was
/// reached to answer at all, so nothing refused the command. It is no
/// [`SessionError`], so a caller telling a Server's refusal from a failure
/// to ask finds none here, as it found none where loopback was asked
/// directly. Its message carries the cause the transport gave.
#[derive(Debug)]
pub(crate) struct NotReached(SessionError);

impl std::fmt::Display for NotReached {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0.message)
    }
}

impl std::error::Error for NotReached {}

/// A transport's `failure` as a command's error: one it answered in a
/// Server's place is [`NotReached`], and any other is the refusal it says,
/// as a Pairing's, answered for a Remote, is today through the proxy.
fn unanswered(failure: SessionError) -> anyhow::Error {
    match failure.code {
        SessionErrorCode::ServerUnreachable | SessionErrorCode::ServerReplaced => {
            anyhow::Error::new(NotReached(failure))
        }
        _ => anyhow::Error::new(failure),
    }
}

/// What a path segment holds, every byte but the unreserved encoded, so an
/// id spells one segment whatever it holds.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn segment(value: impl Display) -> String {
    utf8_percent_encode(&value.to_string(), SEGMENT).to_string()
}

/// The path of `rest` beneath the Session `session_id`'s own route.
fn session_path(session_id: SessionId, rest: &str) -> String {
    format!("/v1/sessions/{}{rest}", segment(session_id))
}

fn attachment_path(attachment_id: &AttachmentId) -> String {
    format!("/v1/attachments/{}", segment(attachment_id))
}

/// `path` with `query` encoded as its query string.
fn with_query(path: &str, query: &impl Serialize) -> Result<String> {
    let query = serde_urlencoded::to_string(query).context("encode the request's query")?;
    Ok(if query.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{query}")
    })
}

/// A request to `path` carrying nothing, refused where `path` is no
/// request URI at all: one too long to be, say, as a path a reader pasted
/// into a query can make it.
pub(super) fn bodiless(method: Method, path: &str) -> Result<http::Request<Bytes>> {
    http::Request::builder()
        .method(method)
        .uri(path)
        .body(Bytes::new())
        .context("build the request")
}

/// A request to `path` carrying `body` as JSON.
pub(super) fn json(
    method: Method,
    path: &str,
    body: &(impl Serialize + ?Sized),
) -> Result<http::Request<Bytes>> {
    let body = serde_json::to_vec(body).context("encode the request's body")?;
    http::Request::builder()
        .method(method)
        .uri(path)
        .header(CONTENT_TYPE, "application/json")
        .body(Bytes::from(body))
        .context("build the request")
}

/// The length an answer says its body is, where it says.
fn content_length(response: &TransportResponse) -> Option<u64> {
    use hyper::body::Body as _;

    response
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok()?.parse().ok())
        .or_else(|| response.body().size_hint().exact())
}

async fn read_body(response: TransportResponse) -> Result<Bytes> {
    Ok(response
        .into_body()
        .collect()
        .await
        .map_err(|error| anyhow!(error))?
        .to_bytes())
}

/// What a command was answered: the JSON of a `T`, or the Server's refusal.
pub(super) async fn decode_api_response<T: DeserializeOwned>(
    response: TransportResponse,
    operation: &str,
) -> Result<T> {
    if !response.status().is_success() {
        return Err(decode_api_error(response, operation).await);
    }
    let body = read_body(response)
        .await
        .with_context(|| format!("decode {operation} response"))?;
    serde_json::from_slice(&body).with_context(|| format!("decode {operation} response"))
}

/// Whether a command answering nothing was taken, or the Server's refusal.
pub(super) async fn decode_empty_api_response(
    response: TransportResponse,
    operation: &str,
) -> Result<()> {
    if response.status().is_success() {
        return Ok(());
    }
    Err(decode_api_error(response, operation).await)
}

/// The Server's refusal of `operation`, as the `SessionError` it sent where
/// it sent one.
pub(super) async fn decode_api_error(
    response: TransportResponse,
    operation: &str,
) -> anyhow::Error {
    let status = response.status();
    read_body(response)
        .await
        .ok()
        .and_then(|body| serde_json::from_slice::<SessionError>(&body).ok())
        .map_or_else(
            || anyhow!("{operation} failed with HTTP {status}"),
            |error| anyhow!(error),
        )
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Mutex};

    use tokio::sync::watch;

    use super::*;
    use crate::{
        managed_client::{
            loopback::LoopbackTransport,
            stream_fixture::open_stream_of,
            transport::{TryNow, full_body},
        },
        protocol::{
            ApprovalId, CheckoutAssociation, CheckoutBranchOutcome, CheckoutId, CheckoutKind,
            CheckoutRemovalInspection, CheckoutRemovalPreview, CheckoutRemovalTarget,
            CheckoutSummary, CompactSessionRequest, Decision, ExecutionDirectory, InitialPrompt,
            InsightsQuery, ListDirectoryRequest, ModelId, MoveDestination, Outlook, PreparationId,
            PrepareCheckoutRequest, PromptDelivery, ProviderId, QuestionnaireId,
            QuestionnaireSubmission, Repository, RepositoryId, RepositoryLocation,
            RuntimeDescriptor, SessionTimestamp, SourceControlAvailability,
            SourceControlCapabilities, UsageMetricsBreakdown, UsageMetricsInterval,
            UsageMetricsQuery, ViewSessionOperationId,
        },
    };

    /// A transport that answers every request with an empty success, and
    /// keeps what it was asked.
    struct Recorder {
        asked: Mutex<Vec<(Method, http::Uri)>>,
        count: watch::Sender<usize>,
    }

    impl Recorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                asked: Mutex::new(Vec::new()),
                count: watch::Sender::new(0),
            })
        }

        /// What it has been asked since last taken.
        fn take(&self) -> Vec<(Method, http::Uri)> {
            std::mem::take(&mut *self.asked.lock().expect("the record lock"))
        }

        /// Waits until it has been asked something since last taken.
        async fn asked_something(&self) {
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
        async fn send(
            &self,
            request: http::Request<Bytes>,
        ) -> Result<TransportResponse, SessionError> {
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

    fn timings() -> SessionCommandTimings {
        SessionCommandTimings {
            initial_recovery_backoff: Duration::from_millis(1),
            max_recovery_backoff: Duration::from_millis(1),
            attachment_fetch_timeout: Duration::from_secs(5),
        }
    }

    /// Every shared command and reader, each asked once with what it takes.
    /// A command added to [`SessionCommands`] is added here, so the walk
    /// judges every route the shared commands send.
    #[allow(clippy::too_many_lines)]
    async fn walk_every_command(
        commands: &SessionCommands,
        recorder: &Recorder,
        judge: impl Fn(&'static str, Vec<(Method, http::Uri)>),
    ) {
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
            "list_models" => commands.list_models().await,
            "refresh_models" => commands.refresh_models().await,
            "warm_models" => commands.warm_models().await,
            "preview_checkout_removal" => commands.preview_checkout_removal(target).await,
            "remove_checkout" => commands.remove_checkout(
                crate::protocol::RemoveCheckoutRequest { preview, force: false },
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
                    operation_id: crate::protocol::AgentSelectionOperationId::new(),
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

    /// The shared commands are those a Peer may ask: each one's route is
    /// classed as the API by the Serving side's own route check, so none of
    /// them is refused when it runs over a Pairing.
    #[tokio::test]
    async fn every_shared_command_asks_what_a_peer_may_ask() {
        let recorder = Recorder::new();
        let commands = SessionCommands::new(recorder.clone(), timings());

        walk_every_command(&commands, &recorder, |name, asked| {
            assert!(!asked.is_empty(), "{name} asks nothing");
            for (method, uri) in asked {
                assert!(
                    crate::serving::a_peer_may_ask(&method, &uri),
                    "{name} asks {method} {uri}, which a Peer may not"
                );
            }
        })
        .await;
    }

    /// An id or a path a command carries spells one segment of its route, or
    /// one value of its query, whatever it holds.
    #[tokio::test]
    async fn what_a_command_carries_is_encoded_whole() {
        let recorder = Recorder::new();
        let commands = SessionCommands::new(recorder.clone(), timings());

        let _ = commands
            .fetch_attachment(&AttachmentId::new("a/b?c#d"))
            .await;
        let _ = commands
            .list_sessions(Some(Path::new("work tree/a&b=c")))
            .await;

        let asked: Vec<_> = recorder
            .take()
            .into_iter()
            .map(|(method, uri)| format!("{method} {uri}"))
            .collect();
        assert_eq!(
            asked,
            [
                "GET /v1/attachments/a%2Fb%3Fc%23d",
                "GET /v1/sessions?workspace=work+tree%2Fa%26b%3Dc",
            ]
        );
    }

    /// A path too long for any request URI — one a reader pasted into the
    /// Directory Browser, say — fails the listing rather than its task, and
    /// nothing is sent.
    #[tokio::test]
    async fn a_listing_too_long_to_ask_is_refused_rather_than_sent() {
        let recorder = Recorder::new();
        let commands = SessionCommands::new(recorder.clone(), timings());

        let error = commands
            .list_directory(ListDirectoryRequest {
                path: PathBuf::from("a".repeat(70_000)),
                base: None,
            })
            .await
            .expect_err("no request URI holds a query that long");

        assert!(
            format!("{error:#}").starts_with("build the request"),
            "{error:#}"
        );
        assert!(recorder.take().is_empty(), "nothing is sent");
    }

    /// Turned toward a Remote, a command reaches it beneath the local
    /// Server's proxy route for it, and the Remote's own Session or
    /// Workspace route there — the Decision, the Icon, the Description and
    /// the Sidekick Workspace among them.
    #[tokio::test]
    async fn a_command_turned_toward_a_remote_asks_the_remotes_own_route() {
        use axum::{Router, extract::Request, routing::any};

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the proxy fixture");
        let address = listener.local_addr().expect("read the fixture address");
        let (asked_tx, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let app = Router::new().fallback(any(move |request: Request| {
            let _ = asked_tx.send(format!("{} {}", request.method(), request.uri()));
            async { http::StatusCode::NO_CONTENT }
        }));
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve the proxy fixture");
        });
        let (_descriptor, descriptor) = watch::channel(RuntimeDescriptor::new(
            format!("http://{address}"),
            "proxy-fixture-token".to_owned(),
            crate::protocol::ServerIdentity {
                instance_id: uuid::Uuid::new_v4(),
                pid: std::process::id(),
                protocol_version: crate::protocol::PROTOCOL_VERSION,
                build_identity: "proxy-fixture".to_owned(),
            },
        ));
        let commands = SessionCommands::new(
            Arc::new(LoopbackTransport::new(
                crate::runtime::loopback_http_client(),
                descriptor,
                Outlook::Remote("workstation".into()),
            )),
            timings(),
        );
        let session = SessionId::new();
        let approval = ApprovalId::new();
        let workspace = WorkspaceId("workspace".to_owned());

        commands
            .submit_decision(session, approval, Decision::Accept)
            .await
            .expect("the proxy takes the Decision");
        let _ = commands.set_session_icon(session, "star").await;
        commands
            .set_workspace_icon(&workspace, "star")
            .await
            .expect("the proxy takes the Icon");
        commands
            .set_workspace_description(&workspace, None, "Notes")
            .await
            .expect("the proxy takes the Description");
        let _ = commands.sidekick_workspace().await;

        let mut routes = Vec::new();
        while let Ok(route) = asked.try_recv() {
            routes.push(route);
        }
        assert_eq!(
            routes,
            [
                format!(
                    "POST /v1/remotes/workstation/v1/sessions/{session}/approvals/{approval}/decision"
                ),
                format!("POST /v1/remotes/workstation/v1/sessions/{session}/icon"),
                "POST /v1/remotes/workstation/v1/workspaces/icon".to_owned(),
                "POST /v1/remotes/workstation/v1/workspaces/description".to_owned(),
                "POST /v1/remotes/workstation/v1/workspaces/sidekick".to_owned(),
            ]
        );
        server.abort();
    }
}
