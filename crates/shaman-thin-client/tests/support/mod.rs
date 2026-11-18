//! What the core's tests stand on: a real Serving Server in process, a way to
//! it the test holds shut or opens, and a phone with nothing but the core.

#![allow(dead_code)]

use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use shaman::{
    managed_client::{ManagedClient, ManagedClientConfig},
    protocol::{
        AgentSelection, IssueInviteRequest, Peer, ProviderId, SettingMutation, SkillCatalog, Way,
    },
    provider::{
        ErrandFuture, ProviderErrand, ProviderError, ProviderFuture, ProviderModelDiscovery,
        ProviderRuntime, ProviderSessionConnection, ProviderSessionRequest,
    },
    server::{self, ServerConfig, ServerTimings},
};
use shaman_pairing::{
    Keepalive, RedeemerTimings,
    signer::{Signer as _, testing::CallbackSigner},
};
use shaman_thin_client::{
    CoreTimings, DeviceListener, KeySigner, Outlook, OutlookListener, Servers, ServersListener,
    SignError, Subscription, ThinClient, ThisDevice,
};
use tokio::{net::TcpListener, sync::watch, time::timeout};

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
    }
}

/// A Server of Shaman's own in process, Serving on loopback, whose Sessions
/// never start.
pub struct ServingServer {
    _state: tempfile::TempDir,
    config: ServerConfig,
    _config_root: tempfile::TempDir,
    server: server::RunningServer,
    client: ManagedClient,
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
        let server =
            server::spawn_with_provider_and_timings(config.clone(), Arc::new(NoProvider), timings)
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
        }
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
/// hears nothing, or opens, so that it carries each connection there.
#[derive(Clone)]
pub struct Gate {
    address: SocketAddr,
    open: watch::Sender<bool>,
    dialled: Arc<AtomicU64>,
}

impl Gate {
    /// A way to `target`, carrying each connection there at once.
    pub async fn open(target: SocketAddr) -> Self {
        let gate = Self::held(target).await;
        gate.open_now();
        gate
    }

    /// A way to `target` held shut until it is opened.
    pub async fn held(target: SocketAddr) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listen for the way");
        let address = listener.local_addr().expect("the way's address");
        let open = watch::Sender::new(false);
        let dialled = Arc::new(AtomicU64::new(0));
        let mut opened = open.subscribe();
        let counted = dialled.clone();
        tokio::spawn(async move {
            // Until it opens, nothing is accepted: a dial connects, and its
            // handshake hears nothing.
            if opened.wait_for(|open| *open).await.is_err() {
                return;
            }
            while let Ok((mut inbound, _)) = listener.accept().await {
                counted.fetch_add(1, Ordering::AcqRel);
                tokio::spawn(async move {
                    if let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
            }
        });
        Self {
            address,
            open,
            dialled,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn open_now(&self) {
        self.open.send_replace(true);
    }

    /// How many connections the way has carried.
    pub fn dialled(&self) -> u64 {
        self.dialled.load(Ordering::Acquire)
    }
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

    /// The fingerprint a Serving Server knows this phone by.
    pub fn fingerprint(&self) -> String {
        shaman_pairing::fingerprint(&self.key.public_key())
    }

    pub async fn outlook(&self) -> Outlook {
        let (sender, mut heard) = watch::channel(None);
        let _subscription = self.core.watch_outlook(Arc::new(Heard(sender)));
        heard_at_once(&mut heard).await
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

    pub fn shutdown(self) {
        drop(self.core);
        drop(self.directory);
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

impl OutlookListener for Heard<Outlook> {
    fn changed(&self, outlook: Outlook) {
        self.0.send_replace(Some(outlook));
    }
}

/// A Provider no Session of the Server's starts on.
struct NoProvider;

impl ProviderRuntime for NoProvider {
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
        _request: ProviderSessionRequest,
    ) -> ProviderFuture<'_, ProviderSessionConnection> {
        Box::pin(async { Err(ProviderError::new("No Session starts on this test Server.")) })
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
