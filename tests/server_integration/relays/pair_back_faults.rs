//! Pairing back where something goes wrong in the middle: a redemption held
//! midway by a route the test controls, a peer that says what no real Server
//! would, and what reaches a Log throughout (ADR-0066).

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{StreamExt as _, stream};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use rcgen::{KeyPair, PublicKeyData};
use suru::{
    protocol::{
        AnswerOfferRequest, OfferHeldUp, PROTOCOL_VERSION, RedemptionFailure, RemoteStatus,
        SessionErrorCode, Way,
    },
    server::ServerTimings,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    time::timeout,
};

use super::{
    Captured, TestServer, error_code,
    pair_back::{
        LAPTOP, answer, hostile, offer, offered_directly, said, serving_directly,
        serving_directly_by, timings,
    },
    pairing::RemoteApi,
};
use crate::support::{PROGRESS_DEADLINE, observed_tcp_proxy::ObservedTcpProxy};

/// What is done with what the held connection's server says.
#[derive(Default)]
struct Hold {
    release: Notify,
    cut: AtomicBool,
    released: AtomicBool,
}

/// A route to a Serving listener carrying everything it is given, except that on
/// the connection numbered `held`, from 1, it stops carrying what the listener
/// says from the moment the dialler's second flight comes: so what that dialler
/// asked is done there, and its answer waits on the test. A redemption dials one
/// connection to prepare and another to commit.
struct HoldingRoute {
    address: SocketAddr,
    hold: Arc<Hold>,
    holding: Arc<AtomicBool>,
}

impl HoldingRoute {
    async fn to(target: SocketAddr, held: usize) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let hold = Arc::new(Hold::default());
        let holding = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));
        let route = Self {
            address,
            hold: hold.clone(),
            holding: holding.clone(),
        };
        tokio::spawn(async move {
            while let Ok((dialler, _)) = listener.accept().await {
                let number = connections.fetch_add(1, Ordering::SeqCst) + 1;
                let hold = hold.clone();
                let holding = holding.clone();
                tokio::spawn(async move {
                    carry(dialler, target, number == held, hold, holding).await;
                });
            }
        });
        route
    }

    /// Waits until a dialler has come to be held.
    async fn holds_someone(&self) {
        timeout(PROGRESS_DEADLINE, async {
            while !self.holding.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("a dialler comes to be held");
    }

    /// Lets what was held back go on.
    fn release(&self) {
        self.hold.released.store(true, Ordering::SeqCst);
        self.hold.release.notify_waiters();
    }

    /// Drops the held connection, what was held back going nowhere.
    fn cut(&self) {
        self.hold.cut.store(true, Ordering::SeqCst);
        self.hold.release.notify_waiters();
    }
}

async fn carry(
    dialler: TcpStream,
    target: SocketAddr,
    held: bool,
    hold: Arc<Hold>,
    holding: Arc<AtomicBool>,
) {
    let Ok(server) = TcpStream::connect(target).await else {
        return;
    };
    let (mut from_dialler, mut to_dialler) = dialler.into_split();
    let (mut from_server, mut to_server) = server.into_split();
    let holds_now = Arc::new(AtomicBool::new(false));
    let noting = holds_now.clone();
    let asking = tokio::spawn(async move {
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut flights = 0;
        loop {
            match from_dialler.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    flights += 1;
                    if held && flights == 2 {
                        noting.store(true, Ordering::SeqCst);
                        holding.store(true, Ordering::SeqCst);
                    }
                    if to_server.write_all(&buffer[..read]).await.is_err() {
                        return;
                    }
                }
            }
        }
    });
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = match from_server.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        while holds_now.load(Ordering::SeqCst) && !hold.released.load(Ordering::SeqCst) {
            if hold.cut.load(Ordering::SeqCst) {
                asking.abort();
                return;
            }
            let released = hold.release.notified();
            tokio::select! {
                () = released => {}
                () = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }
        if to_dialler.write_all(&buffer[..read]).await.is_err() {
            break;
        }
    }
    asking.abort();
}

/// An address nothing listens at.
async fn nothing_at() -> SocketAddr {
    TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap()
        .local_addr()
        .unwrap()
}

/// An Invite naming `key`, offering `ways`, by the format every Invite has.
fn invite_offering(key: &KeyPair, ways: serde_json::Value) -> String {
    let payload = serde_json::json!({
        "w": ways,
        "k": URL_SAFE_NO_PAD.encode(key.subject_public_key_info()),
        "t": URL_SAFE_NO_PAD.encode([9_u8; 32]),
        "h": "stand-in",
    });
    format!(
        "suru-v1-{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    )
}

/// Runs `app` over the Pairing's TLS holding `identity`, speaking HTTP/1.1 as a
/// direct way's connection does, and answers where it is reached.
async fn standing_in(identity: &KeyPair, app: Router) -> SocketAddr {
    let certificate = rcgen::CertificateParams::new(vec!["suru-server".to_owned()])
        .unwrap()
        .self_signed(identity)
        .unwrap();
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.der().clone()],
        rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
            identity.serialize_der(),
        )),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let app = app.clone();
            tokio::spawn(async move {
                if let Ok(stream) = acceptor.accept(stream).await {
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), TowerToHyperService::new(app))
                        .await;
                }
            });
        }
    });
    address
}

/// The accepting machine's offer, with `laptop`'s answer to `word` said to it
/// by a task of its own, for a test to wait on while it holds things up.
async fn saying(
    laptop: &TestServer,
    workstation: &TestServer,
    invite: String,
    id: uuid::Uuid,
) -> tokio::task::JoinHandle<String> {
    let mut hostile = hostile(laptop, workstation).await;
    tokio::spawn(async move {
        let (_, body) = hostile
            .ask(said(
                serde_json::json!({ "invite": { "id": id, "invite": invite } }),
            ))
            .await
            .unwrap();
        answer(&body)
    })
}

/// Why the one offer `server` holds has not formed, as it tells its Clients.
async fn held_up(server: &TestServer) -> Option<OfferHeldUp> {
    server.held_offer().await.answer?.held_up
}

/// Waits until the offer `server` holds says it failed as `why`.
async fn failed_as(server: &TestServer, why: RedemptionFailure) {
    let wanted = OfferHeldUp::RedemptionFailed { why };
    server
        .offers_become(|offers| {
            offers
                .held
                .first()
                .and_then(|held| held.answer.as_ref())
                .is_some_and(|answer| answer.held_up.as_ref() == Some(&wanted))
        })
        .await;
}

/// What no Log may hold, in what a Server wrote: any Invite, and `token`.
fn assert_holds_no_token(log: &[u8], token: &str) {
    let log = String::from_utf8_lossy(log);
    assert!(!log.is_empty(), "both Servers logged something");
    assert!(!log.contains("suru-v1-"), "a Log holds an Invite: {log}");
    assert!(!log.contains(token), "a Log holds an Invite's token: {log}");
}

/// A Serving Server standing in for the real one with its key, which has
/// accepted the offer it is told of, and keeps every word said of it.
#[derive(Clone)]
struct AcceptingStandIn {
    id: uuid::Uuid,
    words: Arc<Mutex<Vec<String>>>,
    protocol_version: u32,
}

async fn stand_in_health(State(stand_in): State<AcceptingStandIn>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "protocol_version": stand_in.protocol_version }))
}

async fn stand_in_tells(State(stand_in): State<AcceptingStandIn>) -> Body {
    let told = serde_json::json!({
        "relays": [],
        "offer": { "id": stand_in.id, "accepted": true },
    });
    Body::from_stream(
        stream::once(async move {
            Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(format!("{told}\n")))
        })
        .chain(stream::pending()),
    )
}

async fn stand_in_hears(
    State(stand_in): State<AcceptingStandIn>,
    body: String,
) -> Json<&'static str> {
    stand_in.words.lock().unwrap().push(body);
    Json("formed")
}

/// However a real pair go through it, and whatever a token is issued for it, no
/// Log on either side holds an Invite or a token: the bound token itself, read
/// off what the offering Server sent a stand-in for the other.
#[tokio::test]
async fn no_invite_or_token_of_an_offer_reaches_a_log() {
    let captured = Captured::default();
    let writer = captured.clone();
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish(),
    );

    // The token, as the offering Server sends it.
    let (workstation, laptop) = serving_directly("records-token").await;
    let route = ObservedTcpProxy::start(workstation.serving_address()).await;
    let invite = workstation.invite(vec![Way::Direct(route.address)]).await;
    laptop
        .redeem_offering(invite)
        .await
        .expect("pair, offering to pair back");
    let stand_in = AcceptingStandIn {
        id: laptop.made_offer().await.id,
        words: Arc::default(),
        protocol_version: PROTOCOL_VERSION,
    };
    let at = standing_in(
        &workstation.identity(),
        Router::new()
            .route("/health", get(stand_in_health))
            .route("/v1/pairing/offered-relays", get(stand_in_tells))
            .route("/v1/pairing/offer", post(stand_in_hears))
            .with_state(stand_in.clone()),
    )
    .await;
    route.retarget(at);
    laptop.looks_at_remote().await;
    laptop.offers_clear().await;
    let word = stand_in.words.lock().unwrap().remove(0);
    let word: serde_json::Value = serde_json::from_str(&word).unwrap();
    let sent = word["invite"]["invite"].as_str().unwrap().to_owned();
    let payload: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(sent.strip_prefix("suru-v1-").unwrap())
            .unwrap(),
    )
    .unwrap();
    let token = payload["t"].as_str().unwrap().to_owned();
    workstation.shutdown().await;
    laptop.shutdown().await;

    // And a pair going all the way.
    let (workstation, laptop) = offered_directly("records-whole").await;
    workstation.accept_offer().await;
    laptop.looks_at_remote().await;
    workstation.remote_comes_to_be(LAPTOP).await;
    laptop.offers_clear().await;
    workstation.shutdown().await;
    laptop.shutdown().await;

    assert_holds_no_token(&captured.0.lock().unwrap(), &token);
}

/// What a stand-in for the offering machine says as it refuses an enrollment.
async fn refuses_saying(code: &'static str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "code": code,
            "message": "Visit https://evil.example to finish pairing",
        })),
    )
}

/// Whatever the machine that made an offer says as it refuses, its reader's
/// screen shows the accepting Server's own words: it chooses among them at most.
#[tokio::test]
async fn what_the_offering_machine_says_as_it_refuses_reaches_no_reader() {
    let (workstation, laptop) = offered_directly("refusal-text").await;
    let at = standing_in(
        &laptop.identity(),
        Router::new().route(
            "/v1/pairing/enroll",
            post(|| refuses_saying("invalid_invite")),
        ),
    )
    .await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = invite_offering(
        &laptop.identity(),
        serde_json::json!([{ "direct": at.to_string() }]),
    );

    let told = saying(&laptop, &workstation, invite, offer.id)
        .await
        .await
        .unwrap();

    assert_eq!(told, "not_formed");
    assert!(matches!(
        held_up(&workstation).await,
        Some(OfferHeldUp::RedemptionFailed { .. })
    ));
    let listing = serde_json::to_string(&workstation.offers().await).unwrap();
    assert!(!listing.contains("evil.example"), "{listing}");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// The address of a Relay it offers is its word too: a reader told to log in
/// there is not shown it.
#[tokio::test]
async fn a_relay_named_only_by_the_offering_machine_reaches_no_reader() {
    let (workstation, laptop) = offered_directly("relay-text").await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = invite_offering(
        &laptop.identity(),
        serde_json::json!([{ "relay": "https://evil.example" }]),
    );

    let told = saying(&laptop, &workstation, invite, offer.id)
        .await
        .await
        .unwrap();

    assert_eq!(told, "not_formed");
    let listing = serde_json::to_string(&workstation.offers().await).unwrap();
    assert!(!listing.contains("evil.example"), "{listing}");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A reader who has accepted cannot rename what is being redeemed: the Remote
/// that comes of it goes by the name it began under, and a crash could not find
/// it to take it back by any other.
#[tokio::test]
async fn an_acceptance_cannot_change_while_it_is_being_redeemed() {
    let (workstation, laptop) = offered_directly("re-accept").await;
    let route = HoldingRoute::to(laptop.serving_address(), 1).await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = laptop.invite(vec![Way::Direct(route.address)]).await;
    let told = saying(&laptop, &workstation, invite, offer.id).await;
    route.holds_someone().await;

    let refused = workstation
        .client
        .answer_offer(
            offer.id,
            AnswerOfferRequest::Accept {
                name: "another-name".to_owned(),
            },
        )
        .await
        .expect_err("it is being redeemed");
    assert_eq!(error_code(&refused), SessionErrorCode::OfferBeingRedeemed);

    route.release();
    assert_eq!(told.await.unwrap(), "formed");
    assert!(workstation.remote_named(LAPTOP).await.is_some());
    assert!(workstation.remote_named("another-name").await.is_none());

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A Pairing made by hand while an offer's redemption is under way stands: what
/// the redemption leaves goes here alone, since telling the other machine to
/// drop its one Peer for this key would end the Pairing made by hand as well.
#[tokio::test]
async fn pairing_by_hand_during_a_redemption_is_not_ended_by_it() {
    let (workstation, laptop) = offered_directly("by-hand").await;
    let route = HoldingRoute::to(laptop.serving_address(), 2).await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = laptop.invite(vec![Way::Direct(route.address)]).await;
    let told = saying(&laptop, &workstation, invite, offer.id).await;
    // Its commit has landed there; its answer is held.
    route.holds_someone().await;
    timeout(PROGRESS_DEADLINE, async {
        while laptop.peer_count().await == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the commit lands");

    let by_hand = laptop.invite(vec![laptop.way()]).await;
    workstation
        .redeem_as(by_hand, "studio")
        .await
        .expect("pair by hand");
    workstation.offers_clear().await;

    route.release();
    assert_eq!(told.await.unwrap(), "gone");

    assert!(workstation.remote_named(LAPTOP).await.is_none());
    assert_eq!(laptop.peer_count().await, 1, "the Peer for this key stands");
    let health = workstation
        .client
        .probe_remote("studio")
        .await
        .expect("probe the Remote made by hand");
    assert_eq!(health.status, RemoteStatus::Available);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// However many ways an Invite says it offers, no more are dialled than one
/// Server could really offer, so a hostile Peer cannot have this one dial
/// thousands of addresses in turn.
#[tokio::test]
async fn an_invite_offering_more_ways_than_a_server_can_is_refused_undialled() {
    let (workstation, laptop) = offered_directly("many-ways").await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let ways = (0..40)
        .map(|index| serde_json::json!({ "direct": format!("127.0.0.{}:{port}", index + 2) }))
        .collect::<Vec<_>>();
    let invite = invite_offering(&laptop.identity(), serde_json::Value::Array(ways));

    let told = saying(&laptop, &workstation, invite, offer.id)
        .await
        .await
        .unwrap();

    assert_eq!(told, "not_formed");
    failed_as(&workstation, RedemptionFailure::TooManyWays).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err(),
        "nothing was dialled"
    );

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A listener on a loopback address reaches nothing on another machine, so as
/// `/serve` does not offer it, neither does an offer: it reads as having no way,
/// and no Invite goes out only to fail.
#[tokio::test]
async fn a_listener_on_loopback_is_no_way_to_offer() {
    let timing = || ServerTimings {
        pair_back_offers_loopback: false,
        ..timings()
    };
    let (workstation, laptop) = serving_directly_by("loopback", timing()).await;
    let invite = workstation.invite(vec![workstation.way()]).await;
    let refused = laptop
        .redeem_offering(invite)
        .await
        .expect_err("it has no way to offer");
    assert_eq!(error_code(&refused), SessionErrorCode::PairBackNotOfferable);
    workstation.shutdown().await;
    laptop.shutdown().await;

    // One made while it had a way, and its listener since on loopback.
    let (workstation, mut laptop) = offered_directly("loopback-later").await;
    workstation.accept_offer().await;
    laptop.timings.pair_back_offers_loopback = false;
    laptop.restart().await;
    laptop.looks_at_remote().await;
    workstation
        .offers_become(|offers| {
            offers
                .held
                .first()
                .and_then(|held| held.answer.as_ref())
                .is_some_and(|answer| answer.held_up == Some(OfferHeldUp::NoWay))
        })
        .await;
    assert_eq!(laptop.peer_count().await, 0, "nothing was issued or tried");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A Peer flipping why it cannot meet an offer costs a reader's Clients one
/// change for each interval, however fast it says so.
#[tokio::test]
async fn why_an_offer_cannot_be_met_changes_no_faster_than_the_retry_bound() {
    let (workstation, laptop) = serving_directly_by(
        "reason-flood",
        timings().with_pair_back_retry_interval(Duration::from_secs(3600)),
    )
    .await;
    offer(&workstation, &laptop).await;
    workstation.accept_offer().await;
    let id = workstation.held_offer().await.id;
    let mut hostile = hostile(&laptop, &workstation).await;
    let word = |why: &str| said(serde_json::json!({ "cannot_offer": { "id": id, "why": why } }));

    let (_, body) = hostile.ask(word("serving_off")).await.unwrap();
    assert_eq!(answer(&body), "noted");
    for why in ["no_way", "serving_off", "no_way"] {
        let (_, body) = hostile.ask(word(why)).await.unwrap();
        assert_eq!(answer(&body), "too_soon");
    }
    assert_eq!(held_up(&workstation).await, Some(OfferHeldUp::ServingOff));

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// Each spelling of a refused route a Peer might try is refused as what it is,
/// by the guard itself rather than by the local router not knowing it.
#[tokio::test]
async fn a_peer_spelling_an_administrative_route_otherwise_is_refused_as_one() {
    let (workstation, laptop) = offered_directly("spellings").await;
    let api = RemoteApi::of(&laptop);
    for path in [
        "/v1/%70airing/offers",
        "/v1/pa%69ring/remotes",
        "/V1/pairing/remotes",
        "/v1/PAIRING/offers",
        "/v1//pairing/offers",
        "//v1/pairing/peers",
        "/%2576%31/pairing/peers",
        "/v1%2Fpairing%2Fpeers",
        "/V1/RELAYS",
        "/v1/%72elays/x",
        "/v1/EVENTS",
        "/v1/%6Fnboarded",
    ] {
        let response = api.get(path).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    for path in ["/V1/settings", "/v1/%73ettings", "/v1/server//stop"] {
        let response = api.post(path).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    // And what a Peer may ask is asked as ever.
    let response = api.get("/v1/workspaces").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A machine that takes the handshake and falls silent holds a redemption no
/// longer than a token lives, at either step, and leaves nothing of it.
#[tokio::test]
async fn a_redemption_given_up_on_a_silent_machine_leaves_nothing_and_says_so() {
    for (step, channel) in [(1, "silent-prepare"), (2, "silent-commit")] {
        let (workstation, laptop) = serving_directly_by(
            channel,
            timings().with_pair_back_invite_ttl(Duration::from_millis(400)),
        )
        .await;
        offer(&workstation, &laptop).await;
        let route = HoldingRoute::to(laptop.serving_address(), step).await;
        workstation.accept_offer().await;
        let offer = workstation.held_offer().await;
        let invite = laptop.invite(vec![Way::Direct(route.address)]).await;

        let told = saying(&laptop, &workstation, invite, offer.id)
            .await
            .await
            .unwrap();

        assert_eq!(told, "not_formed");
        failed_as(&workstation, RedemptionFailure::TimedOut).await;
        assert!(
            workstation.client.list_remotes().await.unwrap().is_empty(),
            "what it left is gone"
        );
        route.cut();

        workstation.shutdown().await;
        laptop.shutdown().await;
    }
}

/// A commit that landed with its answer lost: the accepting machine takes its
/// Remote back, and the next Invite forms the one Pairing, the other machine
/// enrolling it again over the Peer it already holds.
#[tokio::test]
async fn a_commit_that_landed_with_its_answer_lost_comes_to_one_pairing() {
    let (workstation, laptop) = offered_directly("commit-lost").await;
    let route = HoldingRoute::to(laptop.serving_address(), 2).await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = laptop.invite(vec![Way::Direct(route.address)]).await;
    let told = saying(&laptop, &workstation, invite, offer.id).await;
    route.holds_someone().await;
    timeout(PROGRESS_DEADLINE, async {
        while laptop.peer_count().await == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the commit lands");

    route.cut();

    assert_eq!(told.await.unwrap(), "not_formed");
    assert!(workstation.client.list_remotes().await.unwrap().is_empty());
    assert_eq!(
        laptop.peer_count().await,
        1,
        "it holds the Peer all the same"
    );
    assert!(workstation.held_offer().await.answer.is_some());

    // Its next connection puts that right, with no step from anyone.
    timeout(PROGRESS_DEADLINE, async {
        while workstation.remote_named(LAPTOP).await.is_none() {
            laptop.looks_at_remote().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the next try forms it");
    laptop.offers_clear().await;
    assert_eq!(workstation.client.list_remotes().await.unwrap().len(), 1);
    assert_eq!(laptop.peer_count().await, 1, "one Peer, enrolled again");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A withdrawal coming once the commit has landed loses the race: the second
/// Pairing stands as an ordinary one, and what the offering side held is swept.
#[tokio::test]
async fn a_withdrawal_losing_the_race_to_a_landed_commit_leaves_an_ordinary_pairing() {
    let (workstation, laptop) = offered_directly("withdraw-race").await;
    let route = HoldingRoute::to(laptop.serving_address(), 2).await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = laptop.invite(vec![Way::Direct(route.address)]).await;
    let told = saying(&laptop, &workstation, invite, offer.id).await;
    route.holds_someone().await;
    timeout(PROGRESS_DEADLINE, async {
        while laptop.peer_count().await == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the commit lands");

    laptop
        .client
        .withdraw_offer(offer.id)
        .await
        .expect("withdraw");
    route.release();

    assert_eq!(told.await.unwrap(), "formed");
    workstation.remote_comes_to_be(LAPTOP).await;
    laptop.looks_at_remote().await;
    laptop.offers_clear().await;
    assert_eq!(laptop.peer_count().await, 1, "listed in /serve, to remove");

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A way that has gone stale says so in the accepting Server's own words, and
/// the next try by a way that answers forms the Pairing.
#[tokio::test]
async fn a_stale_address_is_said_and_the_next_try_forms_the_pairing() {
    let (workstation, laptop) = offered_directly("stale").await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let stale = nothing_at().await;
    let invite = laptop.invite(vec![Way::Direct(stale)]).await;

    let told = saying(&laptop, &workstation, invite, offer.id)
        .await
        .await
        .unwrap();

    assert_eq!(told, "not_formed");
    failed_as(&workstation, RedemptionFailure::NotReached).await;
    timeout(PROGRESS_DEADLINE, async {
        while workstation.remote_named(LAPTOP).await.is_none() {
            laptop.looks_at_remote().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the next try forms it");
    assert_eq!(laptop.peer_count().await, 1);

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A refusal's code is the other machine's word as much as its message is, so it
/// sends no reader to `/relay` or round a rename: those are said only of what
/// this Server judged for itself.
#[tokio::test]
async fn a_code_the_offering_machine_refuses_with_picks_no_sentence() {
    let (workstation, laptop) = offered_directly("refusal-code").await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    for code in [
        "relay_login_needed",
        "relay_different_accounts",
        "relay_cap_reached",
        "remote_name_conflict",
        "invalid_remote_name",
        "pairing_authentication_failed",
        "pairing_connection_failed",
        "invalid_invite",
    ] {
        let at = standing_in(
            &laptop.identity(),
            Router::new().route("/v1/pairing/enroll", post(move || refuses_saying(code))),
        )
        .await;
        let invite = invite_offering(
            &laptop.identity(),
            serde_json::json!([{ "direct": at.to_string() }]),
        );
        timeout(PROGRESS_DEADLINE, async {
            while saying(&laptop, &workstation, invite.clone(), offer.id)
                .await
                .await
                .unwrap()
                == "too_soon"
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("it is tried");
        assert_eq!(
            held_up(&workstation).await,
            Some(OfferHeldUp::RedemptionFailed {
                why: RedemptionFailure::Refused
            }),
            "{code}"
        );
    }

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// Puts a directory where `server` keeps its Peers' records, so they cannot be
/// written, answering what puts them back as they were.
fn peers_cannot_be_stored(server: &TestServer) -> impl FnOnce() {
    let records = server.config.data_dir().join("peers.json");
    let aside = server.config.data_dir().join("peers.json.aside");
    std::fs::rename(&records, &aside).expect("set the records aside");
    std::fs::create_dir(&records).expect("put a directory in their place");
    move || {
        std::fs::remove_dir(&records).expect("take the directory away");
        std::fs::rename(&aside, &records).expect("put the records back");
    }
}

/// A failed redemption whose standing cannot be stored leaves the offer accepted
/// all the same: nothing later is told one is under way, and the next Invite
/// forms the Pairing, storing what could not be.
#[tokio::test]
async fn a_failed_redemption_that_cannot_be_stored_does_not_stick() {
    let (mut workstation, laptop) = offered_directly("stuck-failed").await;
    let route = HoldingRoute::to(laptop.serving_address(), 1).await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = laptop.invite(vec![Way::Direct(route.address)]).await;
    let told = saying(&laptop, &workstation, invite, offer.id).await;
    route.holds_someone().await;
    let put_back = peers_cannot_be_stored(&workstation);

    route.cut();

    assert_eq!(told.await.unwrap(), "not_formed");
    assert_eq!(
        held_up(&workstation).await,
        Some(OfferHeldUp::RedemptionFailed {
            why: RedemptionFailure::NotReached
        })
    );
    put_back();

    // What is stored still says a redemption is under way, and nothing here
    // believes it.
    let another = laptop.invite(vec![Way::Direct(nothing_at().await)]).await;
    timeout(PROGRESS_DEADLINE, async {
        loop {
            let told = saying(&laptop, &workstation, another.clone(), offer.id)
                .await
                .await
                .unwrap();
            assert_ne!(told, "busy", "no redemption is under way");
            if told != "too_soon" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("it is tried");
    timeout(PROGRESS_DEADLINE, async {
        while workstation.remote_named(LAPTOP).await.is_none() {
            laptop.looks_at_remote().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the next try forms it");
    laptop.offers_clear().await;

    // And what came of it was stored, for a Server starting again.
    workstation.restart().await;
    assert!(workstation.offers().await.held.is_empty());
    assert!(workstation.remote_named(LAPTOP).await.is_some());

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A redemption that forms its Pairing with the offer's end unstored has formed
/// it, and says so: the offer goes here whether or not that could be written.
#[tokio::test]
async fn a_formed_pairing_whose_offer_cannot_be_cleared_from_store_does_not_stick() {
    let (workstation, laptop) = offered_directly("stuck-formed").await;
    let route = HoldingRoute::to(laptop.serving_address(), 2).await;
    workstation.accept_offer().await;
    let offer = workstation.held_offer().await;
    let invite = laptop.invite(vec![Way::Direct(route.address)]).await;
    let told = saying(&laptop, &workstation, invite.clone(), offer.id).await;
    route.holds_someone().await;
    let put_back = peers_cannot_be_stored(&workstation);

    route.release();

    assert_eq!(told.await.unwrap(), "formed");
    put_back();
    assert!(workstation.offers().await.held.is_empty());
    assert!(workstation.remote_named(LAPTOP).await.is_some());
    assert_eq!(
        saying(&laptop, &workstation, invite, offer.id)
            .await
            .await
            .unwrap(),
        "gone"
    );

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// One look at the Remote inside the bound on tries is owed a try once the bound
/// has passed, with nothing more asked of anyone: it is the connection the
/// acceptance was waiting on, though what looked has let the Remote go by then.
#[tokio::test]
async fn a_look_inside_the_bound_on_tries_is_owed_a_try_when_it_passes() {
    let (workstation, laptop) = serving_directly_by(
        "look-in-bound",
        timings().with_pair_back_retry_interval(Duration::from_millis(250)),
    )
    .await;
    offer(&workstation, &laptop).await;
    let third = TestServer::with_timings("look-in-bound-third", timings()).await;
    third.serve().await;
    workstation.accept_offer().await;
    let taken = third.invite(vec![third.way()]).await;
    workstation
        .redeem_as(taken, LAPTOP)
        .await
        .expect("that name comes to be taken");
    laptop.looks_at_remote().await;
    laptop
        .offers_become(|offers| {
            offers
                .made
                .first()
                .is_some_and(|made| made.held_up == Some(OfferHeldUp::NotRedeemed))
        })
        .await;
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
