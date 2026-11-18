//! What the core's tests stand on: a real Serving Server in process, a way to
//! it the test holds shut or opens, and a phone with nothing but the core.

#![allow(dead_code)]

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
        AgentId, AgentIdentity, AgentSelection, ApprovalPosture, CreateSessionRequest,
        ExecutionDirectory, InitialPrompt, IssueInviteRequest, ModelId, Peer, PromptId, ProviderId,
        SessionId, SettingMutation, SkillCatalog, Way,
    },
    provider::{
        AttributedProviderEvent, ErrandFuture, ProviderErrand, ProviderError, ProviderEvent,
        ProviderEventStream, ProviderFuture, ProviderInterruption, ProviderModelDiscovery,
        ProviderPostureApplication, ProviderRuntime, ProviderSession, ProviderSessionConnection,
        ProviderSessionRequest, ProviderSteerInput, ProviderTurnInput,
    },
    server::{self, ServerConfig, ServerTimings},
};
use shaman_client::SessionCommandTimings;
use shaman_pairing::{
    Keepalive, RedeemerTimings,
    signer::{Signer as _, testing::CallbackSigner},
};
use shaman_thin_client::{
    ActsListener, ConversationChange, ConversationItem, ConversationListener, CoreTimings,
    DeviceListener, FrameListener, KeySigner, Listing, ListingListener, OpenSession, Outlook,
    OutlookListener, Servers, ServersListener, SessionFrame, SignError, Subscription, ThinClient,
    ThisDevice,
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
    _state: tempfile::TempDir,
    config: ServerConfig,
    _config_root: tempfile::TempDir,
    server: server::RunningServer,
    client: ManagedClient,
    scripts: Arc<Mutex<Vec<Script>>>,
}

impl ServingServer {
    pub async fn start(channel: &str) -> Self {
        let state = tempfile::tempdir().expect("create the Server's state directory");
        let config_root = tempfile::tempdir().expect("create the Server's config root");
        let config = ServerConfig::new(state.path(), channel)
            .expect("configure the Server")
            .with_config_dir(config_root.path());
        let timings = ServerTimings {
            shutdown_grace: Duration::from_millis(5),
            ..ServerTimings::default()
        };
        let provider = Arc::new(TestProvider::default());
        let scripts = provider.scripts.clone();
        let server = server::spawn_with_provider_and_timings(config.clone(), provider, timings)
            .await
            .expect("spawn the Server");
        let client = ManagedClient::connect(
            ManagedClientConfig::new(state.path(), channel).expect("configure the Client"),
        )
        .await
        .expect("attach the Client");
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
            _state: state,
            config,
            _config_root: config_root,
            server,
            client,
            scripts,
        }
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
        self.client
            .create_session(CreateSessionRequest {
                session_id: None,
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

    /// The hostname this Server gives itself in its Invites.
    pub fn hostname(&self) -> String {
        shaman_pairing::machine_hostname()
    }

    pub async fn shutdown(self) {
        self.server.shutdown().await.expect("stop the Server");
    }
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

    /// Shuts the way again: every connection it carries is cut, and each
    /// made from now on connects and hears nothing until it opens.
    pub fn shut_now(&self) {
        self.open.send_replace(false);
        for proxy in self.carrying.lock().unwrap().drain(..) {
            proxy.abort();
        }
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
    pub core: Arc<ThinClient>,
}

impl Phone {
    pub fn new() -> Self {
        Self::with(timings())
    }

    /// A phone whose core waits as `timings` say.
    pub fn with(timings: CoreTimings) -> Self {
        let directory = tempfile::tempdir().expect("create the phone's directory");
        let key = TestKey::new();
        let core = open(&directory, &key, timings);
        Self {
            directory,
            key,
            timings,
            core,
        }
    }

    /// The directory the core keeps everything in.
    pub fn directory(&self) -> &std::path::Path {
        self.directory.path()
    }

    /// The phone after the app was ended and launched again.
    pub fn relaunched(self) -> Self {
        let Self {
            directory,
            key,
            timings,
            core,
        } = self;
        drop(core);
        let core = open(&directory, &key, timings);
        Self {
            directory,
            key,
            timings,
            core,
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

    /// Opens the Session `reference` names, as a screen pushed to read it.
    pub fn open(&self, reference: &str) -> Reading {
        let session = self.core.open_session(reference.to_owned());
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
) -> Arc<ThinClient> {
    ThinClient::open_with(
        directory.path().to_string_lossy().into_owned(),
        key.clone(),
        timings,
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

impl ActsListener for Heard<bool> {
    fn changed(&self, under_way: bool) {
        self.0.send_replace(Some(under_way));
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

/// A Provider whose Sessions, but a scripted one, never start: one begun in
/// a directory named [`WORKING`] waits on it for ever, and any other fails.
#[derive(Default)]
struct TestProvider {
    /// What each scripted Session's Provider sends, the latest started last.
    scripts: Arc<Mutex<Vec<Script>>>,
}

impl ProviderRuntime for TestProvider {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new("none")
    }

    fn display_name(&self) -> &str {
        "none"
    }

    fn list_models(&self) -> ProviderFuture<'_, ProviderModelDiscovery> {
        Box::pin(async { Err(ProviderError::new("Model discovery is unavailable.")) })
    }

    fn skill_catalog(&self, _workspace: &std::path::Path) -> ProviderFuture<'_, SkillCatalog> {
        Box::pin(async { Err(ProviderError::new("No Skills are offered.")) })
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
        let (working, scripted) = (named(WORKING), named(SCRIPTED));
        let scripts = self.scripts.clone();
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
                    Arc::new(ScriptedSession(script)),
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

    fn shutdown(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// A scripted Session's Provider side: it takes every Turn and steer, and
/// stops a Turn by saying so.
struct ScriptedSession(Script);

impl ProviderSession for ScriptedSession {
    fn update_approval_posture(
        &self,
        _posture: ApprovalPosture,
        _has_active_work: bool,
    ) -> ProviderFuture<'_, ProviderPostureApplication> {
        Box::pin(async { Ok(ProviderPostureApplication::NextTurn) })
    }

    fn start_turn(&self, _input: ProviderTurnInput) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn steer_turn(&self, _input: ProviderSteerInput) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn interrupt_turn(&self) -> ProviderFuture<'_, ProviderInterruption> {
        let _ = self.0.send(Ok(ProviderEvent::TurnInterrupted.into()));
        Box::pin(async { Ok(ProviderInterruption::Stopped) })
    }

    fn stop_subagents(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn shutdown(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}
