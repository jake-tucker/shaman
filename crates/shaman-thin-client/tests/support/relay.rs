//! A real Relay in process, whose logins the test carries out at its identity
//! provider, known by the address of a route to it bound before it starts.
//! The route reads what each connection carried through it asks of the
//! Relay, as a Server or a phone says it, and notes it by the key that
//! connection proved, so a test sees who stood at the Relay and asked what.

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use shaman_relay::{Admission, AdmissionRule, Identity, RelayConfig, ScriptedProvider};

pub struct TestRelay {
    address: String,
    provider: Arc<ScriptedProvider>,
    _directory: tempfile::TempDir,
    running: shaman_relay::RunningRelay,
    route: tokio::task::JoinHandle<()>,
    /// What each connection carried through the route asked, in the order
    /// they were made.
    connections: Arc<Mutex<Vec<Carried>>>,
}

/// One connection the route carried to the Relay.
#[derive(Clone, Debug, Default)]
pub struct Carried {
    /// The fingerprint of the key the connection said hello with.
    pub fingerprint: Option<String>,
    /// What it asked of the Relay, by each message's type, in order.
    pub asked: Vec<String>,
    /// Whether the route carries it still.
    pub open: bool,
}

impl Carried {
    /// Whether it carries a join to a Serving Server, rather than standing
    /// at the Relay itself.
    pub fn joins(&self) -> bool {
        self.asked.iter().any(|asked| asked == "join")
    }
}

impl TestRelay {
    pub async fn start() -> Self {
        let directory = tempfile::tempdir().expect("create the Relay's directory");
        let provider = Arc::new(ScriptedProvider::new());
        // The route comes first, since the Relay is known by the address it
        // is reached at.
        let route = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the route to the Relay");
        let address = format!(
            "http://{}",
            route.local_addr().expect("the route's address")
        );
        let running = shaman_relay::start(
            RelayConfig::new(
                (std::net::Ipv4Addr::LOCALHOST, 0).into(),
                directory.path().join("relay.db"),
                address.clone(),
            )
            .with_connection_log(std::io::sink())
            .with_admission(Admission::by([provider.clone() as Arc<dyn AdmissionRule>]))
            // Checked often, so an Account the test stops admitting lapses at
            // once.
            .with_admission_interval(std::time::Duration::from_millis(10)),
            provider.clone(),
        )
        .await
        .expect("start the Relay");
        let relay = running.address();
        let connections = Arc::new(Mutex::new(Vec::<Carried>::new()));
        let noted = connections.clone();
        let route = tokio::spawn(async move {
            while let Ok((accepted, _)) = route.accept().await {
                let noted = noted.clone();
                tokio::spawn(async move {
                    let Ok(onward) = tokio::net::TcpStream::connect(relay).await else {
                        return;
                    };
                    let index = {
                        let mut noted = noted.lock().expect("the route's notes");
                        noted.push(Carried {
                            open: true,
                            ..Carried::default()
                        });
                        noted.len() - 1
                    };
                    carry(accepted, onward, &noted, index).await;
                    noted.lock().expect("the route's notes")[index].open = false;
                });
            }
        });
        Self {
            address,
            provider,
            _directory: directory,
            running,
            route,
            connections,
        }
    }

    /// Every connection the route has carried for the key whose fingerprint
    /// is `fingerprint`.
    pub fn carried_for(&self, fingerprint: &str) -> Vec<Carried> {
        self.connections
            .lock()
            .expect("the route's notes")
            .iter()
            .filter(|carried| carried.fingerprint.as_deref() == Some(fingerprint))
            .cloned()
            .collect()
    }

    /// The address the Relay is reached at.
    pub fn address(&self) -> String {
        self.address.clone()
    }

    /// Logs in the login whose code is `user_code` as the user `username`
    /// at the identity provider, answering whether one waited under it. Two
    /// users of different names are two Accounts.
    pub fn approve(&self, user_code: &str, username: &str) -> bool {
        self.provider.approve(
            user_code,
            Identity {
                subject: format!("subject-{username}"),
                username: username.to_owned(),
            },
        )
    }
}

impl TestRelay {
    /// Has the Relay admit the user `username` or not, as its rules come to
    /// say: one no longer admitted lapses, and the Relay refuses every Login
    /// under it until a fresh login once it is admitted again.
    pub async fn admit(&self, username: &str, admitted: bool) {
        self.provider
            .set_admitted(&format!("subject-{username}"), admitted);
        if admitted {
            return;
        }
        tokio::time::timeout(super::PROGRESS_DEADLINE, async {
            loop {
                let accounts = self
                    .running
                    .store()
                    .accounts()
                    .await
                    .expect("read the Accounts");
                if accounts.iter().any(|account| account.lapsed) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the Account lapses");
    }
}

/// Carries `accepted` to `onward` and back as it is, reading what the side
/// that dialled says as it goes: past the request that opens the WebSocket,
/// each text frame it sends — masked, as a client's are — is a message whose
/// type is noted, and the key of its hello.
async fn carry(
    accepted: tokio::net::TcpStream,
    onward: tokio::net::TcpStream,
    noted: &Mutex<Vec<Carried>>,
    index: usize,
) {
    let (mut from_dialler, mut to_dialler) = accepted.into_split();
    let (mut from_relay, mut to_relay) = onward.into_split();
    let back = async move {
        let _ = tokio::io::copy(&mut from_relay, &mut to_dialler).await;
        let _ = to_dialler.shutdown().await;
    };
    let forth = async move {
        let mut said = Said::default();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = match from_dialler.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            if to_relay.write_all(&buffer[..read]).await.is_err() {
                break;
            }
            for message in said.take(&buffer[..read]) {
                let mut noted = noted.lock().expect("the route's notes");
                let carried = &mut noted[index];
                if let Some(key) = message.get("key").and_then(serde_json::Value::as_str)
                    && let Ok(key) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(key)
                {
                    carried.fingerprint = Some(shaman_pairing::fingerprint(&key));
                }
                if let Some(kind) = message.get("type").and_then(serde_json::Value::as_str) {
                    carried.asked.push(kind.to_owned());
                }
            }
        }
        let _ = to_relay.shutdown().await;
    };
    tokio::join!(back, forth);
}

/// What the dialling side of one connection has said so far, read a frame
/// at a time.
#[derive(Default)]
struct Said {
    /// Whether the request that opens the WebSocket has been read past.
    opened: bool,
    unread: Vec<u8>,
}

impl Said {
    /// Takes in `bytes`, answering each whole JSON message they complete.
    fn take(&mut self, bytes: &[u8]) -> Vec<serde_json::Value> {
        self.unread.extend_from_slice(bytes);
        if !self.opened {
            let Some(end) = self
                .unread
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
            else {
                return Vec::new();
            };
            self.unread.drain(..end + 4);
            self.opened = true;
        }
        let mut messages = Vec::new();
        while let Some((opcode, payload, length)) = frame(&self.unread) {
            self.unread.drain(..length);
            // A text frame is a message; a binary one is what a join
            // carries, which is not the Relay's to read.
            if opcode == 1
                && let Ok(message) = serde_json::from_slice(&payload)
            {
                messages.push(message);
            }
        }
        messages
    }
}

/// The whole WebSocket frame `bytes` begins with, where they hold one: its
/// opcode, its payload unmasked, and how many bytes it took.
fn frame(bytes: &[u8]) -> Option<(u8, Vec<u8>, usize)> {
    let [first, second, ..] = *bytes else {
        return None;
    };
    let masked = second & 0x80 != 0;
    let (length, mut at) = match second & 0x7f {
        126 => (
            u64::from(u16::from_be_bytes(bytes.get(2..4)?.try_into().ok()?)),
            4,
        ),
        127 => (u64::from_be_bytes(bytes.get(2..10)?.try_into().ok()?), 10),
        length => (u64::from(length), 2),
    };
    let mask = if masked {
        let mask: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        at += 4;
        Some(mask)
    } else {
        None
    };
    let end = at + usize::try_from(length).ok()?;
    let mut payload = bytes.get(at..end)?.to_vec();
    if let Some(mask) = mask {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Some((first & 0x0f, payload, end))
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.route.abort();
    }
}
