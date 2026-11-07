//! Pairing back: the ceremony that forms one Pairing offering the opposite one,
//! between two real Servers joined directly and through a real Relay
//! (ADR-0066). `laptop` is the machine whose user pairs and offers, redeeming
//! `workstation`'s Invite; the second Pairing has `workstation` reach `laptop`.

use std::time::Duration;

use eventsource_stream::Eventsource as _;
use futures_util::StreamExt as _;
use http_body_util::Full;
use hyper::{Method, Request, StatusCode};
use suru::{
    managed_client::ManagedEvent,
    protocol::{
        AnswerOfferRequest, HeldOffer, InviteText, IssueInviteRequest, MadeOffer, OfferHeldUp,
        Outlook, PROTOCOL_VERSION, PairBackOffers, RedeemInviteRequest, Remote, RemoteStatus,
        SessionErrorCode, Way,
    },
    server::ServerTimings,
};
use tokio::time::timeout;

use super::{
    TestRelay, TestServer, error_code,
    pairing::{REMOTE, next_catalog_event},
    relay_timings,
    serving::{Multiplexed, paired_tls},
};
use crate::support::PROGRESS_DEADLINE;

/// What `workstation` is offered to call `laptop`, once it accepts.
const LAPTOP: &str = "laptop-back";

/// A try that came to nothing is tried again soon, so tests wait on no default.
fn timings() -> ServerTimings {
    relay_timings().with_pair_back_retry_interval(Duration::from_millis(25))
}

impl TestServer {
    /// The way this Server's Serving listener is reached.
    fn way(&self) -> Way {
        Way::Direct(self.serving_address())
    }

    /// Redeems `invite`, naming the Remote [`REMOTE`] and offering to pair
    /// back, as Configure Remote does with its option on.
    async fn redeem_offering(&self, invite: impl Into<InviteText>) -> anyhow::Result<Remote> {
        self.client
            .redeem_invite(RedeemInviteRequest {
                invite: invite.into(),
                name: Some(REMOTE.to_owned()),
                ways: Vec::new(),
                pair_back: true,
            })
            .await
    }

    async fn offers(&self) -> PairBackOffers {
        self.client.list_offers().await.expect("list the offers")
    }

    /// Waits until this Server's offers stand as `reached` says.
    async fn offers_become(&self, reached: impl Fn(&PairBackOffers) -> bool) -> PairBackOffers {
        let waiting = timeout(PROGRESS_DEADLINE, async {
            loop {
                let offers = self.offers().await;
                if reached(&offers) {
                    return offers;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        match waiting {
            Ok(offers) => offers,
            Err(_) => panic!(
                "the offers never came to stand as expected; they stand as {:?}",
                self.offers().await
            ),
        }
    }

    /// Waits until this Server holds no offers of either kind.
    async fn offers_clear(&self) {
        self.offers_become(|offers| offers.held.is_empty() && offers.made.is_empty())
            .await;
    }

    /// The offer this Server holds, there being one.
    async fn held_offer(&self) -> HeldOffer {
        let offers = self.offers().await;
        assert_eq!(offers.held.len(), 1, "one offer held: {offers:?}");
        offers.held.into_iter().next().unwrap()
    }

    /// The offer this Server made, there being one.
    async fn made_offer(&self) -> MadeOffer {
        let offers = self.offers().await;
        assert_eq!(offers.made.len(), 1, "one offer made: {offers:?}");
        offers.made.into_iter().next().unwrap()
    }

    /// Answers the offer this Server holds by accepting it.
    async fn accept_offer(&self) {
        let offer = self.held_offer().await;
        self.client
            .answer_offer(
                offer.id,
                AnswerOfferRequest::Accept {
                    name: LAPTOP.to_owned(),
                },
            )
            .await
            .expect("accept the offer");
    }

    /// Connects to its Remote, as a Client looking at it does.
    async fn looks_at_remote(&self) {
        self.client
            .probe_remote(REMOTE)
            .await
            .expect("probe the Remote");
    }

    /// Whether it holds a Remote named `name`, and as what.
    async fn remote_named(&self, name: &str) -> Option<Remote> {
        self.client
            .list_remotes()
            .await
            .expect("list the Remotes")
            .into_iter()
            .find(|remote| remote.name == name)
    }

    async fn remote_comes_to_be(&self, name: &str) -> Remote {
        timeout(PROGRESS_DEADLINE, async {
            loop {
                if let Some(remote) = self.remote_named(name).await {
                    return remote;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no Remote named {name} came to be"))
    }

    /// Waits until an Invite of this Server's would offer `relay`.
    async fn relay_is_offered(&self, relay: &TestRelay) {
        timeout(PROGRESS_DEADLINE, async {
            while self
                .client
                .issue_invite(IssueInviteRequest {
                    ways: vec![Way::Relay(relay.address())],
                })
                .await
                .is_err()
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the Relay comes to be offered");
    }

    async fn peer_count(&self) -> usize {
        self.client
            .list_peers()
            .await
            .expect("list the Peers")
            .len()
    }
}

/// A workstation and a laptop each Serving directly, not paired yet, the laptop
/// able to offer.
async fn serving_directly(channel: &str) -> (TestServer, TestServer) {
    serving_directly_by(channel, timings()).await
}

async fn serving_directly_by(channel: &str, timings: ServerTimings) -> (TestServer, TestServer) {
    let workstation =
        TestServer::with_timings(&format!("{channel}-workstation"), timings.clone()).await;
    let laptop = TestServer::with_timings(&format!("{channel}-laptop"), timings).await;
    workstation.serve().await;
    laptop.serve().await;
    (workstation, laptop)
}

/// The same, paired, with the laptop's offer standing on both.
async fn offered_directly(channel: &str) -> (TestServer, TestServer) {
    let (workstation, laptop) = serving_directly(channel).await;
    offer(&workstation, &laptop).await;
    (workstation, laptop)
}

async fn offer(workstation: &TestServer, laptop: &TestServer) {
    let invite = workstation.invite(vec![workstation.way()]).await;
    laptop
        .redeem_offering(invite)
        .await
        .expect("pair, offering to pair back");
}

/// What is stored under `server`, gathered from every file its records are in.
fn records_of(server: &TestServer) -> String {
    std::fs::read_dir(server.config.data_dir())
        .expect("read the Server's data directory")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .collect()
}

#[tokio::test]
async fn leaving_it_off_sends_nothing_and_forms_the_one_way_pairing_asked_for() {
    let (workstation, laptop) = serving_directly("off").await;
    let invite = workstation.invite(vec![workstation.way()]).await;
    laptop.redeem_as(invite, REMOTE).await.expect("pair");

    assert_eq!(workstation.offers().await, PairBackOffers::default());
    assert_eq!(laptop.offers().await, PairBackOffers::default());
    assert_eq!(workstation.peer_count().await, 1);
    assert_eq!(laptop.peer_count().await, 0);
    assert!(workstation.client.list_remotes().await.unwrap().is_empty());

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn an_offer_carries_no_token_and_survives_either_server_restarting() {
    let (mut workstation, mut laptop) = offered_directly("restarts").await;
    let held = workstation.held_offer().await;
    let made = laptop.made_offer().await;
    assert_eq!(held.id, made.id, "one offer, known to both by its id");
    assert!(held.answer.is_none());
    assert!(held.notice, "its Notice is yet to be given");

    // What is held is an id and when, and no token: the only Invite in reach
    // is the one that was redeemed, whose token the laptop's records and the
    // workstation's hold no copy of.
    for records in [records_of(&workstation), records_of(&laptop)] {
        assert!(records.contains(&held.id.to_string()));
        assert!(!records.contains("suru-v1-"), "no Invite stored: {records}");
    }

    workstation.restart().await;
    assert_eq!(workstation.held_offer().await.id, held.id);
    laptop.restart().await;
    assert_eq!(laptop.made_offer().await.id, made.id);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn an_acceptance_with_the_offerer_connected_forms_the_second_pairing_at_once() {
    let (workstation, laptop) = offered_directly("connected").await;
    // Something holds the Remote in view from here on, as a Client looking at
    // it does, so nothing else moves it.
    let mut held_open = laptop
        .client
        .outlook(Outlook::Remote(REMOTE.to_owned()))
        .subscribe_catalog();
    assert!(matches!(
        next_catalog_event(&mut held_open).await,
        Some(ManagedEvent::SessionCatalogReconciled(_))
    ));

    workstation.accept_offer().await;

    let remote = workstation.remote_comes_to_be(LAPTOP).await;
    assert_eq!(remote.fingerprint, laptop.fingerprint());
    workstation.offers_clear().await;
    laptop.offers_clear().await;
    assert_eq!(laptop.peer_count().await, 1, "the workstation, as a Peer");
    drop(held_open);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn an_acceptance_with_the_offerer_away_waits_and_forms_on_its_next_connection() {
    let (workstation, laptop) = offered_directly("away").await;

    workstation.accept_offer().await;
    let waiting = workstation.held_offer().await;
    assert_eq!(
        waiting.answer.as_ref().map(|answer| answer.name.as_str()),
        Some(LAPTOP),
        "it stands accepted under its name, waiting: {waiting:?}"
    );
    assert!(workstation.remote_named(LAPTOP).await.is_none());

    // No further step from anyone: the laptop looks at the Remote.
    laptop.looks_at_remote().await;

    workstation.remote_comes_to_be(LAPTOP).await;
    workstation.offers_clear().await;
    laptop.offers_clear().await;

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn an_acceptance_forms_it_through_a_relay_the_offerer_serves_through() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = super::pairing::serving_through(&relay, "relay", timings()).await;
    // The laptop opens no port: the Relay is the way it has to offer.
    laptop.serve().await;
    laptop.listen(false).await.expect("close the listener");
    laptop.serve_through(&relay, true).await;
    laptop.relay_is_offered(&relay).await;
    let invite = workstation.invite(vec![Way::Relay(relay.address())]).await;
    laptop.redeem_offering(invite).await.expect("pair");

    workstation.accept_offer().await;
    laptop.wait_for_remote(RemoteStatus::Available).await;

    let remote = workstation.remote_comes_to_be(LAPTOP).await;
    assert_eq!(remote.fingerprint, laptop.fingerprint());
    assert_eq!(remote.ways, vec![Way::Relay(relay.address())]);
    workstation.offers_clear().await;
    laptop.offers_clear().await;

    // The second Pairing shows up where machines are found, by its key.
    let suru::protocol::RelayMachinesAnswer::Listed { machines, .. } = workstation
        .client
        .list_relay_machines(&relay.address())
        .await
        .expect("ask the Relay")
        .answer
    else {
        panic!("the Relay lists its machines");
    };
    let found = machines
        .iter()
        .find(|machine| machine.fingerprint == laptop.fingerprint())
        .expect("the laptop is listed");
    assert_eq!(found.remote.as_deref(), Some(LAPTOP));

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_decline_clears_it_at_once_there_and_here_once_the_offerer_next_connects() {
    let (workstation, laptop) = offered_directly("decline").await;
    let offer = workstation.held_offer().await;

    workstation
        .client
        .answer_offer(offer.id, AnswerOfferRequest::Decline)
        .await
        .expect("decline");
    assert!(workstation.offers().await.held.is_empty());
    assert_eq!(
        laptop.made_offer().await.id,
        offer.id,
        "it has yet to hear of it"
    );

    laptop.looks_at_remote().await;
    laptop.offers_clear().await;
    assert!(workstation.remote_named(LAPTOP).await.is_none());
    assert_eq!(laptop.peer_count().await, 0, "nothing formed");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_withdrawal_issues_nothing_though_accepted_and_reaches_the_other_on_next_connection() {
    let (workstation, laptop) = offered_directly("withdraw").await;
    let offer = laptop.made_offer().await;
    workstation.accept_offer().await;

    laptop
        .client
        .withdraw_offer(offer.id)
        .await
        .expect("withdraw");
    assert!(laptop.made_offer().await.withdrawn);
    assert_eq!(
        workstation.held_offer().await.id,
        offer.id,
        "it has yet to hear"
    );

    laptop.looks_at_remote().await;
    workstation.offers_clear().await;
    laptop.offers_clear().await;
    assert!(workstation.remote_named(LAPTOP).await.is_none());
    assert_eq!(laptop.peer_count().await, 0, "no Invite was issued for it");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn removing_the_remote_ends_the_offer_on_both() {
    let (workstation, laptop) = offered_directly("end-from-remote").await;
    workstation.accept_offer().await;

    let removal = laptop.client.remove_remote(REMOTE).await.expect("remove");
    assert!(removal.acknowledged);

    laptop.offers_clear().await;
    workstation.offers_clear().await;
    assert_eq!(workstation.peer_count().await, 0);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn removing_the_peer_ends_it_at_once_there_and_when_next_refused_here() {
    let (workstation, laptop) = offered_directly("end-from-peer").await;
    let peer = workstation.client.list_peers().await.unwrap().remove(0);

    workstation
        .client
        .remove_peer(&peer.id)
        .await
        .expect("remove the Peer");
    assert!(workstation.offers().await.held.is_empty());
    assert_eq!(laptop.offers().await.made.len(), 1, "yet to hear");

    laptop.wait_for_remote(RemoteStatus::Revoked).await;
    laptop.offers_clear().await;

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn with_no_way_at_acceptance_nothing_is_issued_the_offer_stands_and_it_says_why() {
    let (workstation, laptop) = offered_directly("no-way").await;
    laptop.stop_serving().await;
    workstation.accept_offer().await;

    laptop.looks_at_remote().await;
    let held = workstation
        .offers_become(|offers| {
            offers
                .held
                .first()
                .and_then(|held| held.answer.as_ref())
                .is_some_and(|answer| answer.held_up == Some(OfferHeldUp::ServingOff))
        })
        .await;
    assert_eq!(held.held.len(), 1, "the offer stands");
    assert_eq!(laptop.peer_count().await, 0, "nothing was issued");
    assert_eq!(
        laptop.made_offer().await.held_up,
        Some(OfferHeldUp::ServingOff),
        "the laptop's own reader is told too"
    );

    // The laptop's user turning Serving on, as its own choice, takes no further
    // step: it goes on from there.
    laptop.serve().await;
    laptop.looks_at_remote().await;
    workstation.remote_comes_to_be(LAPTOP).await;
    workstation.offers_clear().await;
    laptop.offers_clear().await;

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn an_offer_cannot_be_made_with_no_way_to_offer_and_the_pairing_is_not_spent_on_trying() {
    let (workstation, laptop) = serving_directly("not-offerable").await;
    laptop.stop_serving().await;
    let invite = workstation.invite(vec![workstation.way()]).await;

    let refused = laptop
        .redeem_offering(invite.clone())
        .await
        .expect_err("it has no way to offer");
    assert_eq!(error_code(&refused), SessionErrorCode::PairBackNotOfferable);
    assert_eq!(workstation.peer_count().await, 0);

    laptop
        .redeem_as(invite, REMOTE)
        .await
        .expect("the Invite stands for a redemption that does not offer");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn the_result_is_an_ordinary_second_pairing_each_ended_on_its_own() {
    let (workstation, laptop) = offered_directly("ordinary").await;
    workstation.accept_offer().await;
    laptop.looks_at_remote().await;
    workstation.remote_comes_to_be(LAPTOP).await;

    workstation
        .client
        .probe_remote(LAPTOP)
        .await
        .expect("the workstation reaches the laptop by the second Pairing");

    // Removing the second leaves the first standing.
    let removal = workstation
        .client
        .remove_remote(LAPTOP)
        .await
        .expect("remove the second Pairing");
    assert!(removal.acknowledged);
    assert_eq!(laptop.peer_count().await, 0);
    assert_eq!(workstation.peer_count().await, 1, "the first stands");
    laptop.wait_for_remote(RemoteStatus::Available).await;

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn removing_the_first_leaves_a_second_pairing_standing() {
    let (workstation, laptop) = offered_directly("first-goes").await;
    workstation.accept_offer().await;
    laptop.looks_at_remote().await;
    workstation.remote_comes_to_be(LAPTOP).await;
    laptop.offers_clear().await;

    laptop.client.remove_remote(REMOTE).await.expect("remove");

    assert_eq!(workstation.peer_count().await, 0);
    assert_eq!(laptop.peer_count().await, 1, "the second stands");
    workstation
        .client
        .probe_remote(LAPTOP)
        .await
        .expect("the workstation still reaches the laptop");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn issuing_for_it_leaves_an_outstanding_invite_good_and_its_short_code_alone() {
    let (workstation, laptop) = offered_directly("outstanding").await;
    let third = TestServer::with_timings("outstanding-third", timings()).await;
    let shown = laptop
        .client
        .issue_invite(IssueInviteRequest {
            ways: vec![laptop.way()],
        })
        .await
        .expect("issue an Invite from /serve");

    workstation.accept_offer().await;
    laptop.looks_at_remote().await;
    workstation.remote_comes_to_be(LAPTOP).await;
    laptop.offers_clear().await;

    third
        .redeem_as(shown.invite, "laptop")
        .await
        .expect("the Invite /serve showed is still good");

    workstation.shutdown().await;
    laptop.shutdown().await;
    third.shutdown().await;
}

#[tokio::test]
async fn a_remote_of_that_key_already_here_ends_the_offer_rather_than_form_a_duplicate() {
    let (workstation, laptop) = offered_directly("already").await;
    let invite = laptop.invite(vec![laptop.way()]).await;
    workstation
        .redeem_as(invite, "by-hand")
        .await
        .expect("the workstation pairs by hand");

    workstation.offers_clear().await;
    laptop.looks_at_remote().await;
    laptop.offers_clear().await;
    assert_eq!(
        workstation.client.list_remotes().await.unwrap().len(),
        1,
        "the one it made itself"
    );

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_failed_redemption_says_why_there_and_is_tried_again_at_a_bounded_pace() {
    let (workstation, laptop) = offered_directly("stale-way").await;
    // The workstation accepts under a name that comes to be taken, so its
    // redemption fails on its own side, whatever the way.
    let third = TestServer::with_timings("stale-way-third", timings()).await;
    third.serve().await;
    workstation.accept_offer().await;
    let invite = third.invite(vec![third.way()]).await;
    workstation
        .redeem_as(invite, LAPTOP)
        .await
        .expect("that name comes to be taken");

    laptop.looks_at_remote().await;
    let failed = workstation
        .offers_become(|offers| {
            offers.held.first().and_then(|held| held.answer.as_ref()).is_some_and(|answer| {
                matches!(&answer.held_up, Some(OfferHeldUp::RedemptionFailed { why }) if why.contains(LAPTOP))
            })
        })
        .await;
    assert_eq!(failed.held.len(), 1, "the offer stands");
    laptop
        .offers_become(|offers| {
            offers
                .made
                .first()
                .is_some_and(|made| made.held_up == Some(OfferHeldUp::NotRedeemed))
        })
        .await;

    // Accepted again under a free name, it forms on the laptop's next look.
    let offer = workstation.held_offer().await;
    workstation
        .client
        .answer_offer(
            offer.id,
            AnswerOfferRequest::Accept {
                name: "free-name".to_owned(),
            },
        )
        .await
        .expect("accept under a free name");
    laptop.looks_at_remote().await;
    workstation.remote_comes_to_be("free-name").await;
    workstation.offers_clear().await;
    laptop.offers_clear().await;

    workstation.shutdown().await;
    laptop.shutdown().await;
    third.shutdown().await;
}

#[tokio::test]
async fn a_try_that_came_to_nothing_is_not_made_again_soon_however_a_peer_asks() {
    let (workstation, laptop) = serving_directly_by(
        "bounded",
        timings().with_pair_back_retry_interval(Duration::from_secs(3600)),
    )
    .await;
    offer(&workstation, &laptop).await;
    let third = TestServer::with_timings("bounded-third", timings()).await;
    third.serve().await;
    workstation.accept_offer().await;
    let taken = third.invite(vec![third.way()]).await;
    workstation
        .redeem_as(taken, LAPTOP)
        .await
        .expect("that name comes to be taken");
    let offer = workstation.held_offer().await;
    let mut hostile = hostile(&laptop, &workstation).await;
    let own = laptop.invite(vec![laptop.way()]).await;
    let word = || said(serde_json::json!({ "invite": { "id": offer.id, "invite": own } }));

    let (_, body) = hostile.ask(word()).await.unwrap();
    assert_eq!(answer(&body), "not_formed");
    for _ in 0..3 {
        let (_, body) = hostile.ask(word()).await.unwrap();
        assert_eq!(answer(&body), "too_soon", "no dial for this one");
    }

    workstation.shutdown().await;
    laptop.shutdown().await;
    third.shutdown().await;
}

/// A connection to `workstation`, speaking as `laptop`'s key does: a hostile
/// laptop, which says what it likes over the Pairing it really holds.
async fn hostile(laptop: &TestServer, workstation: &TestServer) -> Multiplexed {
    let stream = tokio::net::TcpStream::connect(workstation.serving_address())
        .await
        .expect("dial the Serving listener");
    let tls = paired_tls(stream, &laptop.identity(), &workstation.identity())
        .await
        .expect("run the Pairing's TLS");
    Multiplexed::over(tls).await.expect("speak HTTP/2")
}

fn said(body: serde_json::Value) -> Request<Full<axum::body::Bytes>> {
    Request::builder()
        .method(Method::POST)
        .uri("/v1/pairing/offer")
        .header("content-type", "application/json")
        .header("x-suru-protocol-version", PROTOCOL_VERSION)
        .body(Full::new(axum::body::Bytes::from(body.to_string())))
        .unwrap()
}

fn answer(said: &[u8]) -> String {
    serde_json::from_slice::<String>(said).expect("an answer in one word")
}

#[tokio::test]
async fn a_peer_past_its_ceremony_has_no_way_to_make_replace_or_remake_an_offer() {
    let (workstation, laptop) = serving_directly("no-late-offer").await;
    let invite = workstation.invite(vec![workstation.way()]).await;
    laptop.redeem_as(invite, REMOTE).await.expect("pair");
    let mut hostile = hostile(&laptop, &workstation).await;

    // Whatever it says of an id, nothing it says makes one.
    let id = uuid::Uuid::new_v4();
    for word in [
        serde_json::json!({ "withdrawn": { "id": id } }),
        serde_json::json!({ "cannot_offer": { "id": id, "why": "no_way" } }),
    ] {
        let (status, body) = hostile.ask(said(word)).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer(&body), "gone");
    }
    let (status, _) = hostile
        .ask(said(serde_json::json!({ "make": { "id": id } })))
        .await
        .unwrap();
    assert!(!status.is_success(), "no such word");
    assert_eq!(workstation.offers().await, PairBackOffers::default());

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn an_invite_is_taken_only_for_an_accepted_offer_and_only_naming_the_peers_own_key() {
    let (workstation, laptop) = offered_directly("unsolicited").await;
    let third = TestServer::with_timings("unsolicited-third", timings()).await;
    third.serve().await;
    let offer = workstation.held_offer().await;
    let mut hostile = hostile(&laptop, &workstation).await;
    let elsewhere = third.invite(vec![third.way()]).await;
    let own = laptop.invite(vec![laptop.way()]).await;

    // Unsolicited: the offer has not been accepted, even by an Invite naming
    // the Peer's own key.
    let (_, body) = hostile
        .ask(said(serde_json::json!({
            "invite": { "id": offer.id, "invite": own }
        })))
        .await
        .unwrap();
    assert_eq!(answer(&body), "not_accepted");
    assert!(workstation.client.list_remotes().await.unwrap().is_empty());

    // Accepted, an Invite to another identity is refused and forms nothing,
    // though it is a good one that other machine would honour.
    workstation.accept_offer().await;
    let (_, body) = hostile
        .ask(said(serde_json::json!({
            "invite": { "id": offer.id, "invite": elsewhere }
        })))
        .await
        .unwrap();
    assert_eq!(answer(&body), "wrong_key");
    assert!(workstation.client.list_remotes().await.unwrap().is_empty());
    assert_eq!(third.peer_count().await, 0);
    assert!(
        matches!(
            workstation.held_offer().await.answer.unwrap().held_up,
            Some(OfferHeldUp::RedemptionFailed { .. })
        ),
        "its reader is told"
    );

    workstation.shutdown().await;
    laptop.shutdown().await;
    third.shutdown().await;
}

#[tokio::test]
async fn a_peer_says_nothing_of_another_peers_offer_and_hears_nothing_of_one() {
    let (workstation, laptop) = offered_directly("others").await;
    let other = TestServer::with_timings("others-other", timings()).await;
    other.make_identity_key().await;
    let invite = workstation.invite(vec![workstation.way()]).await;
    other.redeem_as(invite, REMOTE).await.expect("pair");
    let laptops = laptop.made_offer().await.id;
    let mut hostile = hostile(&other, &workstation).await;

    for word in [
        serde_json::json!({ "withdrawn": { "id": laptops } }),
        serde_json::json!({ "cannot_offer": { "id": laptops, "why": "no_way" } }),
        serde_json::json!({ "invite": { "id": laptops, "invite": "suru-v1-x" } }),
    ] {
        let (_, body) = hostile.ask(said(word)).await.unwrap();
        assert_eq!(
            answer(&body),
            "gone",
            "that offer is no one else's to touch"
        );
    }
    assert_eq!(workstation.held_offer().await.id, laptops);
    let (status, told) = hostile
        .ask_first_line(
            Request::builder()
                .uri("/v1/pairing/offered-relays")
                .header("x-suru-protocol-version", PROTOCOL_VERSION)
                .body(Full::new(axum::body::Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&told).unwrap(),
        serde_json::json!({ "relays": [] }),
        "told nothing of the laptop's"
    );

    workstation.shutdown().await;
    laptop.shutdown().await;
    other.shutdown().await;
}

#[tokio::test]
async fn what_a_peer_asks_through_the_proxy_reaches_no_offer_route() {
    let (workstation, laptop) = offered_directly("proxy").await;
    let offer = workstation.held_offer().await;
    let api = super::pairing::RemoteApi::of(&laptop);

    // What a Client or a Sidekick on the laptop carries to the workstation is
    // the same request, and is refused as server administration.
    let accept = serde_json::json!({ "accept": { "name": "taken-over" } });
    for (path, status) in [
        (
            format!("/v1/pairing/offers/{}/answer", offer.id),
            StatusCode::FORBIDDEN,
        ),
        (
            format!("/v1/pairing/offers/{}/notice", offer.id),
            StatusCode::FORBIDDEN,
        ),
        ("/v1/pairing/remotes".to_owned(), StatusCode::FORBIDDEN),
        ("/v1/pairing/offer".to_owned(), StatusCode::FORBIDDEN),
    ] {
        let response = api.post(&path).json(&accept).send().await.unwrap();
        assert_eq!(response.status(), status, "{path}");
    }
    let listing = api.get("/v1/pairing/offers").send().await.unwrap();
    assert_eq!(listing.status(), StatusCode::FORBIDDEN);
    assert!(workstation.held_offer().await.answer.is_none());

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn the_notice_of_an_offer_is_given_once_to_one_client_across_restarts() {
    let (mut workstation, laptop) = offered_directly("notice").await;
    let offer = workstation.held_offer().await;
    let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());

    assert!(
        workstation
            .client
            .claim_offer_notice(offer.id, first)
            .await
            .unwrap()
    );
    assert!(
        workstation
            .client
            .claim_offer_notice(offer.id, first)
            .await
            .unwrap(),
        "the Client given it, not having heard, is told it was"
    );
    assert!(
        !workstation
            .client
            .claim_offer_notice(offer.id, second)
            .await
            .unwrap()
    );
    assert!(!workstation.held_offer().await.notice);

    workstation.restart().await;
    assert!(
        !workstation.held_offer().await.notice,
        "asked of no Client again"
    );
    assert!(
        !workstation
            .client
            .claim_offer_notice(offer.id, second)
            .await
            .unwrap()
    );

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_client_is_told_of_offers_as_it_connects_and_as_they_change() {
    let (mut workstation, laptop) = serving_directly("pushed").await;
    let invite = workstation.invite(vec![workstation.way()]).await;
    laptop.redeem_offering(invite).await.expect("pair");

    let told = timeout(PROGRESS_DEADLINE, async {
        loop {
            match workstation.client.next().await {
                Some(ManagedEvent::PairBack(offers)) if !offers.held.is_empty() => return offers,
                Some(_) => {}
                None => panic!("the Client stopped hearing from its Server"),
            }
        }
    })
    .await
    .expect("told of the offer");
    assert_eq!(told.held[0].offered_name, super::machine_hostname());

    // A Client attaching later is told as it connects, and told again of what
    // changes.
    workstation.restart().await;
    let descriptor = workstation.server.as_ref().unwrap().descriptor().clone();
    let mut events = crate::support::loopback_client()
        .get(format!("{}/v1/events", descriptor.base_url))
        .bearer_auth(&descriptor.token)
        .send()
        .await
        .expect("open the event stream")
        .bytes_stream()
        .eventsource();
    let told_again = timeout(PROGRESS_DEADLINE, async {
        loop {
            let event = events
                .next()
                .await
                .expect("the stream stays open")
                .expect("an event");
            if event.event == suru::protocol::PAIR_BACK_EVENT {
                return serde_json::from_str::<PairBackOffers>(&event.data).unwrap();
            }
        }
    })
    .await
    .expect("told as it connects");
    assert_eq!(told_again.held.len(), 1);
    drop(events);
    workstation.accept_offer().await;
    let told = timeout(PROGRESS_DEADLINE, async {
        loop {
            match workstation.client.next().await {
                Some(ManagedEvent::PairBack(offers)) => return offers,
                Some(_) => {}
                None => panic!("the Client stopped hearing from its Server"),
            }
        }
    })
    .await
    .expect("told of the acceptance");
    assert_eq!(told.held[0].answer.as_ref().unwrap().name, LAPTOP);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn an_offer_waits_through_a_version_mismatch_and_strands_no_pairing() {
    let (workstation, mut laptop) = offered_directly("mismatch").await;
    workstation.accept_offer().await;

    // The laptop comes to speak another version of the Pairing than the
    // workstation. Everything over the Pairing is refused, as ever, the offer
    // among it: it is neither asked of nor ended by that.
    let speaking = laptop.timings.pairing_protocol_version;
    laptop.timings.pairing_protocol_version = speaking - 1;
    laptop.restart().await;
    laptop.wait_for_remote(RemoteStatus::ProtocolMismatch).await;
    assert_eq!(laptop.made_offer().await.held_up, None);
    assert_eq!(laptop.peer_count().await, 0, "nothing was issued");
    assert!(workstation.remote_named(LAPTOP).await.is_none());
    assert!(workstation.held_offer().await.answer.is_some());

    // Speaking one version again, it goes on as if nothing had happened.
    laptop.timings.pairing_protocol_version = speaking;
    laptop.restart().await;
    laptop.wait_for_remote(RemoteStatus::Available).await;
    workstation.remote_comes_to_be(LAPTOP).await;
    laptop.offers_clear().await;

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_server_of_another_version_is_refused_the_pairing_with_no_offer_asked_of_it() {
    let (older, laptop) = serving_directly_by(
        "older",
        ServerTimings {
            pairing_protocol_version: PROTOCOL_VERSION - 1,
            ..timings()
        },
    )
    .await;
    let laptop_timings = ServerTimings {
        pairing_protocol_version: PROTOCOL_VERSION,
        ..timings()
    };
    let current = TestServer::with_timings("older-current", laptop_timings).await;
    current.serve().await;

    let refused = current
        .redeem_offering(older.invite(vec![older.way()]).await)
        .await
        .expect_err("another version");
    assert_eq!(
        error_code(&refused),
        SessionErrorCode::PairingProtocolMismatch
    );
    assert_eq!(older.offers().await, PairBackOffers::default());
    assert_eq!(current.offers().await, PairBackOffers::default());

    older.shutdown().await;
    laptop.shutdown().await;
    current.shutdown().await;
}

#[tokio::test]
async fn a_machine_that_already_reaches_this_one_is_offered_nothing() {
    let (workstation, laptop) = serving_directly("reached-already").await;
    let invite = laptop.invite(vec![laptop.way()]).await;
    workstation
        .redeem_as(invite, LAPTOP)
        .await
        .expect("the workstation pairs with the laptop");

    let refused = laptop
        .redeem_offering(workstation.invite(vec![workstation.way()]).await)
        .await
        .expect_err("nothing to offer");
    assert_eq!(error_code(&refused), SessionErrorCode::PairBackNotOfferable);
    assert_eq!(workstation.offers().await, PairBackOffers::default());

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn pairing_again_replaces_what_the_last_ceremony_offered_on_both() {
    let (workstation, laptop) = offered_directly("again").await;
    let first = workstation.held_offer().await.id;

    // Another ceremony, by another of the workstation's user's Invites.
    let invite = workstation.invite(vec![workstation.way()]).await;
    laptop
        .client
        .redeem_invite(RedeemInviteRequest {
            invite: invite.into(),
            name: Some("second-name".to_owned()),
            ways: Vec::new(),
            pair_back: true,
        })
        .await
        .expect("pair again");
    let second = workstation.held_offer().await.id;
    assert_ne!(first, second);
    assert_eq!(laptop.made_offer().await.id, second);
    assert_eq!(laptop.made_offer().await.remote, "second-name");

    // And one with the option off leaves none on either.
    let invite = workstation.invite(vec![workstation.way()]).await;
    laptop
        .redeem_as(invite, "third-name")
        .await
        .expect("pair once more");
    assert_eq!(workstation.offers().await, PairBackOffers::default());
    assert_eq!(laptop.offers().await, PairBackOffers::default());

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn no_record_holds_an_invite_once_it_has_formed_a_pairing() {
    let (workstation, laptop) = offered_directly("logs").await;
    workstation.accept_offer().await;
    laptop.looks_at_remote().await;
    workstation.remote_comes_to_be(LAPTOP).await;
    laptop.offers_clear().await;
    for server in [&workstation, &laptop] {
        assert!(
            !records_of(server).contains("suru-v1-"),
            "no Invite is written where a Server keeps its records"
        );
    }

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_withdrawal_with_the_other_connected_reaches_it_at_once() {
    let (workstation, laptop) = offered_directly("withdraw-connected").await;
    let offer = laptop.made_offer().await;
    let mut held_open = laptop
        .client
        .outlook(Outlook::Remote(REMOTE.to_owned()))
        .subscribe_catalog();
    assert!(matches!(
        next_catalog_event(&mut held_open).await,
        Some(ManagedEvent::SessionCatalogReconciled(_))
    ));

    laptop
        .client
        .withdraw_offer(offer.id)
        .await
        .expect("withdraw");

    workstation.offers_clear().await;
    laptop.offers_clear().await;
    drop(held_open);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_delivery_whose_answer_was_lost_is_settled_by_the_next_telling_with_one_pairing() {
    let (workstation, laptop) = offered_directly("answer-lost").await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = laptop.invite(vec![laptop.way()]).await;

    // The workstation is sent what the laptop's Server would send, and redeems
    // it, while the laptop's Server has no idea: as if the answer never came.
    let mut hostile = hostile(&laptop, &workstation).await;
    let (_, body) = hostile
        .ask(said(serde_json::json!({
            "invite": { "id": offer.id, "invite": invite }
        })))
        .await
        .unwrap();
    assert_eq!(answer(&body), "formed");
    assert_eq!(laptop.made_offer().await.id, offer.id, "it has yet to hear");

    // Told on its next connection that no offer stands, it clears its record
    // and issues nothing, and a second delivery finds nothing to redeem for.
    laptop.looks_at_remote().await;
    laptop.offers_clear().await;
    assert_eq!(workstation.client.list_remotes().await.unwrap().len(), 1);
    assert_eq!(laptop.peer_count().await, 1);
    let again = laptop.invite(vec![laptop.way()]).await;
    let (_, body) = hostile
        .ask(said(serde_json::json!({
            "invite": { "id": offer.id, "invite": again }
        })))
        .await
        .unwrap();
    assert_eq!(answer(&body), "gone");
    assert_eq!(workstation.client.list_remotes().await.unwrap().len(), 1);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_relay_the_accepting_machine_holds_no_login_at_is_said_and_it_forms_once_it_does() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = serving_directly("no-login").await;
    // The laptop is reached through the Relay alone, where only it is logged in.
    laptop.log_in(&relay, "583231", "octocat").await;
    laptop.serve_through(&relay, true).await;
    laptop.listen(false).await.expect("close the listener");
    laptop.relay_is_offered(&relay).await;
    offer(&workstation, &laptop).await;
    workstation.accept_offer().await;
    let mut held_open = laptop
        .client
        .outlook(Outlook::Remote(REMOTE.to_owned()))
        .subscribe_catalog();
    assert!(matches!(
        next_catalog_event(&mut held_open).await,
        Some(ManagedEvent::SessionCatalogReconciled(_))
    ));

    let failed = workstation
        .offers_become(|offers| {
            offers
                .held
                .first()
                .and_then(|held| held.answer.as_ref())
                .is_some_and(|answer| {
                    matches!(answer.held_up, Some(OfferHeldUp::RedemptionFailed { .. }))
                })
        })
        .await;
    assert_eq!(failed.held.len(), 1, "the offer stands: {failed:?}");
    assert!(workstation.remote_named(LAPTOP).await.is_none());

    // Logging in there takes nothing further: it is tried again while the two
    // are connected.
    workstation.log_in(&relay, "583231", "octocat").await;
    workstation.remote_comes_to_be(LAPTOP).await;
    workstation.offers_clear().await;
    laptop.offers_clear().await;
    drop(held_open);

    workstation.shutdown().await;
    laptop.shutdown().await;
}
