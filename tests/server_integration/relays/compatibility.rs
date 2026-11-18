//! A Peer and a Serving Server on different releases (ADR-0078): each states
//! the version it speaks on everything it says and speaks with the other
//! while that is at or above its own floor. Where it is not, the side that
//! judged refuses naming the side to update, the Serving side lists a Peer
//! it refused as behind, and the Pairing stands, so updating heals it.

use shaman::{
    protocol::{
        Peer, RemoteHealth, RemoteStatus, SessionError, SessionErrorCode, SettingMutation, Way,
    },
    server::{PairingProtocol, ServerTimings},
};

use super::{NEWER, NEWER_SPEAKING, OLDER, TestServer, error_code, pairing::REMOTE, relay_timings};

/// The header each side states the version it speaks under.
const STATED: &str = "x-shaman-protocol-version";

/// A Server speaking `protocol` across its Pairings.
async fn speaking(channel: &str, protocol: PairingProtocol) -> TestServer {
    TestServer::with_timings(
        channel,
        ServerTimings {
            pairing_protocol: protocol,
            ..relay_timings()
        },
    )
    .await
}

/// A workstation Serving directly, at a port it keeps across restarts, and a
/// laptop that redeems its Invites, each speaking as a test has it.
struct Machines {
    workstation: TestServer,
    laptop: TestServer,
}

impl Machines {
    async fn speaking(
        channel: &str,
        workstation: PairingProtocol,
        laptop: PairingProtocol,
    ) -> Self {
        let workstation = speaking(&format!("{channel}-workstation"), workstation).await;
        workstation.serve().await;
        // The port it was given is pinned, so the way the laptop holds still
        // reaches it once it restarts speaking otherwise.
        workstation
            .client
            .mutate_setting(SettingMutation::ServingPort {
                value: Some(workstation.serving_address().port()),
            })
            .await
            .expect("pin the Serving port");
        let laptop = speaking(&format!("{channel}-laptop"), laptop).await;
        Self {
            workstation,
            laptop,
        }
    }

    /// The same, the laptop paired with the workstation as [`REMOTE`].
    async fn paired(channel: &str, workstation: PairingProtocol, laptop: PairingProtocol) -> Self {
        let machines = Self::speaking(channel, workstation, laptop).await;
        machines.redeem().await.expect("pair the laptop");
        machines
    }

    /// The laptop redeeming an Invite to the workstation, as [`REMOTE`].
    async fn redeem(&self) -> anyhow::Result<shaman::protocol::Remote> {
        let invite = self
            .workstation
            .invite(vec![Way::Direct(self.workstation.serving_address())])
            .await;
        self.laptop.redeem_as(invite, REMOTE).await
    }

    /// The workstation updated, or put back, to speak `protocol`.
    async fn workstation_speaking(&mut self, protocol: PairingProtocol) {
        self.workstation.timings.pairing_protocol = protocol;
        self.workstation.restart().await;
    }

    /// The laptop updated, or put back, to speak `protocol`.
    async fn laptop_speaking(&mut self, protocol: PairingProtocol) {
        self.laptop.timings.pairing_protocol = protocol;
        self.laptop.restart().await;
    }

    /// What the laptop's Server answers its Client listing the workstation's
    /// Sessions with, through its proxy to the Remote.
    async fn list_sessions(&self) -> reqwest::Response {
        let descriptor = self.laptop.server.as_ref().unwrap().descriptor().clone();
        crate::support::loopback_client()
            .get(format!(
                "{}/v1/remotes/{REMOTE}/v1/sessions",
                descriptor.base_url
            ))
            .bearer_auth(&descriptor.token)
            .send()
            .await
            .expect("ask the laptop's Server for the Remote's Sessions")
    }

    /// The laptop's Server refusing its Client the workstation's Sessions,
    /// and why.
    async fn refused_sessions(&self) -> SessionError {
        let refused = self.list_sessions().await;
        assert_eq!(refused.status(), reqwest::StatusCode::CONFLICT);
        refused.json().await.expect("decode the refusal")
    }

    /// The laptop's probe of the workstation.
    async fn probe(&self) -> RemoteHealth {
        self.laptop
            .client
            .probe_remote(REMOTE)
            .await
            .expect("probe the Remote")
    }

    /// The Peers the workstation lists.
    async fn peers(&self) -> Vec<Peer> {
        self.workstation
            .client
            .list_peers()
            .await
            .expect("list the Peers")
    }

    /// The Remotes the laptop holds.
    async fn remotes(&self) -> usize {
        self.laptop
            .client
            .list_remotes()
            .await
            .expect("list the Remotes")
            .len()
    }

    async fn shutdown(self) {
        self.laptop.shutdown().await;
        self.workstation.shutdown().await;
    }
}

#[tokio::test]
async fn each_side_speaks_with_one_at_or_above_its_floor_however_much_newer() {
    // A Serving Server newer than its Peer, whose floor the Peer is at.
    let mut machines =
        Machines::paired("speaks-newer", NEWER_SPEAKING, PairingProtocol::CURRENT).await;
    let health = machines.probe().await;
    assert_eq!(health.status, RemoteStatus::Available);
    assert_eq!(health.protocol_version, Some(NEWER_SPEAKING.version));
    let listed = machines.list_sessions().await;
    assert!(listed.status().is_success(), "{}", listed.status());
    assert_eq!(
        listed
            .headers()
            .get(STATED)
            .and_then(|stated| stated.to_str().ok()),
        Some(NEWER_SPEAKING.version.to_string().as_str()),
        "the Serving side states its version on its answers"
    );
    assert!(machines.peers().await.iter().all(|peer| !peer.behind));

    // A Peer newer than its Serving Server, whose floor that Server is at.
    machines
        .workstation_speaking(PairingProtocol::CURRENT)
        .await;
    machines.laptop_speaking(NEWER_SPEAKING).await;
    let health = machines.probe().await;
    assert_eq!(health.status, RemoteStatus::Available);
    assert_eq!(
        health.protocol_version,
        Some(PairingProtocol::CURRENT.version)
    );
    assert!(machines.list_sessions().await.status().is_success());
    assert!(machines.peers().await.iter().all(|peer| !peer.behind));

    machines.shutdown().await;
}

#[tokio::test]
async fn a_peer_below_the_serving_floor_is_refused_as_behind_listed_so_and_healed_by_its_update() {
    let mut machines = Machines::paired(
        "peer-behind",
        PairingProtocol::CURRENT,
        PairingProtocol::CURRENT,
    )
    .await;
    machines.workstation_speaking(NEWER).await;

    let refusal = machines.refused_sessions().await;
    assert_eq!(refusal.code, SessionErrorCode::PairingPeerBehind);
    assert_eq!(
        refusal.message,
        "That Server no longer works with a Shaman as old as this one: update Shaman here, and \
         the pairing works again"
    );
    let health = machines.probe().await;
    assert_eq!(health.status, RemoteStatus::Ahead);
    assert_eq!(health.protocol_version, Some(NEWER.version));
    let peers = machines.peers().await;
    assert_eq!(peers.len(), 1, "the Pairing stands on the Serving side");
    assert!(peers[0].behind, "the refused Peer is listed as behind");
    assert_eq!(
        machines.remotes().await,
        1,
        "the Pairing stands on the Peer"
    );

    // The Peer updated, the Pairing works again with no second ceremony.
    machines.laptop_speaking(NEWER).await;
    let health = machines.probe().await;
    assert_eq!(health.status, RemoteStatus::Available);
    assert!(machines.list_sessions().await.status().is_success());
    assert!(
        !machines.peers().await[0].behind,
        "the Peer is behind no longer"
    );

    machines.shutdown().await;
}

#[tokio::test]
async fn a_serving_server_below_the_peers_floor_is_refused_as_behind_and_healed_by_its_update() {
    let mut machines = Machines::paired(
        "server-behind",
        PairingProtocol::CURRENT,
        PairingProtocol::CURRENT,
    )
    .await;
    machines.workstation_speaking(OLDER).await;

    let refusal = machines.refused_sessions().await;
    assert_eq!(refusal.code, SessionErrorCode::PairingServerBehind);
    assert_eq!(
        refusal.message,
        "That Server's Shaman is too old to work with this one: update Shaman there, and the \
         pairing works again"
    );
    let health = machines.probe().await;
    assert_eq!(health.status, RemoteStatus::Behind);
    assert_eq!(health.protocol_version, Some(OLDER.version));
    let peers = machines.peers().await;
    assert_eq!(peers.len(), 1, "the Pairing stands on the Serving side");
    assert!(
        !peers[0].behind,
        "the Serving side speaks with the Peer, so it is not behind there"
    );
    assert_eq!(
        machines.remotes().await,
        1,
        "the Pairing stands on the Peer"
    );

    // The Serving Server updated, the Pairing works again.
    machines
        .workstation_speaking(PairingProtocol::CURRENT)
        .await;
    assert_eq!(machines.probe().await.status, RemoteStatus::Available);
    assert!(machines.list_sessions().await.status().is_success());

    machines.shutdown().await;
}

#[tokio::test]
async fn the_pairing_ceremony_is_refused_naming_the_side_behind_in_both_directions() {
    // A Serving Server whose floor the redeeming side is below.
    let machines = Machines::speaking("ceremony-peer", NEWER, PairingProtocol::CURRENT).await;
    let refused = machines
        .redeem()
        .await
        .expect_err("a Serving Server that no longer pairs with one as old");
    assert_eq!(error_code(&refused), SessionErrorCode::PairingPeerBehind);
    assert!(
        refused.to_string().contains("update Shaman here"),
        "{refused}"
    );
    assert_eq!(machines.remotes().await, 0, "no Pairing was formed");
    assert!(machines.peers().await.is_empty(), "no Peer was enrolled");
    machines.shutdown().await;

    // A Serving Server below the redeeming side's floor.
    let machines = Machines::speaking("ceremony-server", OLDER, PairingProtocol::CURRENT).await;
    let refused = machines
        .redeem()
        .await
        .expect_err("a Serving Server too old to pair with");
    assert_eq!(error_code(&refused), SessionErrorCode::PairingServerBehind);
    assert!(
        refused.to_string().contains("update Shaman there"),
        "{refused}"
    );
    assert_eq!(machines.remotes().await, 0, "no Pairing was formed");
    assert!(machines.peers().await.is_empty(), "no Peer was enrolled");
    machines.shutdown().await;

    // And across releases each speaks with, the ceremony pairs them.
    let machines =
        Machines::speaking("ceremony-across", PairingProtocol::CURRENT, NEWER_SPEAKING).await;
    machines.redeem().await.expect("pair across releases");
    assert_eq!(machines.probe().await.status, RemoteStatus::Available);
    machines.shutdown().await;
}
