//! What the core's tests stand on: a real Serving Server in process, a way to
//! it the test holds shut or opens, and a phone with nothing but the core.

#![allow(dead_code)]

pub mod relay;

use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use shaman::{
    managed_client::{ManagedClient, ManagedClientConfig},
    protocol::{
        AgentId, AgentIdentity, AgentSelection, ApprovalId, ApprovalPosture, CreateSessionRequest,
        Decision, ExecutionDirectory, InitialPrompt, IssueInviteRequest, ModelAvailability,
        ModelDescriptor, ModelDetails, ModelId, Peer, PromptId, ProviderId, QuestionnaireId,
        QuestionnaireSubmission, RedeemInviteRequest, SessionId, SettingMutation, SkillCatalog,
        SkillCatalogCapabilities, SkillCatalogStatus, Way,
    },
    provider::{
        AttributedProviderEvent, BrokerHandoff, ErrandFuture, ProviderDecisionDelivery,
        ProviderErrand, ProviderError, ProviderEvent, ProviderEventStream, ProviderFuture,
        ProviderInterruption, ProviderModelDiscovery, ProviderPostureApplication, ProviderRuntime,
        ProviderSession, ProviderSessionConnection, ProviderSessionRequest, ProviderSteerInput,
        ProviderSubagentId, ProviderTurnInput,
    },
    server::{self, ServerConfig, ServerTimings},
};
use shaman_client::SessionCommandTimings;
use shaman_pairing::{
    Keepalive, PairingProtocol, RedeemerTimings,
    signer::{Signer as _, testing::CallbackSigner},
};
use shaman_thin_client::{
    ActivityDetail, ActsListener, ConversationChange, ConversationItem, ConversationListener,
    CoreTimings, DetailListener, DeviceListener, DeviceLogin, DeviceLoginListener, FrameListener,
    KeySigner, Landing, LandingListener, Listing, ListingListener, Notice, NoticeListener,
    OpenSession, Outlook, OutlookListener, Servers, ServersListener, SessionFrame, SignError,
    SubagentTree, Subscription, ThinClient, ThisDevice, TreeListener, TurnWork, WorkListener,
    WorkspacePicker, WorkspacesListener,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
    time::timeout,
};

/// The marker `src/main.rs` gives Shaman's binary, so a Server a test starts
/// in process identifies this test binary without hashing the whole of it.
#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__shaman"))]
#[cfg_attr(not(target_os = "macos"), unsafe(link_section = ".shaman"))]
static BUILD_ID: [u8; 32] = shaman_build_id::generate!();

/// How long a test waits for anything it must see.
pub const PROGRESS_DEADLINE: Duration = Duration::from_secs(20);

/// The core's timings at a test's pace: a way that holds a connection
/// unanswered is given up within a fraction of a second, and confirming a
/// pairing keeps dialling for not much longer.
pub fn timings() -> CoreTimings {
    CoreTimings {
        relays: shaman_pairing::relay::RelayClientTimings {
            answer_timeout: Duration::from_secs(5),
        },
        redeemer: RedeemerTimings {
            handshake_timeout: Duration::from_millis(150),
            direct_head_start: Duration::from_millis(25),
            direct_retry_interval: Duration::from_millis(100),
            keepalive: Keepalive {
                interval: Duration::from_millis(250),
                timeout: Duration::from_secs(1),
            },
            withdrawal_timeout: Duration::from_millis(500),
            told_relays_store_interval: Duration::from_millis(25),
        },
        pairing_keeps_dialling: Duration::from_millis(600),
        pairing_redial: Duration::from_millis(25),
        readers: SessionCommandTimings {
            initial_recovery_backoff: Duration::from_millis(5),
            max_recovery_backoff: Duration::from_millis(50),
            attachment_fetch_timeout: PROGRESS_DEADLINE,
        },
        act_bound: PROGRESS_DEADLINE,
        ..shaman_thin_client::CoreTimings::default()
    }
}

/// A Server of Shaman's own in process, Serving on loopback, whose Sessions
/// never start: a Session begun in a directory named [`WORKING`] is left
/// Working, as its Provider never answers, and any other fails its first
/// Turn.
pub struct ServingServer {
    state: tempfile::TempDir,
    channel: String,
    config: ServerConfig,
    _config_root: tempfile::TempDir,
    server: server::RunningServer,
    provider: Arc<TestProvider>,
    client: ManagedClient,
    refusing_skills: Arc<std::sync::atomic::AtomicBool>,
    scripts: Arc<Mutex<Vec<Script>>>,
    handoffs: Handoffs,
    decisions: Decisions,
    submissions: Submissions,
}

impl ServingServer {
    pub async fn start(channel: &str) -> Self {
        Self::starting(channel, false, None).await
    }

    /// A Server whose Provider offers one Model, so that a Session its
    /// Sidekick begins through the Broker has an Agent to begin with, and a
    /// Session begun with no Agent Selection would get it, as its health
    /// says; and Skills, so a new Worktree can be prepared for it.
    pub async fn start_offering_a_model(channel: &str) -> Self {
        Self::starting(channel, true, None).await
    }

    /// [`Self::start`], each Invite it issues living for `ttl`.
    pub async fn start_with_invites_living(channel: &str, ttl: Duration) -> Self {
        Self::starting(channel, false, Some(ttl)).await
    }

    async fn starting(channel: &str, offers_a_model: bool, invite_ttl: Option<Duration>) -> Self {
        let state = tempfile::tempdir().expect("create the Server's state directory");
        let config_root = tempfile::tempdir().expect("create the Server's config root");
        let config = ServerConfig::new(state.path(), channel)
            .expect("configure the Server")
            .with_config_dir(config_root.path());
        let defaults = ServerTimings::default();
        let timings = ServerTimings {
            shutdown_grace: Duration::from_millis(5),
            invite_ttl: invite_ttl.unwrap_or(defaults.invite_ttl),
            ..defaults
        }
        .with_relay_retry_backoff(Duration::from_millis(5), Duration::from_millis(25));
        let provider = Arc::new(TestProvider {
            offers_a_model,
            ..TestProvider::default()
        });
        let refusing_skills = provider.refusing_skills.clone();
        let kept = provider.clone();
        let scripts = provider.scripts.clone();
        let handoffs = provider.handoffs.clone();
        let decisions = provider.decisions.clone();
        let submissions = provider.submissions.clone();
        let server = server::spawn_with_provider_and_timings(config.clone(), provider, timings)
            .await
            .expect("spawn the Server");
        let client = attach(&state, channel).await;
        for mutation in [
            SettingMutation::ServingPort { value: Some(0) },
            SettingMutation::ServingBindAddress {
                value: Some(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)),
            },
            SettingMutation::ServingEnabled { value: Some(true) },
        ] {
            client
                .mutate_setting(mutation)
                .await
                .expect("turn Serving on");
        }
        Self {
            state,
            channel: channel.to_owned(),
            config,
            _config_root: config_root,
            server,
            provider: kept,
            client,
            refusing_skills,
            scripts,
            handoffs,
            decisions,
            submissions,
        }
    }

    /// The directory a Session in this Server's Sidekick Workspace works in,
    /// made as it is first asked for. A Session begun there is a Sidekick's,
    /// and starts at once, as one begun in a directory named [`SCRIPTED`]
    /// does.
    pub async fn sidekick_directory(&self) -> std::path::PathBuf {
        self.client
            .sidekick_workspace()
            .await
            .expect("resolve the Sidekick Workspace")
            .execution_directory
            .expect("a Session can work in the Sidekick Workspace")
            .path
    }

    /// The Broker as the Agent of the scripted Session this Server started
    /// last reaches it, ready to call its Tools.
    pub async fn broker(&self) -> Broker {
        let handoff = timeout(PROGRESS_DEADLINE, async {
            loop {
                if let Some(handoff) = self.handoffs.lock().unwrap().last().cloned() {
                    return handoff;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("a scripted Session was handed the Broker");
        Broker::handed(&handoff).await
    }

    /// Pairs this Server with `other`, as a Peer of it that knows it as
    /// `name`.
    pub async fn pair_with(&self, other: &ServingServer, name: &str) {
        let invite = other
            .invite(vec![Way::Direct(other.serving_address())])
            .await;
        let remote = self
            .client
            .redeem_invite(RedeemInviteRequest {
                invite: invite.into(),
                name: Some(name.to_owned()),
                ways: Vec::new(),
                pair_back: false,
            })
            .await
            .expect("pair with the other Server");
        assert_eq!(remote.name, name);
    }

    /// Every Questionnaire submission a scripted Session's Provider has been
    /// delivered, in the order delivered.
    pub fn submissions(&self) -> Vec<(QuestionnaireId, QuestionnaireSubmission)> {
        self.submissions.delivered.lock().unwrap().clone()
    }

    /// Has a scripted Session's Provider refuse the next `count`
    /// Questionnaire submissions, as one that took none of them does: the
    /// Questionnaire stays live, to be answered again.
    pub fn refuse_submissions(&self, count: usize) {
        self.submissions.refusing.store(count, Ordering::Release);
    }

    /// Every Decision a scripted Session's Provider has been delivered, in
    /// the order delivered.
    pub fn decisions(&self) -> Vec<(ApprovalId, Decision)> {
        self.decisions.lock().unwrap().clone()
    }

    /// The Decisions delivered, once there are `count` of them.
    pub async fn decisions_until(&self, count: usize) -> Vec<(ApprovalId, Decision)> {
        timeout(PROGRESS_DEADLINE, async {
            loop {
                let decisions = self.decisions();
                if decisions.len() >= count {
                    return decisions;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the Decisions were delivered")
    }

    /// What the scripted Session this Server started last hears from its
    /// Provider, once it has started.
    pub async fn script(&self) -> Script {
        timeout(PROGRESS_DEADLINE, async {
            loop {
                if let Some(script) = self.scripts.lock().unwrap().last().cloned() {
                    return script;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("a scripted Session started")
    }

    pub fn serving_address(&self) -> SocketAddr {
        self.server
            .serving_address()
            .expect("the Server is Serving")
    }

    /// An Invite to this Server offering `ways`.
    pub async fn invite(&self, ways: Vec<Way>) -> String {
        self.client
            .issue_invite(IssueInviteRequest { ways })
            .await
            .expect("issue an Invite")
            .invite
            .0
    }

    /// Begins a Session in `workspace` with the Prompt `text`, as the
    /// Server's own user would.
    pub async fn begin(&self, workspace: &std::path::Path, text: &str) -> SessionId {
        self.begin_under(workspace, text, None).await
    }

    /// Begins the Session `session` in `workspace` with the Prompt `text`, as
    /// the Server's own user would under an identity chosen beforehand.
    pub async fn begin_as(&self, workspace: &std::path::Path, text: &str, session: SessionId) {
        self.begin_under(workspace, text, Some(session)).await;
    }

    async fn begin_under(
        &self,
        workspace: &std::path::Path,
        text: &str,
        session: Option<SessionId>,
    ) -> SessionId {
        self.client
            .create_session(CreateSessionRequest {
                session_id: session,
                preparation_id: None,
                agent_selection: None,
                execution_directory: ExecutionDirectory {
                    path: workspace.to_owned(),
                },
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: text.to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
            })
            .await
            .expect("begin a Session")
            .session
            .id
    }

    /// Has its Provider refuse its Skills while `refusing`, so a Worktree
    /// prepared meanwhile is made and kept, not ready, to be resumed.
    pub fn refuse_skills(&self, refusing: bool) {
        self.refusing_skills.store(refusing, Ordering::Release);
    }

    /// Names `roots` this Server's Workspace Roots, so the Repositories
    /// directly within each are among its Workspaces.
    pub async fn set_workspace_roots(&self, roots: &[&std::path::Path]) {
        self.client
            .mutate_setting(SettingMutation::WorkspaceRoots {
                value: Some(
                    roots
                        .iter()
                        .map(|root| root.to_string_lossy().into_owned())
                        .collect(),
                ),
            })
            .await
            .expect("name the Workspace Roots");
    }

    /// Interrupts the Session `session`, as the Server's own user would.
    pub async fn interrupt(&self, session: SessionId) {
        self.client
            .interrupt_session(session)
            .await
            .expect("interrupt the Session");
    }

    /// Deletes the Session `session`, as the Server's own user would.
    pub async fn delete(&self, session: SessionId) {
        self.client
            .delete_session(session)
            .await
            .expect("delete the Session");
    }

    /// Removes the Peer `fingerprint` names, ending its Pairing.
    pub async fn remove_peer(&self, fingerprint: &str) {
        let peer = self
            .peers()
            .await
            .into_iter()
            .find(|peer| peer.fingerprint == fingerprint)
            .expect("the Peer is paired");
        self.client
            .remove_peer(&peer.id)
            .await
            .expect("remove the Peer");
    }

    /// What the Server holds of the Session `session` now.
    pub async fn read(&self, session: SessionId) -> shaman::protocol::SessionSnapshot {
        self.client
            .read_session(session)
            .await
            .expect("read the Session")
    }

    /// The Peers this Server is paired with.
    pub async fn peers(&self) -> Vec<Peer> {
        self.client.list_peers().await.expect("list the Peers")
    }

    /// The offers to pair back this Server holds or made.
    pub async fn offers(&self) -> Vec<String> {
        let offers = self.client.list_offers().await.expect("list the offers");
        offers
            .held
            .iter()
            .map(|offer| format!("{offer:?}"))
            .chain(offers.made.iter().map(|offer| format!("{offer:?}")))
            .collect()
    }

    /// The fingerprint of this Server's identity key, as an Invite of its
    /// carries the key.
    pub fn fingerprint(&self) -> String {
        let key = std::fs::read(self.config.data_dir().join("server-identity.pk8"))
            .expect("read the Server's identity key");
        let key = rcgen_key(&key);
        shaman_pairing::fingerprint(&key)
    }

    /// Logs this Server in at `relay` as the user `username`, as its own
    /// user would by `/relay`.
    pub async fn log_in_at(&self, relay: &relay::TestRelay, username: &str) {
        let address = relay.address();
        self.client
            .add_relay(address.clone())
            .await
            .expect("add the Relay");
        let login = self
            .client
            .begin_relay_login(&address)
            .await
            .expect("begin a login at the Relay");
        assert!(relay.approve(&login.user_code, username));
        let done = self
            .client
            .follow_relay_login(&address)
            .await
            .expect("follow the login to its end");
        assert!(
            matches!(
                done.outcome,
                shaman::protocol::RelayLoginOutcome::Done { .. }
            ),
            "the Server logs in at the Relay: {done:?}"
        );
    }

    /// Has this Server Serve through `relay`, where it is logged in.
    pub async fn serve_through(&self, relay: &relay::TestRelay) {
        self.client
            .set_relay_serve_through(&relay.address(), true)
            .await
            .expect("Serve through the Relay");
    }

    /// Has this Server stop Serving through `relay`, staying logged in there.
    pub async fn stop_serving_through(&self, relay: &relay::TestRelay) {
        self.client
            .set_relay_serve_through(&relay.address(), false)
            .await
            .expect("stop Serving through the Relay");
    }

    /// The machines `relay` lists under this Server's Account there: each by
    /// its fingerprint, with the hostname it reported.
    pub async fn machines_at(&self, relay: &relay::TestRelay) -> Vec<(String, String)> {
        let listed = self
            .client
            .list_relay_machines(&relay.address())
            .await
            .expect("ask the Relay for the machines logged in there");
        match listed.answer {
            shaman::protocol::RelayMachinesAnswer::Listed { machines, .. } => machines
                .into_iter()
                .map(|machine| (machine.fingerprint, machine.hostname))
                .collect(),
            answer => panic!("the Relay lists the Account's machines: {answer:?}"),
        }
    }

    /// The hostname this Server gives itself in its Invites.
    pub fn hostname(&self) -> String {
        shaman_pairing::machine_hostname()
    }

    pub async fn shutdown(self) {
        self.server.shutdown().await.expect("stop the Server");
    }

    /// Has this Server send, on the Session `session`'s stream, a change of
    /// a kind no build knows, as a Server newer than the phone sends one.
    pub async fn send_unknown_change(&self, session: SessionId) {
        self.server
            .session_event_sink()
            .publish(session, vec![shaman::protocol::SessionChange::Unknown])
            .await
            .expect("publish a change of a kind no build knows");
    }

    /// This Server stopped and started again on the address it Serves on,
    /// keeping its records and so every Pairing, speaking `protocol` to its
    /// Peers: as it would after an update to another release.
    pub async fn restarted_speaking(self, protocol: server::PairingProtocol) -> Self {
        let address = self.serving_address();
        self.client
            .mutate_setting(SettingMutation::ServingPort {
                value: Some(address.port()),
            })
            .await
            .expect("keep the Serving port across the restart");
        let Self {
            state,
            channel,
            config,
            _config_root,
            server,
            provider,
            client,
            refusing_skills,
            scripts,
            handoffs,
            decisions,
            submissions,
        } = self;
        drop(client);
        server.shutdown().await.expect("stop the Server");
        let timings = ServerTimings {
            shutdown_grace: Duration::from_millis(5),
            pairing_protocol: protocol,
            ..ServerTimings::default()
        };
        let server =
            server::spawn_with_provider_and_timings(config.clone(), provider.clone(), timings)
                .await
                .expect("start the Server again");
        let client = attach(&state, &channel).await;
        let restarted = Self {
            state,
            channel,
            config,
            _config_root,
            server,
            provider,
            client,
            refusing_skills,
            scripts,
            handoffs,
            decisions,
            submissions,
        };
        assert_eq!(
            restarted.serving_address(),
            address,
            "Serving again where the phone dials it"
        );
        restarted
    }
}

/// A Client of the Server whose state is in `state`, on `channel`.
async fn attach(state: &tempfile::TempDir, channel: &str) -> ManagedClient {
    ManagedClient::connect(
        ManagedClientConfig::new(state.path(), channel).expect("configure the Client"),
    )
    .await
    .expect("attach the Client")
}

/// The public key of the PKCS #8 key `pkcs8`, as a Pairing pins it.
fn rcgen_key(pkcs8: &[u8]) -> Vec<u8> {
    shaman_pairing::signer::SoftwareSigner::from_pkcs8(pkcs8)
        .expect("take up the Server's identity key")
        .public_key()
}

/// A way to a Server the test holds shut, so that what dials it connects and
/// hears nothing, or opens, so that it carries each connection there; and
/// which the test may cut, as a network swapped or lost while the phone
/// slept: what it carried then goes silent though it stays open, and each
/// connection dialled after is refused or held as the cut says, until it is
/// mended.
#[derive(Clone)]
pub struct Gate {
    address: SocketAddr,
    open: watch::Sender<bool>,
    /// How the way stands, and how many times it has been cut.
    cut: watch::Sender<(Cut, u64)>,
    dialled: Arc<AtomicU64>,
    attempted: Arc<AtomicU64>,
    /// The connections it carries, which shutting it cuts.
    carrying: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// Whether what the Server says back is dropped.
    muted: watch::Sender<bool>,
    /// Where each connection it carries from now on is carried.
    target: Arc<Mutex<SocketAddr>>,
}

/// How a way stands for what is dialled to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// Carried to the Server.
    Mended,
    /// Closed at once, so the dial fails at once.
    Refusing,
    /// Held open and carried nowhere, so the dial hears nothing.
    Silent,
}

impl Gate {
    /// A way to `target`, carrying each connection there at once.
    pub async fn open(target: SocketAddr) -> Self {
        let gate = Self::held(target).await;
        gate.open_now();
        gate
    }

    /// A way to `target` carrying its first connection there, and holding
    /// each after it, which connects and hears nothing: a Pairing's
    /// preparation is carried and its commit is not.
    pub async fn once(target: SocketAddr) -> Self {
        let gate = Self::carrying(target, 1).await;
        gate.open_now();
        gate
    }

    /// A way to `target` held shut until it is opened.
    pub async fn held(target: SocketAddr) -> Self {
        Self::carrying(target, u64::MAX).await
    }

    /// A way to `target` held shut until it is opened, then carrying as many
    /// as `carried` connections there and holding each after them, and
    /// holding each new one while it is shut again.
    async fn carrying(target: SocketAddr, carried: u64) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listen for the way");
        let address = listener.local_addr().expect("the way's address");
        let open = watch::Sender::new(false);
        let muted = watch::Sender::new(false);
        let muting = muted.clone();
        let cut = watch::Sender::new((Cut::Mended, 0));
        let dialled = Arc::new(AtomicU64::new(0));
        let carrying = Arc::new(Mutex::new(Vec::new()));
        let mut opened = open.subscribe();
        let counted = dialled.clone();
        let proxies = carrying.clone();
        let attempted = Arc::new(AtomicU64::new(0));
        let attempts = attempted.clone();
        let cuts = cut.clone();
        let target = Arc::new(Mutex::new(target));
        let pointed = target.clone();
        tokio::spawn(async move {
            // Until it opens, nothing is accepted: a dial connects, and its
            // handshake hears nothing.
            if opened.wait_for(|open| *open).await.is_err() {
                return;
            }
            let holding: Arc<Mutex<Vec<Box<dyn std::any::Any + Send>>>> = Arc::default();
            while let Ok((inbound, _)) = listener.accept().await {
                attempts.fetch_add(1, Ordering::AcqRel);
                let (standing, cuts_so_far) = *cuts.borrow();
                match standing {
                    Cut::Refusing => continue,
                    Cut::Silent => {
                        holding.lock().unwrap().push(Box::new(inbound));
                        continue;
                    }
                    Cut::Mended => {}
                }
                if !*opened.borrow() || counted.fetch_add(1, Ordering::AcqRel) >= carried {
                    holding.lock().unwrap().push(Box::new(inbound));
                    continue;
                }
                let (muted, mut cut, frozen) =
                    (muting.subscribe(), cuts.subscribe(), holding.clone());
                let target = *pointed.lock().unwrap();
                let proxy = tokio::spawn(async move {
                    let Ok(outbound) = tokio::net::TcpStream::connect(target).await else {
                        return;
                    };
                    let (mut from_phone, mut to_phone) = inbound.into_split();
                    let (mut from_server, mut to_server) = outbound.into_split();
                    tokio::select! {
                        () = carry(
                            &mut from_phone,
                            &mut to_phone,
                            &mut from_server,
                            &mut to_server,
                            muted,
                        ) => return,
                        _ = cut.wait_for(|(_, cuts)| *cuts > cuts_so_far) => {}
                    }
                    // Cut: it stays open and carries nothing, ever after.
                    frozen.lock().unwrap().extend([
                        Box::new(from_phone) as Box<dyn std::any::Any + Send>,
                        Box::new(to_phone),
                        Box::new(from_server),
                        Box::new(to_server),
                    ]);
                });
                proxies.lock().unwrap().push(proxy);
            }
        });
        Self {
            address,
            open,
            muted,
            cut,
            dialled,
            attempted,
            carrying,
            target,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn open_now(&self) {
        self.open.send_replace(true);
    }

    /// Mutes the way: what the phone says still reaches the Server, and
    /// nothing the Server says back reaches the phone, on every connection.
    pub fn mute_now(&self) {
        self.muted.send_replace(true);
    }

    /// Carries what the Server says back again; what was dropped while the
    /// way was muted stays lost, so what it carried then breaks.
    pub fn unmute_now(&self) {
        self.muted.send_replace(false);
    }

    /// Shuts the way again: every connection it carries is cut, and each
    /// made from now on connects and hears nothing until it opens.
    pub fn shut_now(&self) {
        self.open.send_replace(false);
        for proxy in self.carrying.lock().unwrap().drain(..) {
            proxy.abort();
        }
    }

    /// Carries each connection dialled from now on to `target`, as an
    /// address given to another machine while the one it named slept: what
    /// it carries already is left as it is.
    pub fn point_at(&self, target: SocketAddr) {
        *self.target.lock().unwrap() = target;
    }

    /// Cuts the way: what it carries goes silent, and what is dialled after
    /// is treated as `cut` says, until it is mended.
    pub fn cut(&self, cut: Cut) {
        self.cut.send_modify(|(standing, cuts)| {
            *standing = cut;
            *cuts += 1;
        });
    }

    /// Carries what is dialled after to the Server again; what went silent
    /// as it was cut stays so.
    pub fn mend(&self) {
        self.cut
            .send_modify(|(standing, _)| *standing = Cut::Mended);
    }

    /// How many connections the way has carried.
    pub fn dialled(&self) -> u64 {
        self.dialled.load(Ordering::Acquire)
    }

    /// How many connections have been dialled to the way since it opened,
    /// carried or not.
    pub fn attempted(&self) -> u64 {
        self.attempted.load(Ordering::Acquire)
    }
}

/// Carries what the phone says to the Server and what it says back, dropping
/// what comes back while `muted` says to.
async fn carry(
    from_phone: &mut tokio::net::tcp::OwnedReadHalf,
    to_phone: &mut tokio::net::tcp::OwnedWriteHalf,
    from_server: &mut tokio::net::tcp::OwnedReadHalf,
    to_server: &mut tokio::net::tcp::OwnedWriteHalf,
    muted: watch::Receiver<bool>,
) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let said = async {
        let _ = tokio::io::copy(from_phone, to_server).await;
    };
    let answered = async {
        let mut buffer = vec![0; 16 * 1024];
        loop {
            let read = match from_server.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => read,
            };
            if *muted.borrow() {
                continue;
            }
            if to_phone.write_all(&buffer[..read]).await.is_err() {
                return;
            }
        }
    };
    tokio::join!(said, answered);
}

/// A key held apart from the core, as the app holds a phone's.
pub struct TestKey(CallbackSigner);

impl TestKey {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(CallbackSigner::elsewhere()))
    }
}

impl KeySigner for TestKey {
    fn public_key(&self) -> Vec<u8> {
        self.0.public_key()
    }

    fn sign(&self, message: Vec<u8>) -> Result<Vec<u8>, SignError> {
        self.0.sign(&message).map_err(|refused| SignError::Refused {
            message: format!("{refused:#}"),
        })
    }
}

/// A phone: nothing but the core, kept in a directory of its own.
pub struct Phone {
    directory: tempfile::TempDir,
    key: Arc<TestKey>,
    timings: CoreTimings,
    protocol: PairingProtocol,
    pub core: Arc<ThinClient>,
}

impl Phone {
    pub fn new() -> Self {
        Self::with(timings())
    }

    /// A phone whose core waits as `timings` say.
    pub fn with(timings: CoreTimings) -> Self {
        Self::speaking(timings, PairingProtocol::CURRENT)
    }

    /// A phone whose core waits as `timings` say, speaking `protocol` to
    /// every Server: an app of another release.
    pub fn speaking(timings: CoreTimings, protocol: PairingProtocol) -> Self {
        let directory = tempfile::tempdir().expect("create the phone's directory");
        let key = TestKey::new();
        let core = open(&directory, &key, timings, protocol);
        Self {
            directory,
            key,
            timings,
            protocol,
            core,
        }
    }

    /// The directory the core keeps everything in.
    pub fn directory(&self) -> &std::path::Path {
        self.directory.path()
    }

    /// The phone after the app was ended and launched again.
    pub fn relaunched(self) -> Self {
        self.relaunched_after(|_| {})
    }

    /// The phone after the app was ended, its directory changed by `change`
    /// while nothing of the core runs to write over it, and launched again.
    pub fn relaunched_after(self, change: impl FnOnce(&std::path::Path)) -> Self {
        let Self {
            directory,
            key,
            timings,
            protocol,
            core,
        } = self;
        drop(core);
        change(directory.path());
        let core = open(&directory, &key, timings, protocol);
        Self {
            directory,
            key,
            timings,
            protocol,
            core,
        }
    }

    /// What the core says of its own accord once it says what `shown` looks
    /// for.
    pub async fn notice_until(&self, shown: impl Fn(&Option<Notice>) -> bool) -> Option<Notice> {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_notices(Arc::new(Heard(sender)));
        let heard = timeout(
            PROGRESS_DEADLINE,
            heard.wait_for(|notice| notice.as_ref().is_some_and(&shown)),
        )
        .await;
        match heard {
            Ok(Ok(notice)) => notice.clone().expect("heard"),
            _ => panic!("the core never said what was looked for"),
        }
    }

    /// How many times this phone's key has signed.
    pub fn signed(&self) -> u64 {
        self.key.0.signed().load(Ordering::Acquire)
    }

    /// The fingerprint a Serving Server knows this phone by.
    pub fn fingerprint(&self) -> String {
        shaman_pairing::fingerprint(&self.key.public_key())
    }

    pub async fn outlook(&self) -> Outlook {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_outlook(Arc::new(Heard(sender)));
        heard_at_once(&mut heard).await
    }

    /// Where the Outlook stands once it stands where `shown` looks for.
    pub async fn outlook_until(&self, shown: impl Fn(&Outlook) -> bool) -> Outlook {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_outlook(Arc::new(Heard(sender)));
        let heard = timeout(
            PROGRESS_DEADLINE,
            heard.wait_for(|outlook| outlook.as_ref().is_some_and(&shown)),
        )
        .await;
        match heard {
            Ok(Ok(outlook)) => outlook.clone().expect("heard"),
            _ => panic!("the Outlook never stood where it was looked for"),
        }
    }

    pub async fn device(&self) -> ThisDevice {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_device(Arc::new(Heard(sender)));
        heard_at_once(&mut heard).await
    }

    pub async fn servers(&self) -> Servers {
        self.servers_until(|_| true).await
    }

    /// What the Servers section shows once it shows what `shown` looks for.
    pub async fn servers_until(&self, shown: impl Fn(&Servers) -> bool) -> Servers {
        let (sender, mut heard) = watch::channel(None);
        let _subscription: Arc<Subscription> = self.core.watch_servers(Arc::new(Heard(sender)));
        let heard = timeout(
            PROGRESS_DEADLINE,
            heard.wait_for(|servers| servers.as_ref().is_some_and(&shown)),
        )
        .await;
        match heard {
            Ok(Ok(servers)) => servers.clone().expect("heard"),
            _ => panic!(
                "the Servers section never showed what was looked for; it shows {:?}",
                heard_now(&self.core)
            ),
        }
    }

    /// Every Servers section the core shows from now on, for as long as what
    /// this answers is held.
    pub fn hear_servers(&self) -> ServersHeard {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let subscription = self
            .core
            .watch_servers(Arc::new(EveryServers(heard.clone())));
        ServersHeard {
            _subscription: subscription,
            heard,
        }
    }

    /// Logs in at the Relay at `relay` through the core, followed until the
    /// login is dropped.
    pub fn log_in(&self, relay: &str) -> Login {
        let (sender, heard) = watch::channel(None);
        let subscription = self
            .core
            .log_in_at_relay(relay.to_owned(), Arc::new(Heard(sender)));
        Login {
            subscription,
            heard,
        }
    }

    /// Logs in at `relay` as the user `username`, approving the code the
    /// core shows, and waits for the Relay to confirm it.
    pub async fn logged_in(&self, relay: &relay::TestRelay, username: &str) -> DeviceLogin {
        let login = self.log_in(&relay.address());
        let code = login.code().await;
        assert!(relay.approve(&code, username), "a login waits under {code}");
        login
            .until(|login| matches!(login, DeviceLogin::Done { .. }))
            .await
    }

    /// What the listing shows once it shows what `shown` looks for.
    pub async fn listing_until(&self, shown: impl Fn(&Listing) -> bool) -> Listing {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_listing(Arc::new(Heard(sender)));
        let heard = timeout(
            PROGRESS_DEADLINE,
            heard.wait_for(|listing| listing.as_ref().is_some_and(&shown)),
        )
        .await;
        match heard {
            Ok(Ok(listing)) => listing.clone().expect("heard"),
            _ => panic!("the listing never showed what was looked for"),
        }
    }

    /// Whether an act is under way, once it is as `wanted` says.
    pub async fn acts_until(&self, wanted: bool) {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_acts(Arc::new(Heard(sender)));
        timeout(
            PROGRESS_DEADLINE,
            heard.wait_for(|under_way| *under_way == Some(wanted)),
        )
        .await
        .expect("acts stood as wanted in time")
        .expect("heard");
    }

    /// What the Landing shows once it shows what `shown` looks for.
    pub async fn landing_until(&self, shown: impl Fn(&Landing) -> bool) -> Landing {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_landing(Arc::new(Heard(sender)));
        let heard = timeout(
            PROGRESS_DEADLINE,
            heard.wait_for(|landing| landing.as_ref().is_some_and(&shown)),
        )
        .await;
        match heard {
            Ok(Ok(landing)) => landing.clone().expect("heard"),
            _ => panic!("the Landing never showed what was looked for"),
        }
    }

    /// What the Workspace Picker shows once it shows what `shown` looks for,
    /// searched for `query`.
    pub async fn workspaces_until(
        &self,
        query: &str,
        shown: impl Fn(&WorkspacePicker) -> bool,
    ) -> WorkspacePicker {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_workspaces(Arc::new(Heard(sender)));
        self.core.search_workspaces(query.to_owned());
        let found = timeout(
            PROGRESS_DEADLINE,
            heard.wait_for(|picker| picker.as_ref().is_some_and(&shown)),
        )
        .await
        .map(|found| found.map(|picker| picker.clone()));
        match found {
            Ok(Ok(picker)) => picker.expect("heard"),
            _ => panic!(
                "the Workspace Picker never showed what was looked for: {:#?}",
                *heard.borrow()
            ),
        }
    }

    /// What the Workspace Picker, opened afresh, shows within `within`, where
    /// it shows what `shown` looks for by then.
    pub async fn workspaces_within(
        &self,
        within: Duration,
        shown: impl Fn(&WorkspacePicker) -> bool,
    ) -> Option<WorkspacePicker> {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_workspaces(Arc::new(Heard(sender)));
        timeout(
            within,
            heard.wait_for(|picker| picker.as_ref().is_some_and(&shown)),
        )
        .await
        .ok()
        .and_then(|found| found.ok().and_then(|picker| picker.clone()))
    }

    /// Begins a Session with the Prompt `text` where the Outlook stands, as
    /// the Landing's send does, and reads it as its screen turns into it.
    pub fn begin(&self, text: &str) -> Reading {
        let session = self
            .core
            .begin_session(text.to_owned(), false)
            .expect("a Session begun");
        Self::reading(session)
    }

    /// Begins a Session with the Prompt `text` where the Outlook stands, in
    /// a new Worktree, as the Landing's send does with that chosen.
    pub fn begin_in_new_worktree(&self, text: &str) -> Reading {
        let session = self
            .core
            .begin_session(text.to_owned(), true)
            .expect("a Session begun");
        Self::reading(session)
    }

    /// Opens the Session `reference` names, as a screen pushed to read it.
    pub fn open(&self, reference: &str) -> Reading {
        Self::reading(self.core.open_session(reference.to_owned()))
    }

    fn reading(session: Arc<OpenSession>) -> Reading {
        let (frame_heard, frame) = watch::channel(None);
        let (conversation_heard, conversation) = watch::channel(Conversation::default());
        let frame_subscription = session.watch_frame(Arc::new(Heard(frame_heard)));
        let conversation_subscription =
            session.watch_conversation(Arc::new(Held(conversation_heard)));
        Reading {
            session,
            frame,
            conversation,
            _subscriptions: [frame_subscription, conversation_subscription],
        }
    }

    pub fn shutdown(self) {
        drop(self.core);
        drop(self.directory);
    }
}

/// An open Session, and what its listeners have heard of it.
/// A login at a Relay, followed through the core until it is dropped, which
/// gives it up where it is still under way.
pub struct Login {
    pub subscription: Arc<Subscription>,
    heard: watch::Receiver<Option<DeviceLogin>>,
}

impl Login {
    /// How the login stands once it stands as `shown` looks for.
    pub async fn until(&self, shown: impl Fn(&DeviceLogin) -> bool) -> DeviceLogin {
        let mut heard = self.heard.clone();
        let found = timeout(
            PROGRESS_DEADLINE,
            heard.wait_for(|login| login.as_ref().is_some_and(&shown)),
        )
        .await;
        match found {
            Ok(Ok(login)) => login.clone().expect("heard"),
            _ => panic!(
                "the login never stood as looked for; it stands {:?}",
                self.heard.borrow()
            ),
        }
    }

    /// The code the core shows, once it shows one.
    pub async fn code(&self) -> String {
        match self
            .until(|login| {
                matches!(
                    login,
                    DeviceLogin::Waiting { .. } | DeviceLogin::Failed { .. }
                )
            })
            .await
        {
            DeviceLogin::Waiting { code, .. } => code,
            failed => panic!("the login shows a code: {failed:?}"),
        }
    }
}

impl DeviceLoginListener for Heard<DeviceLogin> {
    fn changed(&self, login: DeviceLogin) {
        self.0.send_replace(Some(login));
    }
}

pub struct Reading {
    pub session: Arc<OpenSession>,
    frame: watch::Receiver<Option<SessionFrame>>,
    conversation: watch::Receiver<Conversation>,
    _subscriptions: [Arc<Subscription>; 2],
}

/// A conversation as a listener holds it: each item by its id, the order
/// they stand in, and every change it was told, in the order told.
#[derive(Clone, Debug, Default)]
pub struct Conversation {
    pub items: Vec<ConversationItem>,
    pub told: Vec<ConversationChange>,
}

impl Reading {
    /// The frame once it shows what `shown` looks for.
    pub async fn frame_until(&self, shown: impl Fn(&SessionFrame) -> bool) -> SessionFrame {
        let mut frame = self.frame.clone();
        let heard = timeout(
            PROGRESS_DEADLINE,
            frame.wait_for(|frame| frame.as_ref().is_some_and(&shown)),
        )
        .await;
        match heard {
            Ok(Ok(frame)) => frame.clone().expect("heard"),
            _ => panic!(
                "the frame never showed what was looked for; it shows {:?}",
                self.frame.borrow()
            ),
        }
    }

    /// The frame as it stands at once.
    pub async fn frame(&self) -> SessionFrame {
        self.frame_until(|_| true).await
    }

    /// The conversation once it holds what `shown` looks for.
    pub async fn conversation_until(
        &self,
        shown: impl Fn(&[ConversationItem]) -> bool,
    ) -> Conversation {
        let mut conversation = self.conversation.clone();
        let heard = timeout(
            PROGRESS_DEADLINE,
            conversation.wait_for(|conversation| {
                !conversation.told.is_empty() && shown(&conversation.items)
            }),
        )
        .await;
        match heard {
            Ok(Ok(conversation)) => conversation.clone(),
            _ => panic!(
                "the conversation never showed what was looked for; it holds {:#?}",
                self.conversation.borrow().items
            ),
        }
    }

    /// The conversation as it was last heard.
    pub fn conversation_now(&self) -> Conversation {
        self.conversation.borrow().clone()
    }

    /// Attaches the work of the Turn `turn` names, as its screen does while
    /// it is up.
    pub fn work(&self, turn: &str) -> Attached<TurnWork> {
        let (heard, told) = watch::channel(Vec::new());
        let subscription = self
            .session
            .watch_work(turn.to_owned(), Arc::new(Told(heard)));
        Attached { told, subscription }
    }

    /// Attaches the whole of the Activity `activity` names, as its screen
    /// does while it is up.
    pub fn detail(&self, activity: &str) -> Attached<ActivityDetail> {
        let (heard, told) = watch::channel(Vec::new());
        let subscription = self
            .session
            .watch_detail(activity.to_owned(), Arc::new(Told(heard)));
        Attached { told, subscription }
    }
}

impl Reading {
    /// Attaches the tree the Session belongs to, as its sheet does while it
    /// is up.
    pub fn tree(&self) -> Attached<SubagentTree> {
        let (heard, told) = watch::channel(Vec::new());
        let subscription = self.session.watch_tree(Arc::new(Told(heard)));
        Attached { told, subscription }
    }
}

/// What a screen pushed from an open Session attached, and everything it was
/// told of it, in the order told.
pub struct Attached<T> {
    told: watch::Receiver<Vec<T>>,
    pub subscription: Arc<Subscription>,
}

impl<T: Clone + std::fmt::Debug> Attached<T> {
    /// What it was told last, once that shows what `shown` looks for.
    pub async fn until(&self, shown: impl Fn(&T) -> bool) -> T {
        let mut told = self.told.clone();
        let heard = timeout(
            PROGRESS_DEADLINE,
            told.wait_for(|told| told.last().is_some_and(&shown)),
        )
        .await;
        match heard {
            Ok(Ok(told)) => told.last().cloned().expect("told"),
            _ => panic!(
                "never told what was looked for; told {:#?}",
                self.told.borrow().last()
            ),
        }
    }

    /// How many times it has been told.
    pub fn told(&self) -> usize {
        self.told.borrow().len()
    }
}

/// A listener keeping everything it was told.
struct Told<T>(watch::Sender<Vec<T>>);

impl WorkListener for Told<TurnWork> {
    fn changed(&self, work: TurnWork) {
        self.0.send_modify(|told| told.push(work));
    }
}

impl TreeListener for Told<SubagentTree> {
    fn changed(&self, tree: SubagentTree) {
        self.0.send_modify(|told| told.push(tree));
    }
}

impl DetailListener for Told<ActivityDetail> {
    fn changed(&self, detail: ActivityDetail) {
        self.0.send_modify(|told| told.push(detail));
    }
}

/// A conversation's listener, holding what it is told as Swift does:
/// replacing each item by its id and standing them in the order told.
struct Held(watch::Sender<Conversation>);

impl ConversationListener for Held {
    fn changed(&self, change: ConversationChange) {
        self.0.send_modify(|held| {
            held.items = match &change {
                ConversationChange::Whole { items } => items.clone(),
                ConversationChange::Changed { items, order } => {
                    let mut by_id = held
                        .items
                        .drain(..)
                        .chain(items.iter().cloned())
                        .map(|item| (item.id.clone(), item))
                        .collect::<std::collections::HashMap<_, _>>();
                    order
                        .iter()
                        .map(|id| by_id.remove(id).expect("every item ordered was told"))
                        .collect()
                }
            };
            held.told.push(change);
        });
    }
}

fn open(
    directory: &tempfile::TempDir,
    key: &Arc<TestKey>,
    timings: CoreTimings,
    protocol: PairingProtocol,
) -> Arc<ThinClient> {
    ThinClient::open_speaking(
        directory.path().to_string_lossy().into_owned(),
        key.clone(),
        timings,
        protocol,
    )
    .expect("open the core")
}

/// What the Servers section shows now, for a failure to say.
fn heard_now(core: &ThinClient) -> Option<Servers> {
    let (sender, heard) = watch::channel(None);
    let _subscription = core.watch_servers(Arc::new(Heard(sender)));
    std::thread::sleep(Duration::from_millis(50));
    heard.borrow().clone()
}

/// The first thing a listener hears, which it hears at once.
async fn heard_at_once<T: Clone>(heard: &mut watch::Receiver<Option<T>>) -> T {
    timeout(PROGRESS_DEADLINE, heard.wait_for(Option::is_some))
        .await
        .expect("heard at once")
        .expect("heard")
        .clone()
        .expect("heard")
}

/// A listener keeping the latest it heard.
struct Heard<T>(watch::Sender<Option<T>>);

/// Every Servers section a phone's core has shown since it was asked.
pub struct ServersHeard {
    _subscription: Arc<Subscription>,
    heard: Arc<Mutex<Vec<Servers>>>,
}

impl ServersHeard {
    /// What has been shown so far, in order.
    pub fn heard(&self) -> Vec<Servers> {
        self.heard.lock().expect("what was heard").clone()
    }
}

struct EveryServers(Arc<Mutex<Vec<Servers>>>);

impl ServersListener for EveryServers {
    fn changed(&self, servers: Servers) {
        self.0.lock().expect("what was heard").push(servers);
    }
}

impl DeviceListener for Heard<ThisDevice> {
    fn changed(&self, device: ThisDevice) {
        self.0.send_replace(Some(device));
    }
}

impl ServersListener for Heard<Servers> {
    fn changed(&self, servers: Servers) {
        self.0.send_replace(Some(servers));
    }
}

impl ListingListener for Heard<Listing> {
    fn changed(&self, listing: Listing) {
        self.0.send_replace(Some(listing));
    }
}

impl FrameListener for Heard<SessionFrame> {
    fn changed(&self, frame: SessionFrame) {
        self.0.send_replace(Some(frame));
    }
}

impl NoticeListener for Heard<Option<Notice>> {
    fn changed(&self, notice: Option<Notice>) {
        self.0.send_replace(Some(notice));
    }
}

impl ActsListener for Heard<bool> {
    fn changed(&self, under_way: bool) {
        self.0.send_replace(Some(under_way));
    }
}

impl LandingListener for Heard<Landing> {
    fn changed(&self, landing: Landing) {
        self.0.send_replace(Some(landing));
    }
}

impl WorkspacesListener for Heard<WorkspacePicker> {
    fn changed(&self, picker: WorkspacePicker) {
        self.0.send_replace(Some(picker));
    }
}

impl OutlookListener for Heard<Outlook> {
    fn changed(&self, outlook: Outlook) {
        self.0.send_replace(Some(outlook));
    }
}

/// The name of a directory a Session begun in is left Working.
pub const WORKING: &str = "working";

/// The name of a directory a Session begun in starts at once, its Turns
/// doing whatever the test scripts and ending as it says.
pub const SCRIPTED: &str = "scripted";

/// What a scripted Session's Provider sends, where the test scripts it.
pub type Script = mpsc::UnboundedSender<Result<AttributedProviderEvent, ProviderError>>;

/// The Broker handoff each scripted Session's start carried, the latest
/// started last.
type Handoffs = Arc<Mutex<Vec<BrokerHandoff>>>;

/// The Decisions every scripted Session's Provider was delivered.
type Decisions = Arc<Mutex<Vec<(ApprovalId, Decision)>>>;

/// The Questionnaire submissions every scripted Session's Provider was
/// delivered, and how many more it refuses.
type Submissions = Arc<Submitted>;

#[derive(Default)]
struct Submitted {
    delivered: Mutex<Vec<(QuestionnaireId, QuestionnaireSubmission)>>,
    refusing: std::sync::atomic::AtomicUsize,
}

/// A Provider whose Sessions, but a scripted one, never start: one begun in
/// a directory named [`WORKING`] waits on it for ever, and any other fails.
#[derive(Default)]
struct TestProvider {
    /// Whether it refuses its Skills for now, so a Worktree prepared for it
    /// is made and not ready, and kept to be resumed.
    refusing_skills: Arc<std::sync::atomic::AtomicBool>,
    /// What each scripted Session's Provider sends, the latest started last.
    scripts: Arc<Mutex<Vec<Script>>>,
    handoffs: Handoffs,
    decisions: Decisions,
    submissions: Submissions,
    /// Whether it offers its one Model, rather than discovering none, and
    /// Skills, so its Server names an Agent a new Session would get and can
    /// prepare a Worktree for it.
    offers_a_model: bool,
}

impl ProviderRuntime for TestProvider {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new("none")
    }

    fn display_name(&self) -> &str {
        "none"
    }

    fn list_models(&self) -> ProviderFuture<'_, ProviderModelDiscovery> {
        let offers = self.offers_a_model;
        Box::pin(async move {
            if !offers {
                return Err(ProviderError::new("Model discovery is unavailable."));
            }
            Ok(ProviderModelDiscovery::new(vec![ModelDescriptor {
                provider: ProviderId::new("none"),
                id: ModelId::new("scripted"),
                display_name: "Scripted".to_owned(),
                description: String::new(),
                is_default: true,
                availability: ModelAvailability::Available,
                options: Vec::new(),
                details: ModelDetails::default(),
            }]))
        })
    }

    fn skill_catalog(&self, workspace: &std::path::Path) -> ProviderFuture<'_, SkillCatalog> {
        let (offers, workspace) = (self.offers_a_model, workspace.to_owned());
        let refusing = self.refusing_skills.load(Ordering::Acquire);
        Box::pin(async move {
            if !offers || refusing {
                return Err(ProviderError::new("No Skills are offered."));
            }
            Ok(SkillCatalog {
                provider: ProviderId::new("none"),
                execution_directory: ExecutionDirectory { path: workspace },
                skills: Vec::new(),
                capabilities: SkillCatalogCapabilities {
                    max_distinct_invocations: None,
                    supported_deliveries: Vec::new(),
                },
                status: SkillCatalogStatus::Fresh { warning: None },
            })
        })
    }

    fn start_session(
        &self,
        request: ProviderSessionRequest,
    ) -> ProviderFuture<'_, ProviderSessionConnection> {
        let named = |name: &str| {
            request
                .execution_directory
                .file_name()
                .is_some_and(|named| named == name)
        };
        // A Sidekick's Session works in the Sidekick Workspace, and is
        // scripted as any other.
        let (working, scripted) = (named(WORKING), named(SCRIPTED) || named("sidekick"));
        let scripts = self.scripts.clone();
        if scripted && let Some(handoff) = &request.broker {
            self.handoffs.lock().unwrap().push(handoff.clone());
        }
        let decisions = self.decisions.clone();
        let submissions = self.submissions.clone();
        Box::pin(async move {
            if scripted {
                let (script, events) = mpsc::unbounded_channel();
                scripts.lock().unwrap().push(script.clone());
                let events: ProviderEventStream =
                    Box::pin(futures_util::stream::unfold(events, |mut events| async {
                        events.recv().await.map(|event| (event, events))
                    }));
                return Ok(ProviderSessionConnection::new(
                    AgentIdentity {
                        agent: AgentId::new("scripted"),
                        selection: AgentSelection {
                            provider: ProviderId::new("none"),
                            model: ModelId::new("scripted"),
                            options: Vec::new(),
                        },
                    },
                    None,
                    Arc::new(ScriptedSession {
                        script,
                        decisions,
                        submissions,
                    }),
                    events,
                ));
            }
            if working {
                std::future::pending::<()>().await;
            }
            Err(ProviderError::new("No Session starts on this test Server."))
        })
    }

    fn run_errand(&self, _errand: ProviderErrand) -> ErrandFuture<'_> {
        Box::pin(async { Err(ProviderError::new("No errand runs on this test Server.").into()) })
    }

    fn errand_selection(&self) -> Option<AgentSelection> {
        None
    }

    // A scripted Session's Subagents may be stopped one at a time, as
    // Claude's and Codex's may.
    fn supports_subagent_stop(&self) -> bool {
        true
    }

    fn shutdown(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// A scripted Session's Provider side: it takes every Turn, steer, Decision
/// and Questionnaire submission, and stops a Turn by saying so.
struct ScriptedSession {
    script: Script,
    decisions: Decisions,
    submissions: Submissions,
}

impl ProviderSession for ScriptedSession {
    fn update_approval_posture(
        &self,
        _posture: ApprovalPosture,
        _has_active_work: bool,
    ) -> ProviderFuture<'_, ProviderPostureApplication> {
        Box::pin(async { Ok(ProviderPostureApplication::NextTurn) })
    }

    fn submit_decision(
        &self,
        id: ApprovalId,
        decision: Decision,
    ) -> ProviderFuture<'_, ProviderDecisionDelivery> {
        self.decisions.lock().unwrap().push((id, decision));
        Box::pin(async { Ok(ProviderDecisionDelivery::complete()) })
    }

    fn submit_questionnaire(
        &self,
        id: QuestionnaireId,
        submission: QuestionnaireSubmission,
    ) -> ProviderFuture<'_, ()> {
        let refused = self
            .submissions
            .refusing
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if refused {
            return Box::pin(async {
                Err(ProviderError::questionnaire_rejected(
                    "The agent did not take the answers.",
                ))
            });
        }
        self.submissions
            .delivered
            .lock()
            .unwrap()
            .push((id, submission));
        Box::pin(async { Ok(()) })
    }

    fn start_turn(&self, _input: ProviderTurnInput) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn steer_turn(&self, _input: ProviderSteerInput) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn interrupt_turn(&self) -> ProviderFuture<'_, ProviderInterruption> {
        let _ = self.script.send(Ok(ProviderEvent::TurnInterrupted.into()));
        Box::pin(async { Ok(ProviderInterruption::Stopped) })
    }

    fn stop_subagents(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn stop_subagent(&self, _subagent_id: ProviderSubagentId) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn shutdown(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// The Broker as a Session's Agent reaches it: JSON-RPC posted to its MCP
/// endpoint under the bearer token the Session's start carried.
pub struct Broker {
    http: reqwest::Client,
    endpoint: String,
    authorization: String,
    next_id: u64,
}

impl Broker {
    /// The Broker `handoff` hands, the MCP session opened.
    async fn handed(handoff: &BrokerHandoff) -> Self {
        let mut broker = Self {
            http: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("build the Broker's client"),
            endpoint: handoff.endpoint().as_str().to_owned(),
            authorization: handoff.token().bearer(),
            next_id: 0,
        };
        broker
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "shaman-thin-client-test", "version": "0" },
                }),
            )
            .await;
        broker
            .post(&serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        broker
    }

    async fn post(&self, message: &serde_json::Value) -> reqwest::Response {
        self.http
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .header("authorization", &self.authorization)
            .json(message)
            .send()
            .await
            .expect("reach the Broker")
    }

    async fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.next_id += 1;
        let id = self.next_id;
        let response = self
            .post(&serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params,
            }))
            .await;
        let streamed = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        let body = response.text().await.expect("read the Broker's answer");
        let answer = if streamed {
            body.split("\n\n")
                .filter_map(|event| {
                    let data = event
                        .lines()
                        .filter_map(|line| line.strip_prefix("data:"))
                        .map(str::trim_start)
                        .collect::<Vec<_>>()
                        .join("\n");
                    serde_json::from_str::<serde_json::Value>(&data).ok()
                })
                .find(|message| message["id"] == serde_json::json!(id))
                .unwrap_or_else(|| panic!("the stream answers {method}: {body}"))
        } else {
            serde_json::from_str(&body).unwrap_or_else(|_| panic!("{method} answers JSON: {body}"))
        };
        answer
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} was answered with an error: {answer}"))
    }

    /// Calls the Tool `tool` with `arguments`, answering what it answered
    /// with, having checked it refused nothing.
    pub async fn call(&mut self, tool: &str, arguments: serde_json::Value) -> serde_json::Value {
        let result = self
            .request(
                "tools/call",
                serde_json::json!({ "name": tool, "arguments": arguments }),
            )
            .await;
        assert_ne!(
            result["isError"],
            serde_json::json!(true),
            "{tool} answers: {result}"
        );
        result["structuredContent"].clone()
    }
}
