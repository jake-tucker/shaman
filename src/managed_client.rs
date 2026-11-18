//! Client-side ownership of server discovery and event streaming.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

use crate::{
    RuntimeConfig,
    protocol::{
        AdmitPromptRequest, AgentSelection, AttachmentDescriptor, AttachmentId,
        CheckoutStateChanged, CreateSessionRequest, Health, InterruptOutcome, InvitePreview,
        IssueInviteRequest, IssuedInvite, LifecycleState, ModelCatalog, MoveOutcome,
        MoveSessionRequest, Outlook, Peer, Prompt, PromptId, RedeemInviteRequest, Relay,
        RelayListing, RelayLogin, RelayMachines, RelayRemoval, Remote, RemoteHealth, RemoteRemoval,
        ResolveWorkspaceRequest, RuntimeDescriptor, ServerShutdown, SessionApprovalPosture,
        SessionCatalogSnapshot, SessionCreated, SessionDeleted, SessionId, SessionListItem,
        SessionMonitoringChanged, SessionRemoteSubsessionsChanged, SessionSettlementChanged,
        SessionSnapshot, SessionStandingInputsChanged, SessionSummary, SessionTitleChanged,
        SessionUsageChanged, SessionWorkingChanged, SettingMutation, SettingsSnapshot,
        ShutdownReason, SkillCatalog, SkillCatalogRequest, UpdateAgentSelectionRequest,
        UpdateApprovalPostureRequest, ViewSessionRequest, WorkspaceDescriptionChanged,
        WorkspaceIconChanged, WorkspaceId, WorkspaceListing,
    },
};

mod administration;
mod event_stream;
mod keeper;
mod launcher;
mod lifecycle;
mod loopback;
mod recovery;
mod session_projection;

pub(crate) use administration::Administration;
pub use keeper::{KeeperTimings, keep_server};
pub use launcher::StoppedDuringLaunch;
#[doc(hidden)]
pub use launcher::launch_detached;
pub(crate) use session_projection::SessionProjection;
pub(crate) use shaman_client::{
    RecoveryBackoff, SESSION_EVENT_CAPACITY, SessionCommandTimings, SessionCommands,
};
pub use shaman_client::{
    RecoveryStatus, SessionCatalogEvent, SessionEvent, SessionStreamError, SessionSubscription,
    SubagentTreeEvent, SubagentTreeSubscription,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(2);
const INITIAL_RECOVERY_BACKOFF: Duration = Duration::from_millis(50);
const MAX_RECOVERY_BACKOFF: Duration = Duration::from_secs(5);
const ATTACHMENT_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct ManagedClientConfig {
    runtime: RuntimeConfig,
    server_executable: PathBuf,
    server_environment: Vec<(OsString, OsString)>,
    startup_timeout: Duration,
    stop_timeout: Duration,
    health_check_timeout: Duration,
    initial_readiness_interval: Duration,
    max_readiness_interval: Duration,
    initial_recovery_backoff: Duration,
    max_recovery_backoff: Duration,
    attachment_fetch_timeout: Duration,
    /// How long a server this client launches waits for the channel's
    /// election lock, where a test holds one in its election.
    election_handoff: Option<Duration>,
}

impl ManagedClientConfig {
    pub fn new(state_base_dir: impl AsRef<Path>, channel: impl Into<String>) -> Result<Self> {
        Ok(Self {
            runtime: RuntimeConfig::new(state_base_dir, channel)?,
            server_executable: std::env::current_exe().context("find current Shaman executable")?,
            server_environment: Vec::new(),
            startup_timeout: STARTUP_TIMEOUT,
            stop_timeout: STOP_TIMEOUT,
            health_check_timeout: HEALTH_CHECK_TIMEOUT,
            initial_readiness_interval: Duration::from_millis(5),
            max_readiness_interval: Duration::from_millis(50),
            initial_recovery_backoff: INITIAL_RECOVERY_BACKOFF,
            max_recovery_backoff: MAX_RECOVERY_BACKOFF,
            attachment_fetch_timeout: ATTACHMENT_FETCH_TIMEOUT,
            election_handoff: None,
        })
    }

    pub fn with_server_executable(mut self, executable: impl Into<PathBuf>) -> Self {
        self.server_executable = executable.into();
        self
    }

    /// Sets `key` to `value` in the environment of every server this client
    /// launches, over the environment it would otherwise inherit from this
    /// process. A launched server outlives its client and runs whatever
    /// Providers it finds there, so this is how a test keeps a server it
    /// causes to launch away from the Providers installed on its machine —
    /// something no other part of the configuration reaches.
    pub fn with_server_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.server_environment.push((key.into(), value.into()));
        self
    }

    /// Bounds how long connecting waits for a server to become ready; injectable
    /// so tests can exercise the deadline without waiting out the default.
    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    /// Bounds how long a manual stop waits for the target process and runtime
    /// registration to settle.
    pub fn with_stop_timeout(mut self, timeout: Duration) -> Self {
        self.stop_timeout = timeout;
        self
    }

    /// Bounds individual authenticated health probes used by status and
    /// shutdown settlement.
    pub fn with_health_check_timeout(mut self, timeout: Duration) -> Self {
        self.health_check_timeout = timeout;
        self
    }

    /// Overrides startup polling: double the initial interval up to the cap.
    /// Internal timing injection, independent of connection-loss recovery.
    /// Clamp to at least one millisecond so zero cannot create a busy loop.
    pub fn with_readiness_polling(mut self, initial: Duration, max: Duration) -> Self {
        self.initial_readiness_interval = initial.max(Duration::from_millis(1));
        self.max_readiness_interval = max.max(self.initial_readiness_interval);
        self
    }

    /// Overrides the crash-recovery retry schedule (initial wait, doubling up to
    /// the cap); injectable so tests can observe the schedule at millisecond scale.
    pub fn with_recovery_backoff(mut self, initial: Duration, max: Duration) -> Self {
        self.initial_recovery_backoff = initial;
        self.max_recovery_backoff = max;
        self
    }

    /// Bounds each request that moves or checks an Attachment's bytes — its
    /// upload, its fetch, and the check that it is still stored — from
    /// sending it to reading the last of its body, so a Server that takes the
    /// request and then stalls fails it rather than leaving it pending for
    /// good; injectable so tests can meet the deadline in milliseconds.
    pub fn with_attachment_fetch_timeout(mut self, timeout: Duration) -> Self {
        self.attachment_fetch_timeout = timeout;
        self
    }

    /// Bounds how long a server this client launches waits for the
    /// channel's election lock to come free before conceding the channel to
    /// whoever holds it; injectable so a test can hold a launched server in
    /// its election for as long as it needs to, which the default second
    /// does not allow.
    pub fn with_election_handoff(mut self, handoff: Duration) -> Self {
        self.election_handoff = Some(handoff);
        self
    }

    pub fn with_data_dir(mut self, data_base_dir: impl AsRef<Path>) -> Self {
        self.runtime = self.runtime.with_data_dir(data_base_dir);
        self
    }

    /// Points a server this client launches at the Config Document root,
    /// mirroring how `SHAMAN_CONFIG_DIR` points a real run at one.
    pub fn with_config_dir(mut self, config_dir: impl AsRef<Path>) -> Self {
        self.runtime = self.runtime.with_config_dir(config_dir);
        self
    }

    /// Has a server this client launches keep a new identity key in
    /// `store` whatever its build, as `SHAMAN_IDENTITY_STORE` asks of a real
    /// run.
    pub fn with_identity_store(mut self, store: crate::IdentityStoreChoice) -> Self {
        self.runtime = self.runtime.with_identity_store(store);
        self
    }

    pub fn runtime(&self) -> &RuntimeConfig {
        &self.runtime
    }

    pub fn state_dir(&self) -> &Path {
        self.runtime.state_dir()
    }

    pub fn data_dir(&self) -> &Path {
        self.runtime.data_dir()
    }

    pub fn channel(&self) -> &str {
        self.runtime.channel()
    }

    fn descriptor_path(&self) -> PathBuf {
        self.runtime.descriptor_path()
    }

    fn create_private_runtime_dir(&self) -> Result<PathBuf> {
        self.runtime.create_private_runtime_dir()
    }

    fn lock_path(&self) -> PathBuf {
        self.runtime.lock_path()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedEvent {
    Connecting,
    Connected(Health),
    /// The server's effective-settings view: pushed right after every connect
    /// and again whenever the server replaces it.
    SettingsSnapshot(SettingsSnapshot),
    /// The server's Model Catalog: pushed right after the Settings snapshot on
    /// every connect, and again whenever a Provider's discovery settles or its
    /// Enablement turns, so a client names Models without asking for them.
    ModelCatalog(ModelCatalog),
    /// A server-authoritative Skill Catalog changed after discovery or
    /// invalidation. Every attached client receives the same state transition.
    SkillCatalogUpdated(SkillCatalog),
    Recovering(RecoveryStatus),
    /// The Server's Relays and how each stands: pushed after the Model
    /// Catalog on every connect, and again whenever any of them changes, so a
    /// client follows them without asking. The lifecycle stream is the local
    /// Server's, so they are always its own.
    Relays(crate::protocol::RelayListing),
    /// How the Server's Serving listener stands: pushed after its Relays on
    /// every connect, and again whenever it changes, so a client offers the
    /// machine's own addresses in an Invite only while the listener listens.
    ServingListener(crate::protocol::ListenerState),
    /// How the Server's latest short code stands, as it connects and again
    /// whenever that changes. Never the code.
    ShortCode(crate::protocol::ShortCodeStanding),
    /// How the Server's latest Invite stands, as it connects, once one has been
    /// issued, and again as that changes.
    InviteStanding(crate::protocol::InviteStanding),
    /// The offers to pair back the Server holds and has made, as it connects
    /// where there are any, and again whenever they change.
    PairBack(crate::protocol::PairBackOffers),
    /// The Server's Workspace listing, as its `GET /v1/workspaces` answers
    /// it: pushed on its Session catalog stream after the Model Catalog on
    /// every connect, and again whenever what it lists moves (ADR 0052), so
    /// a client holds each Server's listing before it asks. It arrives from
    /// whichever Server's catalog the client follows.
    WorkspaceListing(crate::protocol::WorkspaceListing),
    /// Whether the Server has been onboarded: pushed after how its Serving
    /// listener stands on every connect, and again the moment any Client marks
    /// it so, so each closes the Onboarding it offered. A Client reads it in
    /// its health too, but that is read before the stream opens; this is the
    /// word that holds from then on.
    Onboarded(bool),
    /// A Remote catalog stream resumed after a transient link failure.
    RemoteRecovered,
    /// A Remote rejected further use of its Pairing. Unlike a transient drop,
    /// this ends the current Outlook and must not be retried automatically.
    RemoteFailed {
        status: crate::protocol::RemoteStatus,
        message: String,
    },
    ServerShutdown(ServerShutdown),
    /// A Session joined the catalog. It carries an id and no more, so a
    /// surface listing Sessions answers it by asking for the listing the new
    /// row is drawn from.
    SessionCreated(SessionCreated),
    SessionCatalogInvalidated {
        session_id: SessionId,
    },
    CheckoutStateChanged(CheckoutStateChanged),
    SessionDeleted(SessionDeleted),
    /// A Session's derived Title and Icon landed. It arrives for every Session
    /// the server holds, open or not, because the picker lists Sessions this
    /// client has never opened.
    SessionTitleChanged(SessionTitleChanged),
    /// A Session was set aside as done for now, or brought back. It arrives on
    /// the same terms as a Title change, and for the same reason: every client
    /// lists the Session, and only some have it open.
    SessionSettlementChanged(SessionSettlementChanged),
    /// A Session's latest Turn began or settled, moving what a listing says
    /// live work has been running for. It arrives on the same terms as a Title
    /// change, and for the same reason: every client lists the Session, and
    /// only some have it open.
    SessionWorkingChanged(SessionWorkingChanged),
    /// A Session began or stopped Monitoring. It arrives on the same terms as
    /// a Working change, and for the same reason; unlike one it moves no
    /// Session's last activity, since a Watch starting or settling is no work
    /// of the Agent's.
    SessionMonitoringChanged(SessionMonitoringChanged),
    /// A Session's latest Turn Settled, replacing the facts from which every
    /// client derives its Sidebar Standing.
    SessionStandingInputsChanged(SessionStandingInputsChanged),
    /// A Session's total Usage moved, its own Turns and its Subagent subtree
    /// counted together. It arrives on the same terms as a Working change, so
    /// a client listing Sessions it has never opened can state what each of
    /// them has consumed.
    SessionUsageChanged(SessionUsageChanged),
    /// A Workspace's Icon landed, or — once issue #360 lands a way to choose
    /// one — changed. It names no Session, unlike every other catalog event
    /// here: a Workspace may hold no Session this client currently lists, and
    /// the Landing draws its current Workspace's Icon whether or not it ever
    /// has.
    WorkspaceIconChanged(WorkspaceIconChanged),
    /// A Workspace's Description was derived, set, or cleared. Like a
    /// Workspace's Icon it names no Session, and like it, it moves the
    /// catalog: it belongs to the Origin whose stream carried it, and only
    /// that Origin's rows may take it.
    WorkspaceDescriptionChanged(WorkspaceDescriptionChanged),
    /// The Sessions a Sidekick's Session began on Remotes changed. It arrives
    /// on the same terms as a Title change, and for the same reason: a
    /// client hiding Subsessions reads it wherever it lists that Session.
    SessionRemoteSubsessionsChanged(SessionRemoteSubsessionsChanged),
    SessionCatalogReconciled(SessionCatalogSnapshot),
    Fatal(String),
}

impl ManagedEvent {
    /// Whether this event says anything a client draws the moment it lands. A
    /// Session another client made says an id and no more — the row it becomes
    /// comes with the listing a surface asks for in answer — so a client that
    /// draws on demand pays for the answer rather than for the announcement
    /// (ADR 0007). Everything else moves something on screen as it arrives.
    pub const fn is_drawn_on_arrival(&self) -> bool {
        // A total moving draws nothing yet either: no listing surface states
        // one, and the client with the Session open reads its total off the
        // Session's own stream.
        !matches!(self, Self::SessionCreated(_) | Self::SessionUsageChanged(_))
    }

    /// Whether this event reports the body of work moving: a Session made,
    /// retitled, deleted, set aside, brought back, worked on, left Monitoring,
    /// or a whole catalog reconciled after a reconnection. A surface listing Sessions is
    /// only as truthful as the last such change it was told about, so it asks
    /// the server again for everything the change itself does not say — for a
    /// Turn starting, the last activity the commit moved and the order a
    /// listing keeps by it.
    ///
    /// A total moving is deliberately not one of them: unlike the others it
    /// carries the whole of what it moved and moves nothing a listing is
    /// ordered or drawn by, so there is nothing left to ask the server for.
    pub const fn moves_the_session_catalog(&self) -> bool {
        matches!(
            self,
            Self::SessionCatalogInvalidated { .. }
                | Self::CheckoutStateChanged(_)
                | Self::SessionCreated(_)
                | Self::SessionDeleted(_)
                | Self::SessionTitleChanged(_)
                | Self::SessionSettlementChanged(_)
                | Self::SessionWorkingChanged(_)
                | Self::SessionMonitoringChanged(_)
                | Self::SessionStandingInputsChanged(_)
                | Self::WorkspaceIconChanged(_)
                | Self::WorkspaceDescriptionChanged(_)
                | Self::SessionRemoteSubsessionsChanged(_)
                | Self::SessionCatalogReconciled(_)
        )
    }
}

impl From<SessionCatalogEvent> for ManagedEvent {
    fn from(event: SessionCatalogEvent) -> Self {
        match event {
            SessionCatalogEvent::ModelCatalog(catalog) => Self::ModelCatalog(catalog),
            SessionCatalogEvent::SkillCatalogUpdated(catalog) => Self::SkillCatalogUpdated(catalog),
            SessionCatalogEvent::WorkspaceListing(listing) => Self::WorkspaceListing(listing),
            SessionCatalogEvent::Recovering(status) => Self::Recovering(status),
            SessionCatalogEvent::RemoteRecovered => Self::RemoteRecovered,
            SessionCatalogEvent::RemoteFailed { status, message } => {
                Self::RemoteFailed { status, message }
            }
            SessionCatalogEvent::SessionCreated(created) => Self::SessionCreated(created),
            SessionCatalogEvent::SessionCatalogInvalidated { session_id } => {
                Self::SessionCatalogInvalidated { session_id }
            }
            SessionCatalogEvent::CheckoutStateChanged(changed) => {
                Self::CheckoutStateChanged(changed)
            }
            SessionCatalogEvent::SessionDeleted(deleted) => Self::SessionDeleted(deleted),
            SessionCatalogEvent::SessionTitleChanged(changed) => Self::SessionTitleChanged(changed),
            SessionCatalogEvent::SessionSettlementChanged(changed) => {
                Self::SessionSettlementChanged(changed)
            }
            SessionCatalogEvent::SessionWorkingChanged(changed) => {
                Self::SessionWorkingChanged(changed)
            }
            SessionCatalogEvent::SessionMonitoringChanged(changed) => {
                Self::SessionMonitoringChanged(changed)
            }
            SessionCatalogEvent::SessionStandingInputsChanged(changed) => {
                Self::SessionStandingInputsChanged(changed)
            }
            SessionCatalogEvent::SessionUsageChanged(changed) => Self::SessionUsageChanged(changed),
            SessionCatalogEvent::WorkspaceIconChanged(changed) => {
                Self::WorkspaceIconChanged(changed)
            }
            SessionCatalogEvent::WorkspaceDescriptionChanged(changed) => {
                Self::WorkspaceDescriptionChanged(changed)
            }
            SessionCatalogEvent::SessionRemoteSubsessionsChanged(changed) => {
                Self::SessionRemoteSubsessionsChanged(changed)
            }
            SessionCatalogEvent::SessionCatalogReconciled(snapshot) => {
                Self::SessionCatalogReconciled(snapshot)
            }
        }
    }
}

/// A reconnecting Session catalog stream for the Server an Outlook names,
/// its events heard among the rest a Client hears. Its lifetime is the
/// interest: dropping it stops the reading and releases its connection.
pub struct SessionCatalogSubscription(shaman_client::SessionCatalogSubscription);

impl SessionCatalogSubscription {
    pub async fn next(&mut self) -> Option<ManagedEvent> {
        self.0.next().await.map(ManagedEvent::from)
    }
}

pub struct ManagedClient {
    events: mpsc::Receiver<ManagedEvent>,
    http: reqwest::Client,
    descriptor: watch::Receiver<RuntimeDescriptor>,
    initial_recovery_backoff: Duration,
    max_recovery_backoff: Duration,
    attachment_fetch_timeout: Duration,
    config_dir: Option<PathBuf>,
    upkeep: Upkeep,
}

/// What keeps a client's Server named to it for as long as the client stands.
enum Upkeep {
    /// A task following the Server, naming each new one, for a client that
    /// stays attached.
    Following(JoinHandle<()>),
    /// The Server as found, which names no other, for one that does not. Held,
    /// since a stream gives up reconnecting once nothing can name a new Server.
    Found {
        _descriptor: watch::Sender<RuntimeDescriptor>,
    },
}

/// Commands addressed to the one Server a Client's Outlook names. Remote
/// requests are encoded beneath the local Server's explicit proxy route; the
/// caller never needs to construct or understand that route.
#[derive(Clone)]
pub struct OutlookClient {
    commands: SessionCommands,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerStatus {
    Missing,
    Starting(Health),
    Ready(Health),
    Stopping(Health),
    Failed(Health),
    Stale(String),
    Unreachable(String),
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(formatter, "Shaman server missing (no runtime registration)"),
            Self::Starting(health) => write!(
                formatter,
                "Shaman server starting (pid {}, instance {})",
                health.pid, health.instance_id
            ),
            Self::Ready(health) => write!(
                formatter,
                "Shaman server ready (pid {}, instance {})",
                health.pid, health.instance_id
            ),
            Self::Stopping(health) => write!(
                formatter,
                "Shaman server stopping (pid {}, instance {})",
                health.pid, health.instance_id
            ),
            Self::Failed(health) => write!(
                formatter,
                "Shaman server failed (pid {}, instance {})",
                health.pid, health.instance_id
            ),
            Self::Stale(reason) => {
                write!(formatter, "Shaman server registration stale: {reason}")
            }
            Self::Unreachable(reason) => write!(formatter, "Shaman server unreachable: {reason}"),
        }
    }
}

impl ManagedClient {
    pub async fn connect(config: ManagedClientConfig) -> Result<Self> {
        recovery::connect(config).await
    }

    /// A client for one command: it reaches the Channel's Server just as
    /// [`Self::connect`] does, launching or replacing one where it must, and
    /// opens no stream and recovers nothing, so a Server going away ends what
    /// the command was doing and the next command starts another. Answers the
    /// Server's health too, where this is what brought it about.
    pub async fn for_one_command(config: ManagedClientConfig) -> Result<(Self, Option<Health>)> {
        recovery::for_one_command(config).await
    }

    pub async fn next(&mut self) -> Option<ManagedEvent> {
        self.events.recv().await
    }

    /// A client of no Server at all, for a test of what a client does with
    /// events it is handed rather than with requests it sends: it reports no
    /// events, and every request it sends fails.
    #[cfg(test)]
    pub(crate) fn offline() -> Self {
        Self::reaching("http://127.0.0.1:9")
    }

    /// A client of whatever answers at `base_url`, for a test standing in a
    /// Server of its own: it reports no events and its requests go there,
    /// retrying at millisecond scale for as long as the client is held.
    #[cfg(test)]
    pub(crate) fn reaching(base_url: &str) -> Self {
        let (_, events) = mpsc::channel(1);
        let (descriptor_tx, descriptor) = watch::channel(RuntimeDescriptor::new(
            base_url.to_owned(),
            "offline-token".to_owned(),
            crate::protocol::ServerIdentity {
                instance_id: uuid::Uuid::new_v4(),
                pid: std::process::id(),
                protocol_version: crate::protocol::PROTOCOL_VERSION,
                build_identity: "offline".to_owned(),
            },
        ));
        Self {
            events,
            http: crate::runtime::loopback_http_client(),
            descriptor,
            initial_recovery_backoff: Duration::from_millis(1),
            max_recovery_backoff: Duration::from_millis(1),
            attachment_fetch_timeout: ATTACHMENT_FETCH_TIMEOUT,
            config_dir: None,
            upkeep: Upkeep::Found {
                _descriptor: descriptor_tx,
            },
        }
    }

    pub(crate) fn config_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    pub fn outlook(&self, outlook: Outlook) -> OutlookClient {
        OutlookClient {
            commands: self.session_commands_for(outlook),
        }
    }

    pub async fn preview_checkout_removal(
        &self,
        target: crate::protocol::CheckoutRemovalTarget,
    ) -> Result<crate::protocol::CheckoutRemovalPreview> {
        self.session_commands()
            .preview_checkout_removal(target)
            .await
    }
    pub async fn remove_checkout(
        &self,
        request: crate::protocol::RemoveCheckoutRequest,
    ) -> Result<crate::protocol::RemoveCheckoutResult> {
        self.session_commands().remove_checkout(request).await
    }
    pub async fn prepare_checkout(
        &self,
        request: crate::protocol::PrepareCheckoutRequest,
    ) -> Result<crate::protocol::PrepareCheckoutResult> {
        self.session_commands().prepare_checkout(request).await
    }

    pub async fn create_session(&self, request: CreateSessionRequest) -> Result<SessionSnapshot> {
        self.session_commands().create_session(request).await
    }

    /// The Sidekick Workspace of this Client's own Server, made there first if
    /// it is not there yet: a Session begun in it is a Sidekick's.
    pub async fn sidekick_workspace(&self) -> Result<crate::protocol::ResolvedWorkspace> {
        self.session_commands().sidekick_workspace().await
    }

    pub async fn admit_prompt(
        &self,
        session_id: SessionId,
        request: AdmitPromptRequest,
    ) -> Result<Prompt> {
        self.session_commands()
            .admit_prompt(session_id, request)
            .await
    }

    pub async fn upload_attachment(&self, png: Vec<u8>) -> Result<AttachmentDescriptor> {
        self.session_commands().upload_attachment(png).await
    }

    pub async fn fetch_attachment(
        &self,
        attachment_id: &AttachmentId,
    ) -> Result<(String, Vec<u8>)> {
        self.session_commands()
            .fetch_attachment(attachment_id)
            .await
    }

    pub async fn attachment_exists(&self, attachment_id: &AttachmentId) -> Result<bool> {
        self.session_commands()
            .attachment_exists(attachment_id)
            .await
    }

    pub async fn update_agent_selection(
        &self,
        session_id: SessionId,
        request: UpdateAgentSelectionRequest,
    ) -> Result<AgentSelection> {
        self.session_commands()
            .update_agent_selection(session_id, request)
            .await
    }

    pub async fn update_approval_posture(
        &self,
        session_id: SessionId,
        request: UpdateApprovalPostureRequest,
    ) -> Result<SessionApprovalPosture> {
        self.session_commands()
            .update_approval_posture(session_id, request)
            .await
    }

    pub async fn confirm_landing_agent_selection(
        &self,
        selection: AgentSelection,
    ) -> Result<AgentSelection> {
        self.session_commands()
            .confirm_landing_agent_selection(selection)
            .await
    }

    /// Changes one Setting in the server's Config Document. The answer is the
    /// effective-settings snapshot the edit leaves in force, which every
    /// attached client — this one included — also receives on its event stream.
    pub async fn mutate_setting(&self, mutation: SettingMutation) -> Result<SettingsSnapshot> {
        self.administration().mutate_setting(mutation).await
    }

    /// Marks the Client's own Server onboarded, whichever way the Outlook is
    /// turned. Every attached Client — this one included — hears
    /// [`ManagedEvent::Onboarded`] the first time it is.
    pub async fn mark_onboarded(&self) -> Result<()> {
        self.administration().mark_onboarded().await
    }

    pub async fn issue_invite(&self, request: IssueInviteRequest) -> Result<IssuedInvite> {
        self.administration().issue_invite(request).await
    }

    /// Makes the Client's own Server ready to issue an Invite: Serving turned on
    /// where it was off, or asked for again where it could not start.
    pub async fn prepare_serving(&self) -> Result<crate::protocol::ServingPreparation> {
        self.administration().prepare_serving().await
    }

    /// Whether the Client's own Server is Serving, changing nothing.
    pub async fn serving_standing(&self) -> Result<crate::protocol::ServingStanding> {
        self.administration().serving_standing().await
    }

    /// An Invite from the Client's own Server offering the kinds of way asked
    /// for, for the Server to say which those are, turning Serving on first.
    pub async fn offer_invite(
        &self,
        request: crate::protocol::OfferInviteRequest,
    ) -> Result<crate::protocol::InviteOfferAnswer> {
        self.administration().offer_invite(request).await
    }

    /// A fresh short code for the Invite that stands, leaving that Invite as
    /// it is.
    pub async fn new_short_code(&self) -> Result<crate::protocol::IssuedShortCode> {
        self.administration().new_short_code().await
    }

    /// Asks a machine chosen from a Relay's list for its Invite by its code.
    pub async fn fetch_short_invite(
        &self,
        request: crate::protocol::FetchShortInviteRequest,
    ) -> Result<crate::protocol::DeliveredInvite> {
        self.administration().fetch_short_invite(request).await
    }

    pub async fn preview_invite(
        &self,
        invite: impl Into<crate::protocol::InviteText>,
    ) -> Result<InvitePreview> {
        self.administration().preview_invite(invite.into()).await
    }

    pub async fn redeem_invite(&self, request: RedeemInviteRequest) -> Result<Remote> {
        self.administration().redeem_invite(request).await
    }

    pub async fn list_peers(&self) -> Result<Vec<Peer>> {
        self.administration().list_peers().await
    }

    pub async fn list_remotes(&self) -> Result<Vec<Remote>> {
        self.administration().list_remotes().await
    }

    pub async fn probe_remote(&self, name: &str) -> Result<RemoteHealth> {
        self.administration().probe_remote(name).await
    }

    pub async fn remove_peer(&self, peer_id: &str) -> Result<()> {
        self.administration().remove_peer(peer_id).await
    }

    pub async fn remove_remote(&self, name: &str) -> Result<RemoteRemoval> {
        self.administration().remove_remote(name).await
    }

    pub async fn set_remote_icon(&self, name: &str, icon: &str) -> Result<()> {
        self.administration().set_remote_icon(name, icon).await
    }

    /// The Relays the Client's own Server holds entries for, whichever way
    /// the Outlook is turned, as they stand at the listing's revision.
    pub async fn list_relays(&self) -> Result<RelayListing> {
        self.administration().list_relays().await
    }

    /// The offers to pair back the Client's own Server holds and has made.
    pub async fn list_offers(&self) -> Result<crate::protocol::PairBackOffers> {
        self.administration().list_offers().await
    }

    /// Answers an offer the Client's own Server holds, as its user.
    pub async fn answer_offer(
        &self,
        id: uuid::Uuid,
        answer: crate::protocol::AnswerOfferRequest,
    ) -> Result<()> {
        self.administration().answer_offer(id, answer).await
    }

    /// Withdraws an offer the Client's own Server made, as its user.
    pub async fn withdraw_offer(&self, id: uuid::Uuid) -> Result<()> {
        self.administration().withdraw_offer(id).await
    }

    /// Claims of the Client's own Server, as the Client run `claimant`, the
    /// Notice of the offer `id`: whether it gave this Client that Notice, to
    /// raise, as it gives it the first Client to claim it and no other.
    pub async fn claim_offer_notice(&self, id: uuid::Uuid, claimant: uuid::Uuid) -> Result<bool> {
        self.administration().claim_offer_notice(id, claimant).await
    }

    /// Claims of the Client's own Server, as the Client run `claimant`, the
    /// Notice of the Relay at `address` coming to need a login in `lapse`:
    /// whether it gave this Client that Notice, to raise, as it gives it the
    /// first Client to claim it and no other.
    pub async fn claim_relay_login_needed_notice(
        &self,
        address: &str,
        lapse: uuid::Uuid,
        claimant: uuid::Uuid,
    ) -> Result<bool> {
        self.administration()
            .claim_relay_login_needed_notice(address, lapse, claimant)
            .await
    }

    pub async fn add_relay(&self, address: impl Into<String>) -> Result<Relay> {
        self.administration().add_relay(address.into()).await
    }

    /// Has the Client's own Server ask the Relay at `address` which other
    /// machines are logged in there, answering how it answered within the
    /// Server's bound on that.
    pub async fn list_relay_machines(&self, address: &str) -> Result<RelayMachines> {
        self.administration().list_relay_machines(address).await
    }

    /// A login at the Relay at `address`, as a reader asks for one where a Login
    /// may stand: the one under way there, or one the Client's own Server begins.
    pub async fn begin_relay_login(&self, address: &str) -> Result<RelayLogin> {
        match self.log_in_at_relay(address, true).await?.standing {
            crate::protocol::RelayLoginStanding::UnderWay { login }
            | crate::protocol::RelayLoginStanding::Begun { login } => Ok(login),
            crate::protocol::RelayLoginStanding::Standing { .. } => {
                bail!("the Server began no login at the Relay")
            }
        }
    }

    /// Has the Client's own Server log in at the Relay at `address`, which it
    /// decides: a login under way is left to go on, and one is begun only where
    /// no Login stands, or `afresh` asks for one regardless.
    pub async fn log_in_at_relay(
        &self,
        address: &str,
        afresh: bool,
    ) -> Result<crate::protocol::RelayLoginLead> {
        self.administration().log_in_at_relay(address, afresh).await
    }

    /// Waits for the latest login begun at the Relay at `address` to end,
    /// answering how it did.
    pub async fn follow_relay_login(&self, address: &str) -> Result<RelayLogin> {
        self.administration().follow_relay_login(address).await
    }

    pub async fn remove_relay(&self, address: &str) -> Result<RelayRemoval> {
        self.administration().remove_relay(address).await
    }

    /// Chooses whether the Client's own Server Serves through the Relay at
    /// `address`, answering the Relay as it then stands.
    pub async fn set_relay_serve_through(
        &self,
        address: &str,
        serve_through: bool,
    ) -> Result<Relay> {
        self.administration()
            .set_relay_serve_through(address, serve_through)
            .await
    }

    pub async fn subscribe_session(&self, session_id: SessionId) -> Result<SessionSubscription> {
        self.session_commands().subscribe_session(session_id).await
    }

    pub async fn promote_prompt(
        &self,
        session_id: SessionId,
        prompt_id: PromptId,
    ) -> Result<Prompt> {
        self.session_commands()
            .promote_prompt(session_id, prompt_id)
            .await
    }

    pub async fn cancel_prompt(
        &self,
        session_id: SessionId,
        prompt_id: PromptId,
    ) -> Result<Prompt> {
        self.session_commands()
            .cancel_prompt(session_id, prompt_id)
            .await
    }

    /// Stops what the Session is doing: its active Turn along with the
    /// Subagents it spawned, or — with no Turn active — its working Subagents
    /// alone. Interrupting a Subagent's own Session stops that one Subagent.
    pub async fn submit_questionnaire(
        &self,
        session_id: SessionId,
        id: crate::protocol::QuestionnaireId,
        submission: crate::protocol::QuestionnaireSubmission,
    ) -> Result<()> {
        self.session_commands()
            .submit_questionnaire(session_id, id, submission)
            .await
    }

    pub async fn submit_decision(
        &self,
        session_id: SessionId,
        id: crate::protocol::ApprovalId,
        decision: crate::protocol::Decision,
    ) -> Result<()> {
        self.session_commands()
            .submit_decision(session_id, id, decision)
            .await
    }

    pub async fn interrupt_session(&self, session_id: SessionId) -> Result<()> {
        self.session_commands().interrupt_session(session_id).await
    }

    /// Asks the Session's Provider to compact its context now, which begins
    /// a Turn of its own holding that Compaction (ADR 0041). A refusal carries
    /// its typed [`crate::protocol::SessionError`].
    pub async fn compact_session(
        &self,
        session_id: SessionId,
        request: crate::protocol::CompactSessionRequest,
    ) -> Result<()> {
        self.session_commands()
            .compact_session(session_id, request)
            .await
    }

    /// Asks the Session's Provider what occupies its context now. A refusal
    /// carries its typed [`crate::protocol::SessionError`].
    pub async fn context_breakdown(
        &self,
        session_id: SessionId,
    ) -> Result<crate::protocol::ContextBreakdown> {
        self.session_commands().context_breakdown(session_id).await
    }

    /// Interrupts the Session and reports what the interrupt did: stopped work,
    /// or the undelivered Prompt it withdrew (ADR 0024).
    pub async fn interrupt_session_reporting_outcome(
        &self,
        session_id: SessionId,
    ) -> Result<InterruptOutcome> {
        self.session_commands()
            .interrupt_session_reporting_outcome(session_id)
            .await
    }

    pub async fn read_session(&self, session_id: SessionId) -> Result<SessionSnapshot> {
        self.session_commands().read_session(session_id).await
    }

    pub async fn delete_session(&self, session_id: SessionId) -> Result<()> {
        self.session_commands().delete_session(session_id).await
    }

    pub async fn settle_session(
        &self,
        session_id: SessionId,
        settled: bool,
    ) -> Result<SessionSummary> {
        self.session_commands()
            .settle_session(session_id, settled)
            .await
    }

    /// Sets a Session's Icon to a user's choice from the Icon Catalog by
    /// name, refused where the Catalog does not carry it.
    pub async fn set_session_icon(
        &self,
        session_id: SessionId,
        icon: &str,
    ) -> Result<SessionSummary> {
        self.session_commands()
            .set_session_icon(session_id, icon)
            .await
    }

    /// Renames a Session to a Title the user wrote, refused where it is blank
    /// or runs past what a Title may carry.
    pub async fn set_session_title(
        &self,
        session_id: SessionId,
        title: &str,
    ) -> Result<SessionSummary> {
        self.session_commands()
            .set_session_title(session_id, title)
            .await
    }

    /// Moves a Session to another directory of its Server, refused — the
    /// Session left where it was — where the destination names no directory
    /// an Agent can work in, or the Session cannot Move now.
    pub async fn move_session(
        &self,
        session_id: SessionId,
        request: &MoveSessionRequest,
    ) -> Result<MoveOutcome> {
        self.session_commands()
            .move_session(session_id, request)
            .await
    }

    /// Whether the Session's Origin would derive a Title proposal for it.
    pub async fn session_title_proposal_offered(&self, session_id: SessionId) -> Result<bool> {
        self.session_commands()
            .session_title_proposal_offered(session_id)
            .await
    }

    /// Derives a Title proposal from a Session's Prompts so far, changing
    /// nothing about the Session.
    pub async fn propose_session_title(&self, session_id: SessionId) -> Result<String> {
        self.session_commands()
            .propose_session_title(session_id)
            .await
    }

    /// Sets a Workspace's Icon to a user's own choice from the Icon Catalog by
    /// name, refused where the Catalog does not carry it or where this Client's
    /// own local server does not know the Workspace. A Remote Workspace's own
    /// choice reaches it through [`Self::outlook`] instead, the way every
    /// other Remote-addressed command does.
    pub async fn set_workspace_icon(&self, workspace_id: &WorkspaceId, icon: &str) -> Result<()> {
        self.session_commands()
            .set_workspace_icon(workspace_id, icon)
            .await
    }

    /// Sets a Workspace's Description to the user's own text, or clears it
    /// where the text is blank, refused where this Client's own local server
    /// cannot tell the Workspace is its own or the text runs longer than a
    /// Description may. `path`, where the Workspace is presented, is what
    /// lets the server take as its own a Workspace no Session has been begun
    /// in yet. A Remote Workspace's own reaches it through [`Self::outlook`]
    /// instead, the way its chosen Icon does.
    pub async fn set_workspace_description(
        &self,
        workspace_id: &WorkspaceId,
        path: Option<&std::path::Path>,
        description: &str,
    ) -> Result<()> {
        self.session_commands()
            .set_workspace_description(workspace_id, path, description)
            .await
    }

    pub async fn view_session(
        &self,
        session_id: SessionId,
        request: ViewSessionRequest,
    ) -> Result<SessionSummary> {
        self.session_commands()
            .view_session(session_id, request)
            .await
    }

    pub async fn list_sessions(&self, workspace: Option<&Path>) -> Result<Vec<SessionListItem>> {
        self.session_commands().list_sessions(workspace).await
    }

    /// Every Workspace this Client's own Server knows, as its Workspace
    /// Registry lists them, spelled in that Server's own paths.
    pub async fn list_workspaces(&self) -> Result<WorkspaceListing> {
        self.session_commands().list_workspaces().await
    }

    pub async fn list_models(&self) -> Result<ModelCatalog> {
        self.session_commands().list_models().await
    }

    pub async fn refresh_models(&self) -> Result<ModelCatalog> {
        self.session_commands().refresh_models().await
    }

    /// Asks the server to discover the Models of every Provider it has not
    /// yet heard from this process. The answer arrives as Model Catalog
    /// events, not here.
    pub async fn warm_models(&self) -> Result<()> {
        self.session_commands().warm_models().await
    }

    pub async fn list_skills(&self, request: SkillCatalogRequest) -> Result<SkillCatalog> {
        self.session_commands().list_skills(request).await
    }

    pub async fn refresh_skills(&self, request: SkillCatalogRequest) -> Result<SkillCatalog> {
        self.session_commands().refresh_skills(request).await
    }

    pub async fn attach_session(&self, session_id: SessionId) -> Result<SessionSubscription> {
        self.session_commands().attach_session(session_id).await
    }

    /// Subscribes to the tree `session_id` belongs to on the local Server.
    pub fn subscribe_subagent_tree(&self, session_id: SessionId) -> SubagentTreeSubscription {
        self.session_commands().subscribe_subagent_tree(session_id)
    }

    /// The commands a Peer may ask, sent to this Client's own Server.
    pub(crate) fn session_commands(&self) -> SessionCommands {
        self.session_commands_for(Outlook::Local)
    }

    /// The commands a Peer may ask, sent to the Server `outlook` names over
    /// a loopback transport through this Client's own Server: the same
    /// commands and readers a thin Client runs over a Pairing transport.
    pub fn session_commands_for(&self, outlook: Outlook) -> SessionCommands {
        SessionCommands::new(
            std::sync::Arc::new(loopback::LoopbackTransport::new(
                self.http.clone(),
                self.descriptor.clone(),
                outlook,
            )),
            SessionCommandTimings {
                initial_recovery_backoff: self.initial_recovery_backoff,
                max_recovery_backoff: self.max_recovery_backoff,
                attachment_fetch_timeout: self.attachment_fetch_timeout,
            },
        )
    }

    /// The administration of this Client's own Server.
    pub(crate) fn administration(&self) -> Administration {
        Administration::new(self.http.clone(), self.descriptor.clone())
    }
}

impl OutlookClient {
    pub fn subscribe_catalog(&self) -> SessionCatalogSubscription {
        SessionCatalogSubscription(self.commands.subscribe_catalog())
    }

    /// Subscribes to the tree `session_id` belongs to, headed by its
    /// top-level Session, on the Server this Outlook names.
    pub fn subscribe_subagent_tree(&self, session_id: SessionId) -> SubagentTreeSubscription {
        self.commands.subscribe_subagent_tree(session_id)
    }

    pub async fn preview_checkout_removal(
        &self,
        target: crate::protocol::CheckoutRemovalTarget,
    ) -> Result<crate::protocol::CheckoutRemovalPreview> {
        self.commands.preview_checkout_removal(target).await
    }
    pub async fn remove_checkout(
        &self,
        request: crate::protocol::RemoveCheckoutRequest,
    ) -> Result<crate::protocol::RemoveCheckoutResult> {
        self.commands.remove_checkout(request).await
    }
    pub async fn prepare_checkout(
        &self,
        request: crate::protocol::PrepareCheckoutRequest,
    ) -> Result<crate::protocol::PrepareCheckoutResult> {
        self.commands.prepare_checkout(request).await
    }

    pub async fn create_session(&self, request: CreateSessionRequest) -> Result<SessionSnapshot> {
        self.commands.create_session(request).await
    }

    pub async fn resolve_workspace(
        &self,
        request: ResolveWorkspaceRequest,
    ) -> Result<crate::protocol::ResolvedWorkspace> {
        self.commands.resolve_workspace(request).await
    }

    /// One directory of the Server this Outlook names, read there for the
    /// Directory Browser: its root, the root's parent, and its child
    /// directories.
    pub async fn list_directory(
        &self,
        request: crate::protocol::ListDirectoryRequest,
    ) -> Result<crate::protocol::DirectoryListing> {
        self.commands.list_directory(request).await
    }

    /// The Usage Metrics of every Session living on the Server this Outlook
    /// names, in the buckets `query` cuts. A refused query carries its typed
    /// [`crate::protocol::SessionError`].
    pub async fn usage_metrics(
        &self,
        query: &crate::protocol::UsageMetricsQuery,
    ) -> Result<crate::protocol::UsageMetricsReport> {
        self.commands.usage_metrics(query).await
    }

    /// The Insights of every Session living on the Server this Outlook
    /// names, over `query`'s interval. A refused query carries its typed
    /// [`crate::protocol::SessionError`].
    pub async fn insights(
        &self,
        query: &crate::protocol::InsightsQuery,
    ) -> Result<crate::protocol::InsightsReport> {
        self.commands.insights(query).await
    }

    /// The Sidekick Workspace of the Server this Client's own Outlook names,
    /// made there first if it is not there yet.
    pub async fn sidekick_workspace(&self) -> Result<crate::protocol::ResolvedWorkspace> {
        self.commands.sidekick_workspace().await
    }

    pub async fn admit_prompt(
        &self,
        session_id: SessionId,
        request: AdmitPromptRequest,
    ) -> Result<Prompt> {
        self.commands.admit_prompt(session_id, request).await
    }

    pub async fn upload_attachment(&self, png: Vec<u8>) -> Result<AttachmentDescriptor> {
        self.commands.upload_attachment(png).await
    }

    pub async fn fetch_attachment(
        &self,
        attachment_id: &AttachmentId,
    ) -> Result<(String, Vec<u8>)> {
        self.commands.fetch_attachment(attachment_id).await
    }

    pub async fn attachment_exists(&self, attachment_id: &AttachmentId) -> Result<bool> {
        self.commands.attachment_exists(attachment_id).await
    }

    pub async fn update_agent_selection(
        &self,
        session_id: SessionId,
        request: UpdateAgentSelectionRequest,
    ) -> Result<AgentSelection> {
        self.commands
            .update_agent_selection(session_id, request)
            .await
    }

    pub async fn update_approval_posture(
        &self,
        session_id: SessionId,
        request: UpdateApprovalPostureRequest,
    ) -> Result<SessionApprovalPosture> {
        self.commands
            .update_approval_posture(session_id, request)
            .await
    }

    pub async fn confirm_landing_agent_selection(
        &self,
        selection: AgentSelection,
    ) -> Result<AgentSelection> {
        self.commands
            .confirm_landing_agent_selection(selection)
            .await
    }

    pub async fn subscribe_session(&self, session_id: SessionId) -> Result<SessionSubscription> {
        self.commands.subscribe_session(session_id).await
    }

    pub async fn promote_prompt(
        &self,
        session_id: SessionId,
        prompt_id: PromptId,
    ) -> Result<Prompt> {
        self.commands.promote_prompt(session_id, prompt_id).await
    }

    pub async fn cancel_prompt(
        &self,
        session_id: SessionId,
        prompt_id: PromptId,
    ) -> Result<Prompt> {
        self.commands.cancel_prompt(session_id, prompt_id).await
    }

    pub async fn submit_decision(
        &self,
        session_id: SessionId,
        id: crate::protocol::ApprovalId,
        decision: crate::protocol::Decision,
    ) -> Result<()> {
        self.commands
            .submit_decision(session_id, id, decision)
            .await
    }

    pub async fn interrupt_session(&self, session_id: SessionId) -> Result<()> {
        self.commands.interrupt_session(session_id).await
    }

    /// Asks the Session's Provider to compact its context now, which begins
    /// a Turn of its own holding that Compaction (ADR 0041).
    pub async fn compact_session(
        &self,
        session_id: SessionId,
        request: crate::protocol::CompactSessionRequest,
    ) -> Result<()> {
        self.commands.compact_session(session_id, request).await
    }

    /// Asks the Session's Provider what occupies its context now.
    pub async fn context_breakdown(
        &self,
        session_id: SessionId,
    ) -> Result<crate::protocol::ContextBreakdown> {
        self.commands.context_breakdown(session_id).await
    }

    /// Interrupts the Session and reports what the interrupt did: stopped work,
    /// or the undelivered Prompt it withdrew (ADR 0024).
    pub async fn interrupt_session_reporting_outcome(
        &self,
        session_id: SessionId,
    ) -> Result<InterruptOutcome> {
        self.commands
            .interrupt_session_reporting_outcome(session_id)
            .await
    }

    pub async fn read_session(&self, session_id: SessionId) -> Result<SessionSnapshot> {
        self.commands.read_session(session_id).await
    }

    pub async fn delete_session(&self, session_id: SessionId) -> Result<()> {
        self.commands.delete_session(session_id).await
    }

    pub async fn settle_session(
        &self,
        session_id: SessionId,
        settled: bool,
    ) -> Result<SessionSummary> {
        self.commands.settle_session(session_id, settled).await
    }

    /// Sets a Session's Icon to a user's choice from the Icon Catalog by
    /// name, refused where the Catalog does not carry it.
    pub async fn set_session_icon(
        &self,
        session_id: SessionId,
        icon: &str,
    ) -> Result<SessionSummary> {
        self.commands.set_session_icon(session_id, icon).await
    }

    /// Renames a Session to a Title the user wrote, refused where it is blank
    /// or runs past what a Title may carry.
    pub async fn set_session_title(
        &self,
        session_id: SessionId,
        title: &str,
    ) -> Result<SessionSummary> {
        self.commands.set_session_title(session_id, title).await
    }

    /// Moves a Session to another directory of its Server, refused — the
    /// Session left where it was — where the destination names no directory
    /// an Agent can work in, or the Session cannot Move now.
    pub async fn move_session(
        &self,
        session_id: SessionId,
        request: &MoveSessionRequest,
    ) -> Result<MoveOutcome> {
        self.commands.move_session(session_id, request).await
    }

    /// Whether the Session's Origin would derive a Title proposal for it.
    pub async fn session_title_proposal_offered(&self, session_id: SessionId) -> Result<bool> {
        self.commands
            .session_title_proposal_offered(session_id)
            .await
    }

    /// Derives a Title proposal from a Session's Prompts so far, changing
    /// nothing about the Session.
    pub async fn propose_session_title(&self, session_id: SessionId) -> Result<String> {
        self.commands.propose_session_title(session_id).await
    }

    /// Sets a Workspace's Icon to a user's own choice from the Icon Catalog by
    /// name, refused where the Catalog does not carry it or where this
    /// Outlook's own server does not know the Workspace.
    pub async fn set_workspace_icon(&self, workspace_id: &WorkspaceId, icon: &str) -> Result<()> {
        self.commands.set_workspace_icon(workspace_id, icon).await
    }

    /// Sets a Workspace's Description to the user's own text, or clears it
    /// where the text is blank, on this Outlook's own server.
    pub async fn set_workspace_description(
        &self,
        workspace_id: &WorkspaceId,
        path: Option<&std::path::Path>,
        description: &str,
    ) -> Result<()> {
        self.commands
            .set_workspace_description(workspace_id, path, description)
            .await
    }

    pub async fn view_session(
        &self,
        session_id: SessionId,
        request: ViewSessionRequest,
    ) -> Result<SessionSummary> {
        self.commands.view_session(session_id, request).await
    }

    pub async fn list_sessions(&self, workspace: Option<&Path>) -> Result<Vec<SessionListItem>> {
        self.commands.list_sessions(workspace).await
    }

    /// Every Workspace the Server this Outlook names knows, as its Workspace
    /// Registry lists them, spelled in that Server's own paths.
    pub async fn list_workspaces(&self) -> Result<WorkspaceListing> {
        self.commands.list_workspaces().await
    }

    pub async fn list_models(&self) -> Result<ModelCatalog> {
        self.commands.list_models().await
    }

    pub async fn refresh_models(&self) -> Result<ModelCatalog> {
        self.commands.refresh_models().await
    }

    pub async fn list_skills(&self, request: SkillCatalogRequest) -> Result<SkillCatalog> {
        self.commands.list_skills(request).await
    }

    pub async fn refresh_skills(&self, request: SkillCatalogRequest) -> Result<SkillCatalog> {
        self.commands.refresh_skills(request).await
    }

    pub async fn attach_session(&self, session_id: SessionId) -> Result<SessionSubscription> {
        self.commands.attach_session(session_id).await
    }
}

pub async fn start_server(config: &ManagedClientConfig) -> Result<Health> {
    let baseline = launcher::StopBaseline::read(config);
    let deadline = tokio::time::Instant::now() + config.startup_timeout;
    launcher::ensure_server(config, deadline, baseline)
        .await
        .map(|registration| registration.health)
}

pub async fn server_status(config: &ManagedClientConfig) -> Result<ServerStatus> {
    let health = match lifecycle::inspect_registration(config).await? {
        lifecycle::RegistrationInspection::Missing => return Ok(ServerStatus::Missing),
        lifecycle::RegistrationInspection::Live(registration) => registration.health,
        lifecycle::RegistrationInspection::Stale(reason) => {
            return Ok(ServerStatus::Stale(reason));
        }
        lifecycle::RegistrationInspection::Unreachable(reason) => {
            return Ok(ServerStatus::Unreachable(reason));
        }
    };
    Ok(match health.lifecycle {
        LifecycleState::Starting => ServerStatus::Starting(health),
        LifecycleState::Ready => ServerStatus::Ready(health),
        LifecycleState::Stopping => ServerStatus::Stopping(health),
        LifecycleState::Failed => ServerStatus::Failed(health),
    })
}

/// What came of a `shaman server stop`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerStop {
    /// This Server was running, and has stopped.
    Stopped(Health),
    /// No Server was running: it had crashed, say. The stop is recorded all the
    /// same, so nothing launched before it serves the Channel in its place.
    NoneRunning,
}

/// Stops the Channel's Server. Where none answers and none holds the election
/// the stop still stands, recorded as a stopping Server would record it, so a
/// Keeper or a recovering client ready to launch another does not (ADR 0002).
/// One holding the election without answering may be on its way up, so it is
/// given until the stop's deadline to answer; one that never does is hung, and
/// no stop is recorded past the lock it holds.
pub async fn stop_server(config: &ManagedClientConfig) -> Result<ServerStop> {
    let held_until = tokio::time::Instant::now() + config.stop_timeout;
    loop {
        let not_stoppable = match lifecycle::inspect_registration(config).await? {
            lifecycle::RegistrationInspection::Live(registration) => {
                lifecycle::shutdown_registered_instance(
                    config,
                    &registration,
                    ShutdownReason::Manual,
                    tokio::time::Instant::now() + config.stop_timeout,
                )
                .await?;
                return Ok(ServerStop::Stopped(registration.health));
            }
            lifecycle::RegistrationInspection::Missing => {
                "cannot stop missing Shaman server".to_owned()
            }
            lifecycle::RegistrationInspection::Stale(reason) => {
                format!("cannot stop stale Shaman server registration: {reason}")
            }
            lifecycle::RegistrationInspection::Unreachable(reason) => {
                format!("cannot stop unreachable Shaman server: {reason}")
            }
        };
        let now = tokio::time::Instant::now();
        match crate::server::stop_with_no_server(&config.runtime)? {
            crate::server::StopWithNoServer::Recorded => return Ok(ServerStop::NoneRunning),
            crate::server::StopWithNoServer::NoChannel => bail!(not_stoppable),
            crate::server::StopWithNoServer::ServerHolds if now >= held_until => {
                bail!(not_stoppable)
            }
            crate::server::StopWithNoServer::ServerHolds => {
                tokio::time::sleep_until((now + Duration::from_millis(25)).min(held_until)).await;
            }
        }
    }
}

impl Drop for ManagedClient {
    fn drop(&mut self) {
        if let Upkeep::Following(task) = &self.upkeep {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    /// The shared commands are those a Peer may ask: each one's route is
    /// classed as the API by the Serving side's own route check, so none of
    /// them is refused when it runs over a Pairing.
    #[tokio::test]
    async fn every_shared_command_asks_what_a_peer_may_ask() {
        shaman_client::testing::walk_every_command(|name, asked| {
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
}
