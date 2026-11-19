//! A real Relay in process, whose logins the test carries out at its identity
//! provider, known by the address of a route to it bound before it starts.

use std::sync::Arc;

use shaman_relay::{Admission, AdmissionRule, Identity, RelayConfig, ScriptedProvider};

pub struct TestRelay {
    address: String,
    provider: Arc<ScriptedProvider>,
    _directory: tempfile::TempDir,
    _running: shaman_relay::RunningRelay,
    route: tokio::task::JoinHandle<()>,
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
            .with_admission(Admission::by([provider.clone() as Arc<dyn AdmissionRule>])),
            provider.clone(),
        )
        .await
        .expect("start the Relay");
        let relay = running.address();
        let route = tokio::spawn(async move {
            while let Ok((mut accepted, _)) = route.accept().await {
                tokio::spawn(async move {
                    if let Ok(mut onward) = tokio::net::TcpStream::connect(relay).await {
                        let _ = tokio::io::copy_bidirectional(&mut accepted, &mut onward).await;
                    }
                });
            }
        });
        Self {
            address,
            provider,
            _directory: directory,
            _running: running,
            route,
        }
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

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.route.abort();
    }
}
