//! Everything a Peer may ask of a Serving Server, by route, as this build
//! speaks it: the Serving listener's own routes, the pairing ceremony among
//! them, and the routes of the Session API it carries on to its Server's own
//! (`peer_route_class` in the root crate's `serving` classes them `Api`).
//!
//! A route is added here as it is added to the Session API, with what it
//! takes and answers. A route the Serving side refuses a Peer is no part of
//! it.

use serde_json::Value;
use shaman_pairing::{
    EnrollmentRequest, EnrollmentResponse, InvitePayload, OFFERED_RELAYS_PATH, OfferAnswer,
    OfferWord, OfferedRelays, PAIRING_FLOOR_HEADER, PAIRING_PROTOCOL_HEADER,
    PAIRING_WITHDRAWAL_PATH, PairingHealth,
};
use shaman_protocol::{
    ACT_HEADER, ATTACHMENT_CONTENT_TYPES, AUTHOR_HEADER, ActId, AdmitPromptRequest, AgentSelection,
    AttachmentDescriptor, Author, CheckoutRemovalPreview, CheckoutRemovalTarget,
    CloneWorkspaceRequest, ClonedWorkspace, CompactSessionRequest, ContextBreakdown,
    CreateSessionRequest, Decision, DirectoryListing, Health, InsightsQuery, InsightsReport,
    InterruptOutcome, ListDirectoryRequest, ListSessionsQuery, MODEL_CATALOG_EVENT, ModelCatalog,
    MoveOutcome, MoveSessionRequest, PROMPT_ADMISSION_HEADER, PROTOCOL_FLOOR, PROTOCOL_VERSION,
    PrepareCheckoutRequest, PrepareCheckoutResult, Prompt, PromptAdmission,
    QuestionnaireSubmission, RemoveCheckoutRequest, RemoveCheckoutResult, ResolveWorkspaceRequest,
    ResolvedWorkspace, SESSION_CATALOG_SNAPSHOT_EVENT, SESSION_CATALOG_UPDATED_EVENT,
    SESSION_ERROR_CODE_HEADER, SESSION_READING_PATH, SESSION_SNAPSHOT_EVENT, SESSION_UPDATED_EVENT,
    SKILL_CATALOG_UPDATED_EVENT, SUBAGENT_TREE_SNAPSHOT_EVENT, SUBAGENT_TREE_UPDATED_EVENT,
    SessionApprovalPosture, SessionCatalogQuery, SessionCatalogSnapshot, SessionCatalogUpdate,
    SessionError, SessionErrorCode, SessionListing, SessionReadingAnswer, SessionReadingQuery,
    SessionSnapshot, SessionSummary, SessionTitleProposal, SessionTitleProposalOffer,
    SessionTreeOutline, SessionUpdate, SetSessionIconRequest, SetSessionTitleRequest,
    SetWorkspaceDescriptionRequest, SetWorkspaceIconRequest, SettleSessionRequest, SkillCatalog,
    SkillCatalogRequest, SnapshotWithSummary, SubagentTreeSnapshot, SubagentTreeUpdate,
    UpdateAgentSelectionRequest, UpdateApprovalPostureRequest, UsageMetricsQuery,
    UsageMetricsReport, ViewSessionRequest, WORKSPACE_LISTING_EVENT, WorkspaceListing,
};

use crate::{HeaderEncoding, WireSchema};

/// Where the schema of the Pairing wire is written down, beside this crate.
pub const SCHEMA_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/pairing-wire.json");

/// The schema of the Pairing wire as this build speaks it.
pub fn pairing_wire() -> Value {
    let mut wire = WireSchema::new(PROTOCOL_VERSION, PROTOCOL_FLOOR);
    headers(&mut wire);
    wire.message::<InvitePayload>("invite")
        .message::<SessionError>("refusal");
    listener(&mut wire);
    workspaces(&mut wire);
    sessions(&mut wire);
    session(&mut wire);
    wire.document()
}

fn headers(wire: &mut WireSchema) {
    wire.header::<u32>(PAIRING_PROTOCOL_HEADER, HeaderEncoding::Text)
        .header::<u32>(PAIRING_FLOOR_HEADER, HeaderEncoding::Text)
        .header::<SessionErrorCode>(SESSION_ERROR_CODE_HEADER, HeaderEncoding::Text)
        .header::<Author>(AUTHOR_HEADER, HeaderEncoding::Base64UrlJson)
        .header::<ActId>(ACT_HEADER, HeaderEncoding::Text)
        .header::<PromptAdmission>(PROMPT_ADMISSION_HEADER, HeaderEncoding::Text);
}

/// The Serving listener's own routes: the ceremony that forms a Pairing, and
/// what a Peer says of it after.
fn listener(wire: &mut WireSchema) {
    wire.listener("GET /health").answer::<PairingHealth>();
    wire.listener("POST /v1/pairing/enroll")
        .body::<EnrollmentRequest>()
        .answer::<EnrollmentResponse>();
    wire.listener(&format!("POST {PAIRING_WITHDRAWAL_PATH}"));
    wire.listener(&format!("GET {OFFERED_RELAYS_PATH}"))
        .lines::<OfferedRelays>();
    wire.listener("POST /v1/pairing/offer")
        .body::<OfferWord>()
        .answer::<OfferAnswer>();
    // Each route of the Session API below, asked under this prefix.
    wire.listener("ANY /v1/pairing/proxy/{*path}");
}

/// What a Server says of itself, its catalogs, its Workspaces and their
/// checkouts.
fn workspaces(wire: &mut WireSchema) {
    wire.api("GET /health").answer::<Health>();
    wire.api("GET /v1/session-events")
        .query::<SessionCatalogQuery>()
        .event::<SessionCatalogSnapshot>(SESSION_CATALOG_SNAPSHOT_EVENT)
        .event::<SessionCatalogUpdate>(SESSION_CATALOG_UPDATED_EVENT)
        .event::<ModelCatalog>(MODEL_CATALOG_EVENT)
        .event::<SkillCatalog>(SKILL_CATALOG_UPDATED_EVENT)
        .event::<WorkspaceListing>(WORKSPACE_LISTING_EVENT);
    wire.api("GET /v1/models").answer::<ModelCatalog>();
    wire.api("POST /v1/models/refresh").answer::<ModelCatalog>();
    wire.api("POST /v1/models/warm");
    wire.api("POST /v1/skills")
        .body::<SkillCatalogRequest>()
        .answer::<SkillCatalog>();
    wire.api("POST /v1/skills/refresh")
        .body::<SkillCatalogRequest>()
        .answer::<SkillCatalog>();
    wire.api("PUT /v1/landing-agent-selection")
        .body::<AgentSelection>()
        .answer::<AgentSelection>();
    wire.api("GET /v1/workspaces").answer::<WorkspaceListing>();
    wire.api("POST /v1/workspaces/resolve")
        .body::<ResolveWorkspaceRequest>()
        .answer::<ResolvedWorkspace>();
    wire.api("GET /v1/workspaces/directory")
        .query::<ListDirectoryRequest>()
        .answer::<DirectoryListing>();
    wire.api("POST /v1/workspaces/icon")
        .body::<SetWorkspaceIconRequest>();
    wire.api("POST /v1/workspaces/description")
        .body::<SetWorkspaceDescriptionRequest>();
    wire.api("POST /v1/workspaces/clone")
        .body::<CloneWorkspaceRequest>()
        .answer::<ClonedWorkspace>();
    wire.api("POST /v1/workspaces/sidekick")
        .answer::<ResolvedWorkspace>();
    wire.api("POST /v1/checkouts/prepare")
        .body::<PrepareCheckoutRequest>()
        .answer::<PrepareCheckoutResult>();
    wire.api("POST /v1/checkouts/removal-preview")
        .body::<CheckoutRemovalTarget>()
        .answer::<CheckoutRemovalPreview>();
    wire.api("POST /v1/checkouts/remove")
        .body::<RemoveCheckoutRequest>()
        .answer::<RemoveCheckoutResult>();
    wire.api("POST /v1/attachments")
        .bytes_body(&ATTACHMENT_CONTENT_TYPES)
        .answer::<AttachmentDescriptor>();
    wire.api("GET /v1/attachments/{attachment_id}")
        .bytes_answer(&ATTACHMENT_CONTENT_TYPES);
    wire.api("HEAD /v1/attachments/{attachment_id}");
    wire.api("POST /v1/usage-metrics")
        .body::<UsageMetricsQuery>()
        .answer::<UsageMetricsReport>();
    wire.api("POST /v1/usage-metrics/insights")
        .body::<InsightsQuery>()
        .answer::<InsightsReport>();
    // What a Remote of the Serving Server is asked, carried on to it: the
    // same routes again.
    wire.api("ANY /v1/remotes/{name}/{*path}");
}

/// The Sessions a Server holds, listed and begun.
fn sessions(wire: &mut WireSchema) {
    wire.api("GET /v1/sessions")
        .query::<ListSessionsQuery>()
        .answer::<SessionListing>();
    wire.api("POST /v1/sessions")
        .body::<CreateSessionRequest>()
        .answer::<SessionSnapshot>();
}

/// One Session: read, followed, prompted, answered and stopped.
fn session(wire: &mut WireSchema) {
    let at = |route: &str| {
        let (method, path) = route.split_once(' ').expect("a route names its method");
        format!("{method} /v1/sessions/{{session_id}}{path}")
    };
    wire.api(&at("GET ")).answer::<SessionSnapshot>();
    wire.api(&at("DELETE "));
    wire.api(&at("GET /with-summary"))
        .answer::<SnapshotWithSummary>();
    wire.api(&at("GET /outline")).answer::<SessionTreeOutline>();
    wire.api(&format!("GET {SESSION_READING_PATH}"))
        .query::<SessionReadingQuery>()
        .answer::<SessionReadingAnswer>();
    wire.api(&at("GET /events"))
        .event::<SessionSnapshot>(SESSION_SNAPSHOT_EVENT)
        .event::<SessionUpdate>(SESSION_UPDATED_EVENT);
    wire.api(&at("GET /subagent-tree"))
        .event::<SubagentTreeSnapshot>(SUBAGENT_TREE_SNAPSHOT_EVENT)
        .event::<SubagentTreeUpdate>(SUBAGENT_TREE_UPDATED_EVENT);
    wire.api(&at("POST /agent-selection"))
        .body::<UpdateAgentSelectionRequest>()
        .answer::<AgentSelection>();
    wire.api(&at("POST /approval-posture"))
        .body::<UpdateApprovalPostureRequest>()
        .answer::<SessionApprovalPosture>();
    wire.api(&at("POST /settlement"))
        .body::<SettleSessionRequest>()
        .answer::<SessionSummary>();
    wire.api(&at("POST /icon"))
        .body::<SetSessionIconRequest>()
        .answer::<SessionSummary>();
    wire.api(&at("POST /title"))
        .body::<SetSessionTitleRequest>()
        .answer::<SessionSummary>();
    wire.api(&at("POST /move"))
        .body::<MoveSessionRequest>()
        .answer::<MoveOutcome>();
    wire.api(&at("GET /title/proposal"))
        .answer::<SessionTitleProposalOffer>();
    wire.api(&at("POST /title/proposal"))
        .answer::<SessionTitleProposal>();
    wire.api(&at("POST /viewed"))
        .body::<ViewSessionRequest>()
        .answer::<SessionSummary>();
    wire.api(&at("POST /prompts"))
        .body::<AdmitPromptRequest>()
        .answer::<Prompt>();
    wire.api(&at("POST /prompts/{prompt_id}/promote"))
        .answer::<Prompt>();
    wire.api(&at("POST /prompts/{prompt_id}/cancel"))
        .answer::<Prompt>();
    wire.api(&at("POST /questionnaires/{id}"))
        .body::<QuestionnaireSubmission>();
    wire.api(&at("POST /approvals/{id}/decision"))
        .body::<Decision>();
    wire.api(&at("POST /interrupt"))
        .answer::<InterruptOutcome>();
    wire.api(&at("POST /compact"))
        .body::<CompactSessionRequest>();
    wire.api(&at("GET /context")).answer::<ContextBreakdown>();
}
