//! The commands a Peer may ask of a Server's work, and the readers of its
//! Sessions, catalog and Subagent trees, all sent over a transport: the root
//! crate's loopback one, or the Pairing transport to a Remote reached over
//! its Pairing alone. Nothing here learns which.
//!
//! These are exactly the commands whose routes the Serving side lets a Peer
//! ask. What only a Client's own Server answers — its Settings, its
//! Onboarding, its Pairings and Relays — is its administration, which stays
//! in the root crate and is sent to that Server alone.

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

use crate::{
    SessionCatalogSubscription, SessionSubscription, SubagentTreeSubscription,
    transport::{Transport, TransportResponse},
};

/// The commands a Peer may ask of the one Server's work a transport reaches.
#[derive(Clone)]
pub struct SessionCommands {
    transport: Arc<dyn Transport>,
    timings: SessionCommandTimings,
}

/// How long the commands and readers wait: a reader's backoff between
/// reconnections, and the deadline a whole Attachment request is held to.
#[derive(Clone, Copy, Debug)]
pub struct SessionCommandTimings {
    pub initial_recovery_backoff: Duration,
    pub max_recovery_backoff: Duration,
    pub attachment_fetch_timeout: Duration,
}

impl SessionCommands {
    pub fn new(transport: Arc<dyn Transport>, timings: SessionCommandTimings) -> Self {
        Self { transport, timings }
    }

    pub fn recovery_backoff(&self) -> (Duration, Duration) {
        (
            self.timings.initial_recovery_backoff,
            self.timings.max_recovery_backoff,
        )
    }

    pub fn subscribe_catalog(&self) -> SessionCatalogSubscription {
        SessionCatalogSubscription::open_attached(
            self.transport.clone(),
            self.timings.initial_recovery_backoff,
            self.timings.max_recovery_backoff,
        )
    }

    pub fn subscribe_subagent_tree(&self, session_id: SessionId) -> SubagentTreeSubscription {
        SubagentTreeSubscription::open(
            self.transport.clone(),
            session_id,
            self.timings.initial_recovery_backoff,
            self.timings.max_recovery_backoff,
        )
    }

    /// The Session's stream, read to its end and not reopened.
    pub async fn subscribe_session(&self, session_id: SessionId) -> Result<SessionSubscription> {
        SessionSubscription::open(self.transport.as_ref(), session_id).await
    }

    /// The Session's stream, reopened whenever it drops, over the transport
    /// an attached stream runs over: one bound to the Server as it stands,
    /// where the transport can tell, so the stream ends once that Server is
    /// replaced and its reader attaches afresh.
    pub async fn attach_session(&self, session_id: SessionId) -> Result<SessionSubscription> {
        SessionSubscription::open_attached(
            self.transport.clone().attached(),
            session_id,
            self.timings.initial_recovery_backoff,
            self.timings.max_recovery_backoff,
        )
        .await
    }

    pub async fn resolve_workspace(
        &self,
        request: ResolveWorkspaceRequest,
    ) -> Result<crate::protocol::ResolvedWorkspace> {
        self.post_json("/v1/workspaces/resolve", &request, "Workspace resolution")
            .await
    }

    pub async fn list_directory(
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
    pub async fn usage_metrics(
        &self,
        query: &crate::protocol::UsageMetricsQuery,
    ) -> Result<crate::protocol::UsageMetricsReport> {
        self.post_json("/v1/usage-metrics", query, "Usage Metrics read")
            .await
    }

    /// The Insights of the Server this transport reaches, asked as its Usage
    /// Metrics are.
    pub async fn insights(
        &self,
        query: &crate::protocol::InsightsQuery,
    ) -> Result<crate::protocol::InsightsReport> {
        self.post_json("/v1/usage-metrics/insights", query, "Insights read")
            .await
    }

    /// The Sidekick Workspace of the Server this transport reaches, which
    /// that Server makes the first time it is asked for.
    pub async fn sidekick_workspace(&self) -> Result<crate::protocol::ResolvedWorkspace> {
        self.post_without_body("/v1/workspaces/sidekick", "Sidekick Workspace")
            .await
    }

    pub async fn list_skills(&self, request: SkillCatalogRequest) -> Result<SkillCatalog> {
        self.post_json("/v1/skills", &request, "Skill Catalog listing")
            .await
    }

    pub async fn refresh_skills(&self, request: SkillCatalogRequest) -> Result<SkillCatalog> {
        self.post_json("/v1/skills/refresh", &request, "Skill Catalog refresh")
            .await
    }

    pub async fn list_models(&self) -> Result<ModelCatalog> {
        self.get_json("/v1/models", "Model listing").await
    }

    pub async fn refresh_models(&self) -> Result<ModelCatalog> {
        self.post_without_body("/v1/models/refresh", "Model catalog refresh")
            .await
    }

    pub async fn warm_models(&self) -> Result<()> {
        let response = self
            .send(
                bodiless(Method::POST, "/v1/models/warm")?,
                "Model catalog warm-up",
            )
            .await?;
        decode_empty_api_response(response, "Model catalog warm-up").await
    }

    pub async fn preview_checkout_removal(
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

    pub async fn remove_checkout(
        &self,
        request: crate::protocol::RemoveCheckoutRequest,
    ) -> Result<crate::protocol::RemoveCheckoutResult> {
        self.post_json("/v1/checkouts/remove", &request, "Worktree removal")
            .await
    }

    pub async fn prepare_checkout(
        &self,
        request: crate::protocol::PrepareCheckoutRequest,
    ) -> Result<crate::protocol::PrepareCheckoutResult> {
        self.post_json("/v1/checkouts/prepare", &request, "Worktree preparation")
            .await
    }

    pub async fn create_session(&self, request: CreateSessionRequest) -> Result<SessionSnapshot> {
        self.post_json("/v1/sessions", &request, "Session creation")
            .await
    }

    pub async fn admit_prompt(
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
    pub async fn upload_attachment(&self, png: Vec<u8>) -> Result<AttachmentDescriptor> {
        self.within_attachment_deadline("Attachment upload", async {
            let request = http::Request::post(crate::protocol::ATTACHMENTS_PATH)
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
    pub async fn fetch_attachment(
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
        let limit = crate::protocol::MAX_ATTACHMENT_BYTES;
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
    pub async fn attachment_exists(&self, attachment_id: &AttachmentId) -> Result<bool> {
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

    pub async fn update_agent_selection(
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

    pub async fn update_approval_posture(
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

    pub async fn confirm_landing_agent_selection(
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

    pub async fn promote_prompt(
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

    pub async fn cancel_prompt(
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

    pub async fn submit_questionnaire(
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

    pub async fn submit_decision(
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

    pub async fn interrupt_session(&self, session_id: SessionId) -> Result<()> {
        self.interrupt_session_reporting_outcome(session_id)
            .await
            .map(|_| ())
    }

    pub async fn compact_session(
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
    pub async fn interrupt_session_reporting_outcome(
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

    pub async fn read_session(&self, session_id: SessionId) -> Result<SessionSnapshot> {
        self.get_json(&session_path(session_id, ""), "Session read")
            .await
    }

    pub async fn context_breakdown(
        &self,
        session_id: SessionId,
    ) -> Result<crate::protocol::ContextBreakdown> {
        self.get_json(&session_path(session_id, "/context"), "Context Breakdown")
            .await
    }

    pub async fn delete_session(&self, session_id: SessionId) -> Result<()> {
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
    pub async fn settle_session(
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
    pub async fn set_session_icon(
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
    pub async fn set_session_title(
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
    pub async fn move_session(
        &self,
        session_id: SessionId,
        request: &MoveSessionRequest,
    ) -> Result<MoveOutcome> {
        self.post_json(&session_path(session_id, "/move"), request, "Session Move")
            .await
    }

    /// Asks the Session's Origin whether it would derive a Title proposal
    /// for it, as the derivation Setting in force there decides.
    pub async fn session_title_proposal_offered(&self, session_id: SessionId) -> Result<bool> {
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
    pub async fn propose_session_title(&self, session_id: SessionId) -> Result<String> {
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
    pub async fn set_workspace_icon(&self, workspace_id: &WorkspaceId, icon: &str) -> Result<()> {
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
    pub async fn set_workspace_description(
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
    pub async fn view_session(
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

    pub async fn list_workspace_sessions(
        &self,
        workspace: Option<&WorkspaceId>,
    ) -> Result<Vec<SessionListItem>> {
        let path = match workspace {
            Some(workspace) => with_query("/v1/sessions", &[("workspace_id", &workspace.0)])?,
            None => "/v1/sessions".to_owned(),
        };
        self.get_json(&path, "Session listing").await
    }

    pub async fn list_sessions(&self, workspace: Option<&Path>) -> Result<Vec<SessionListItem>> {
        let path = match workspace {
            Some(workspace) => with_query("/v1/sessions", &[("workspace", workspace)])?,
            None => "/v1/sessions".to_owned(),
        };
        self.get_json(&path, "Session listing").await
    }

    pub async fn list_workspaces(&self) -> Result<WorkspaceListing> {
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
pub struct NotReached(SessionError);

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
fn bodiless(method: Method, path: &str) -> Result<http::Request<Bytes>> {
    http::Request::builder()
        .method(method)
        .uri(path)
        .body(Bytes::new())
        .context("build the request")
}

/// A request to `path` carrying `body` as JSON.
fn json(
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
    use http_body::Body as _;

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
pub async fn decode_api_response<T: DeserializeOwned>(
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
pub async fn decode_empty_api_response(response: TransportResponse, operation: &str) -> Result<()> {
    if response.status().is_success() {
        return Ok(());
    }
    Err(decode_api_error(response, operation).await)
}

/// The Server's refusal of `operation`, as the `SessionError` it sent where
/// it sent one, and as that Server being too old for it where it answered a
/// not-found saying nothing.
pub async fn decode_api_error(response: TransportResponse, operation: &str) -> anyhow::Error {
    let status = response.status();
    let error = read_body(response)
        .await
        .ok()
        .and_then(|body| serde_json::from_slice::<SessionError>(&body).ok());
    match error {
        Some(error) => anyhow!(error),
        None if status == http::StatusCode::NOT_FOUND => {
            anyhow!(crate::remote_connection::too_old_for_this())
        }
        None => anyhow!("{operation} failed with HTTP {status}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures_util::StreamExt;

    use super::*;
    use crate::{
        protocol::{ListDirectoryRequest, MAX_ATTACHMENT_BYTES},
        testing::{Answering, Recorder, streamed},
        transport::{ResponseBody, full_body},
    };

    fn timings() -> SessionCommandTimings {
        SessionCommandTimings {
            initial_recovery_backoff: Duration::from_millis(1),
            max_recovery_backoff: Duration::from_millis(1),
            attachment_fetch_timeout: Duration::from_secs(5),
        }
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

    /// Commands over `transport`, whose Attachment requests are failed at
    /// `deadline`.
    fn commands_within(transport: Arc<dyn Transport>, deadline: Duration) -> SessionCommands {
        SessionCommands::new(
            transport,
            SessionCommandTimings {
                attachment_fetch_timeout: deadline,
                ..timings()
            },
        )
    }

    /// An answer of `body`, a PNG's worth of bytes.
    fn png(body: ResponseBody) -> Result<TransportResponse, SessionError> {
        let mut response = http::Response::new(body);
        response
            .headers_mut()
            .insert(CONTENT_TYPE, http::HeaderValue::from_static("image/png"));
        Ok(response)
    }

    /// The id a request for one Attachment names.
    fn attachment_named(request: &http::Request<Bytes>) -> String {
        request
            .uri()
            .path()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned()
    }

    /// Awaits `request`, which must end by itself: a guard long past any
    /// deadline a test sets fails the test rather than letting it hang.
    async fn ends_by_itself<T>(request: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .expect("the request ends at its own deadline")
    }

    /// The deadline a stalled request is failed at.
    const DEADLINE: Duration = Duration::from_millis(50);

    #[tokio::test]
    async fn a_fetch_the_server_stalls_fails_at_its_deadline() {
        // Stalls where the id says: `headers` before sending any, and `body`
        // after its first chunk.
        let commands = commands_within(
            Answering::new(|request| async move {
                if attachment_named(&request) == "headers" {
                    std::future::pending::<()>().await;
                }
                let first = futures_util::stream::once(async { Bytes::from(vec![0_u8; 1024]) });
                png(streamed(first.chain(futures_util::stream::pending())))
            }),
            DEADLINE,
        );

        for id in ["headers", "body"] {
            let refused = ends_by_itself(commands.fetch_attachment(&AttachmentId::new(id)))
                .await
                .expect_err("a stalled fetch fails");
            assert!(
                refused
                    .to_string()
                    .contains("Attachment fetch timed out after 50ms"),
                "{id}: {refused:#}"
            );
        }
    }

    #[tokio::test]
    async fn a_fetch_finished_within_its_deadline_is_taken() {
        // Its body comes in three chunks a few milliseconds apart.
        let commands = commands_within(
            Answering::new(|_| async {
                let chunks = futures_util::stream::iter(0..3).then(|_| async {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    Bytes::from(vec![0_u8; 1024])
                });
                png(streamed(chunks))
            }),
            Duration::from_millis(500),
        );

        let (mime_type, bytes) = commands
            .fetch_attachment(&AttachmentId::new("trickled"))
            .await
            .expect("a body read to its end in time is an Attachment");
        assert_eq!(mime_type, "image/png");
        assert_eq!(bytes.len(), 3 * 1024);
    }

    #[tokio::test]
    async fn an_upload_or_a_check_the_server_stalls_fails_at_its_deadline() {
        let commands = commands_within(Answering::new(|_| std::future::pending()), DEADLINE);

        let upload = ends_by_itself(commands.upload_attachment(vec![0_u8; 16]))
            .await
            .expect_err("a stalled upload fails");
        assert!(
            upload
                .to_string()
                .contains("Attachment upload timed out after 50ms"),
            "{upload:#}"
        );
        let check = ends_by_itself(commands.attachment_exists(&AttachmentId::new("stored")))
            .await
            .expect_err("a stalled check fails");
        assert!(
            check
                .to_string()
                .contains("Attachment check timed out after 50ms"),
            "{check:#}"
        );
    }

    #[tokio::test]
    async fn a_fetch_over_the_attachment_cap_is_refused_and_one_at_it_is_taken() {
        // The id names the length: `exact` is the cap, `over` one byte past
        // it and saying so, and `streamed-over` one byte past it without
        // saying so up front.
        let commands = SessionCommands::new(
            Answering::new(|request| async move {
                match attachment_named(&request).as_str() {
                    "exact" => png(full_body(vec![0_u8; MAX_ATTACHMENT_BYTES])),
                    "over" => png(full_body(vec![0_u8; MAX_ATTACHMENT_BYTES + 1])),
                    _ => {
                        let chunks = (0..=MAX_ATTACHMENT_BYTES / 65_536)
                            .map(|_| Bytes::from(vec![0_u8; 65_536]));
                        png(streamed(futures_util::stream::iter(chunks)))
                    }
                }
            }),
            timings(),
        );

        let (mime_type, bytes) = commands
            .fetch_attachment(&AttachmentId::new("exact"))
            .await
            .expect("a body at the cap is an Attachment");
        assert_eq!(mime_type, "image/png");
        assert_eq!(bytes.len(), MAX_ATTACHMENT_BYTES);

        for id in ["over", "streamed-over"] {
            let refused = commands
                .fetch_attachment(&AttachmentId::new(id))
                .await
                .expect_err("a body past the cap is refused");
            assert!(
                refused
                    .to_string()
                    .contains("larger than an Attachment may be"),
                "{id}: {refused:#}"
            );
        }
    }

    #[tokio::test]
    async fn only_a_not_found_naming_the_attachment_reads_as_not_stored() {
        // Answers a check as the id names: stored, not stored, a Remote the
        // proxy no longer knows, a Not Found that does not say why, and a
        // failure.
        let commands = SessionCommands::new(
            Answering::new(|request| async move {
                let answered = |status: http::StatusCode, code: Option<&'static str>| {
                    let mut response = http::Response::new(full_body(Bytes::new()));
                    *response.status_mut() = status;
                    if let Some(code) = code {
                        response.headers_mut().insert(
                            SESSION_ERROR_CODE_HEADER,
                            http::HeaderValue::from_static(code),
                        );
                    }
                    Ok(response)
                };
                match attachment_named(&request).as_str() {
                    "stored" => answered(http::StatusCode::OK, None),
                    "swept" => answered(http::StatusCode::NOT_FOUND, Some("attachment_not_found")),
                    "unpaired" => answered(http::StatusCode::NOT_FOUND, Some("remote_not_found")),
                    "silent" => answered(http::StatusCode::NOT_FOUND, None),
                    _ => answered(http::StatusCode::INTERNAL_SERVER_ERROR, None),
                }
            }),
            timings(),
        );
        let exists = |id: &str| {
            let commands = commands.clone();
            let id = AttachmentId::new(id);
            async move { commands.attachment_exists(&id).await }
        };

        assert!(exists("stored").await.expect("a stored Attachment answers"));
        assert!(!exists("swept").await.expect("a missing Attachment answers"));
        for id in ["unpaired", "silent", "broken"] {
            let error = exists(id)
                .await
                .expect_err("a refusal not about the Attachment is no answer");
            assert!(
                error.to_string().contains("Attachment check failed"),
                "{id}: {error:#}"
            );
        }
    }

    /// A command a Server answers with a bare not-found, as one older than
    /// the command answers a route it does not have, is refused as that
    /// Server being too old for it, naming the side to update.
    #[tokio::test]
    async fn a_command_an_older_server_does_not_know_reads_as_that_server_being_too_old() {
        let commands = SessionCommands::new(
            Answering::new(|_| async {
                let mut response = http::Response::new(full_body(Bytes::new()));
                *response.status_mut() = http::StatusCode::NOT_FOUND;
                Ok(response)
            }),
            timings(),
        );

        let error = commands
            .read_session(SessionId::new())
            .await
            .expect_err("the Server does not know the route");

        assert_eq!(
            error.to_string(),
            "That server is too old for this; update Shaman there"
        );
        assert_eq!(
            error
                .downcast_ref::<SessionError>()
                .map(|refusal| refusal.code),
            Some(SessionErrorCode::ServerTooOld)
        );
    }

    /// What a Server answers every request with under a code a newer Server
    /// added, saying `SAID` beside it.
    fn refusing_under_an_unknown_code() -> SessionCommands {
        SessionCommands::new(
            Answering::new(|_| async {
                let body = serde_json::json!({
                    "code": "a_code_added_since_this_build",
                    "message": SAID,
                    "added_since_this_build": { "by": "a newer Server" },
                });
                let mut response = http::Response::new(full_body(body.to_string()));
                *response.status_mut() = http::StatusCode::CONFLICT;
                Ok(response)
            }),
            timings(),
        )
    }

    const SAID: &str = "That Session is being tidied away; try again in a moment";

    /// A refusal under a code a newer Server added is read as the generic
    /// code, and what a reader is shown is the message beside it.
    #[tokio::test]
    async fn a_command_refused_under_an_unknown_code_reads_as_the_generic_code_with_its_message_shown()
     {
        let error = refusing_under_an_unknown_code()
            .read_session(SessionId::new())
            .await
            .expect_err("the Server refused the read");

        assert_eq!(error.to_string(), SAID, "{error:#}");
        assert_eq!(
            error
                .downcast_ref::<SessionError>()
                .map(|refusal| refusal.code),
            Some(SessionErrorCode::Other)
        );
    }

    #[tokio::test]
    async fn a_session_stream_refused_under_an_unknown_code_shows_its_message() {
        let Err(error) = refusing_under_an_unknown_code()
            .subscribe_session(SessionId::new())
            .await
        else {
            panic!("the Server refused the stream");
        };

        assert!(format!("{error:#}").contains(SAID), "{error:#}");
    }

    /// A transport that cannot reach its Server fails a command as not
    /// reached, rather than as a Server's refusal, saying why.
    #[tokio::test]
    async fn a_command_its_transport_cannot_send_is_not_reached_rather_than_refused() {
        let error = SessionCommands::new(
            Answering::new(|_| async { Err(crate::testing::unreachable()) }),
            timings(),
        )
        .read_session(SessionId::new())
        .await
        .expect_err("nothing answered");

        assert!(error.downcast_ref::<NotReached>().is_some(), "{error:#}");
        assert!(error.downcast_ref::<SessionError>().is_none());
        assert!(
            format!("{error:#}").contains("nothing answered"),
            "{error:#}"
        );
    }
}
