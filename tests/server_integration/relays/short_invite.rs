//! Pairing by a short Invite: a machine chosen from those a Relay lists, its
//! code typed, the ordinary Invite delivered by a password-authenticated key
//! exchange the Relay carries and cannot read, and that Invite redeemed as any
//! other (ADR-0065). Two real Servers are joined through a real Relay, and a
//! hostile party stands in the path wherever it could gain anything.
//!
//! What a hostile party says is written out here from the ADR rather than
//! borrowed from the Server, so it holds the Server to what was written down.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use rcgen::{KeyPair, PublicKeyData};
use ring::{aead, digest, hkdf, hmac};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use suru::protocol::{
    DeliveredInvite, FetchShortInviteRequest, IssueInviteRequest, IssuedInvite, RemoteStatus,
    SessionErrorCode, ShortCode, ShortCodeStanding, ShortCodeState, Way,
};
use suru_relay_protocol::{RelayMessage, ServerMessage};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream},
    time::{Duration, timeout},
};
use tokio_tungstenite::tungstenite::Message;

use super::{
    Captured, RelaySocket, TestRelay, TestServer, error_code, greet, heard, relay_timings,
    scripted_relay, tell,
};
use crate::{
    server_integration::relays::pairing::{REMOTE, error_message, serving_through},
    support::{
        PROGRESS_DEADLINE,
        relay_voice::carried,
        short_code_forms::{another_code, form_held, forms_of},
    },
};

const PREAMBLE: &[u8] = b"suru-short-invite";
const VERSION: u8 = 1;
const SAYS_SHARE: u8 = 1;
const SAYS_DELIVERY: u8 = 2;
const SAYS_FAILED: u8 = 3;
const SAYS_REFUSED: u8 = 4;
const NO_CODE: u8 = 1;

impl TestServer {
    /// An Invite to this Server offering `ways`, and its short code.
    async fn invite_with_code(&self, ways: Vec<Way>) -> (IssuedInvite, ShortCode) {
        let issued = self
            .client
            .issue_invite(IssueInviteRequest { ways })
            .await
            .expect("issue an Invite");
        let code = issued
            .short_code
            .clone()
            .expect("an Invite offering a Relay has a short code")
            .code;
        (issued, code)
    }

    /// This Server's identity key, as a machine is listed by it.
    fn listed_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.identity().subject_public_key_info())
    }

    /// Asks `machine` for its Invite by `code`, through the Relays at `relays`.
    async fn fetch(
        &self,
        machine: &TestServer,
        relays: &[String],
        code: &ShortCode,
    ) -> anyhow::Result<DeliveredInvite> {
        self.client
            .fetch_short_invite(FetchShortInviteRequest {
                server_key: machine.listed_key(),
                relays: relays.to_vec(),
                code: code.clone(),
            })
            .await
    }

    /// Waits until this Server tells its Client its short code stands as
    /// `state`.
    async fn told_code(&mut self, state: ShortCodeState) -> ShortCodeStanding {
        let telling = timeout(PROGRESS_DEADLINE, async {
            loop {
                match self.client.next().await {
                    Some(suru::managed_client::ManagedEvent::ShortCode(standing))
                        if standing.state == state =>
                    {
                        return standing;
                    }
                    Some(_) => {}
                    None => panic!("the Client's stream ended"),
                }
            }
        })
        .await;
        telling.unwrap_or_else(|_| panic!("the Server never said its short code stands {state:?}"))
    }
}

/// The workstation and laptop of a test, the workstation Serving through
/// `relay` and both logged in there.
async fn pair_of(relay: &TestRelay, channel: &str) -> (TestServer, TestServer) {
    serving_through(relay, channel, relay_timings()).await
}

/// The opening message a redeemer sends: `key` and `share`, its SPAKE2 message.
fn opening(key: &[u8], share: &[u8]) -> Vec<u8> {
    let mut opening = PREAMBLE.to_vec();
    opening.push(VERSION);
    let length = u16::try_from(2 + key.len() + share.len()).unwrap();
    opening.extend_from_slice(&length.to_be_bytes());
    opening.extend_from_slice(&u16::try_from(key.len()).unwrap().to_be_bytes());
    opening.extend_from_slice(key);
    opening.extend_from_slice(share);
    opening
}

/// One thing said in the exchange.
async fn said<S: AsyncRead + Unpin>(stream: &mut S) -> Option<Vec<u8>> {
    let mut length = [0_u8; 2];
    stream.read_exact(&mut length).await.ok()?;
    let mut body = vec![0_u8; usize::from(u16::from_be_bytes(length))];
    stream.read_exact(&mut body).await.ok()?;
    Some(body)
}

async fn say<S: AsyncWrite + Unpin>(stream: &mut S, body: &[u8]) {
    let mut framed = u16::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    framed.extend_from_slice(body);
    let _ = stream.write_all(&framed).await;
    let _ = stream.flush().await;
}

/// The keys the ADR says an exchange ends in.
struct Keys {
    transcript: [u8; 32],
    redeemer: hmac::Key,
    serving: hmac::Key,
    invite: [u8; 32],
}

fn keys(
    shared: &[u8],
    way: &str,
    redeeming: &[u8],
    serving: &[u8],
    redeemer_share: &[u8],
    serving_share: &[u8],
) -> Keys {
    let mut hash = digest::Context::new(&digest::SHA256);
    for part in [
        b"suru short invite v1".as_slice(),
        &[VERSION],
        way.as_bytes(),
        redeeming,
        serving,
        redeemer_share,
        serving_share,
    ] {
        hash.update(&u32::try_from(part.len()).unwrap().to_be_bytes());
        hash.update(part);
    }
    let transcript: [u8; 32] = hash.finish().as_ref().try_into().unwrap();
    let secret = hkdf::Salt::new(hkdf::HKDF_SHA256, &transcript).extract(shared);
    let expanded = |label: &[u8]| {
        let mut key = [0_u8; 32];
        secret
            .expand(&[label], hkdf::HKDF_SHA256)
            .unwrap()
            .fill(&mut key)
            .unwrap();
        key
    };
    Keys {
        transcript,
        redeemer: hmac::Key::new(
            hmac::HMAC_SHA256,
            &expanded(b"suru short invite v1 redeemer proof"),
        ),
        serving: hmac::Key::new(
            hmac::HMAC_SHA256,
            &expanded(b"suru short invite v1 serving proof"),
        ),
        invite: expanded(b"suru short invite v1 invite"),
    }
}

/// An Invite to the Server whose key is `key`, offering `way`, by the format
/// every Invite has.
fn invite_naming(key: &[u8], way: &str) -> String {
    let payload = serde_json::json!({
        "w": [{ "relay": way }],
        "k": URL_SAFE_NO_PAD.encode(key),
        "t": URL_SAFE_NO_PAD.encode([9_u8; 32]),
        "h": "stand-in",
    });
    format!(
        "suru-v1-{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    )
}

/// A Relay of the test's own logging every Server in, and handing `substitute`
/// each join asked there in place of the machine asked for, counting them.
async fn substituting_relay<F, Fut>(
    substitute: F,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>)
where
    F: Fn(DuplexStream, Vec<u8>, String) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let joins = Arc::new(AtomicUsize::new(0));
    let counted = joins.clone();
    let script = move |mut socket: RelaySocket, relay: String, _: usize| {
        let (substitute, joins) = (substitute.clone(), counted.clone());
        async move {
            if !greet(&mut socket, &relay).await {
                return;
            }
            match heard(&mut socket).await {
                Some(ServerMessage::BeginLogin { .. }) => {
                    tell(
                        &mut socket,
                        &RelayMessage::LoginStarted {
                            verification_uri: "https://login.example.com".to_owned(),
                            user_code: "CODE-HELD".to_owned(),
                            expires_in_seconds: 900,
                        },
                    )
                    .await;
                    tell(
                        &mut socket,
                        &RelayMessage::LoginDone {
                            account: suru_relay_protocol::Account {
                                provider: "scripted".to_owned(),
                                username: "octocat".to_owned(),
                            },
                        },
                    )
                    .await;
                }
                Some(ServerMessage::Join { server }) => {
                    joins.fetch_add(1, Ordering::AcqRel);
                    tell(&mut socket, &RelayMessage::Joined).await;
                    substitute(carried(socket), server.0, relay).await;
                    return;
                }
                _ => {}
            }
            while heard(&mut socket).await.is_some() {}
        }
    };
    let (address, answering) = scripted_relay(script).await;
    (address, joins, answering)
}

/// A laptop logged in at the stand-in Relay at `address`.
async fn laptop_at(address: &str, channel: &str) -> TestServer {
    let laptop = TestServer::start(channel).await;
    laptop.client.add_relay(address.to_owned()).await.unwrap();
    laptop.client.begin_relay_login(address).await.unwrap();
    let login = laptop.client.follow_relay_login(address).await.unwrap();
    assert!(
        matches!(
            login.outcome,
            suru::protocol::RelayLoginOutcome::Done { .. }
        ),
        "{login:?}"
    );
    laptop
}

/// The request to fetch a machine's Invite by `key`, its listing the stand-in's
/// word.
fn fetching(key: &[u8], relay: &str, code: &ShortCode) -> FetchShortInviteRequest {
    FetchShortInviteRequest {
        server_key: URL_SAFE_NO_PAD.encode(key),
        relays: vec![relay.to_owned()],
        code: code.clone(),
    }
}

/// `relay`'s connection log, once it has `joins` lines: one for each join ended.
async fn joins_logged(relay: &TestRelay, joins: usize) -> String {
    timeout(PROGRESS_DEADLINE, async {
        loop {
            let log = String::from_utf8(relay.connection_log.0.lock().unwrap().clone()).unwrap();
            if log.lines().count() >= joins {
                return log;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the joins end and are logged")
}

#[tokio::test]
async fn a_machine_chosen_and_its_code_typed_reach_the_preview_and_the_usual_redemption_pairs_them()
{
    let relay = TestRelay::start().await;
    let (mut workstation, laptop) = pair_of(&relay, "short-invite-pairs").await;
    let through = Way::Relay(relay.address());
    let (issued, code) = workstation.invite_with_code(vec![through.clone()]).await;
    assert_eq!(code.grouped().len(), 9, "two groups of four");

    let delivered = laptop
        .fetch(&workstation, &[relay.address()], &code)
        .await
        .expect("the code delivers the Invite");
    assert_eq!(
        delivered.invite, issued.invite,
        "the Invite issued, and no other"
    );
    assert_eq!(delivered.preview.ways, vec![through]);
    assert_eq!(
        delivered.preview.fingerprint,
        blake3::hash(&workstation.identity().subject_public_key_info())
            .to_hex()
            .to_string()
    );
    assert!(
        laptop.client.list_remotes().await.unwrap().is_empty(),
        "being delivered an Invite forms nothing"
    );
    assert!(workstation.client.list_peers().await.unwrap().is_empty());
    workstation.told_code(ShortCodeState::Delivered).await;

    let remote = laptop
        .redeem_as(delivered.invite, REMOTE)
        .await
        .expect("the Invite is redeemed as any other");
    assert_eq!(
        remote.fingerprint,
        blake3::hash(&workstation.identity().subject_public_key_info())
            .to_hex()
            .to_string()
    );
    laptop.wait_for_remote(RemoteStatus::Available).await;
    assert_eq!(workstation.client.list_peers().await.unwrap().len(), 1);
    workstation.told_code(ShortCodeState::Spent).await;

    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn the_invite_delivered_offers_every_way_ticked_and_a_direct_way_is_preferred() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = pair_of(&relay, "short-invite-every-way").await;
    let direct = Way::Direct(workstation.serving_address());
    let through = Way::Relay(relay.address());
    let (_, code) = workstation
        .invite_with_code(vec![direct.clone(), through.clone()])
        .await;

    let delivered = laptop
        .fetch(&workstation, &[relay.address()], &code)
        .await
        .expect("the code delivers the Invite");
    assert_eq!(
        delivered.preview.ways,
        vec![direct.clone(), through.clone()]
    );

    laptop
        .redeem_as(delivered.invite, REMOTE)
        .await
        .expect("the Invite is redeemed as any other");
    laptop.wait_for_remote(RemoteStatus::Available).await;
    // Everything since the exchange was answered directly, so the Relay
    // joined that exchange, beside the join that found the workstation waiting,
    // and nothing more.
    let connection_log = joins_logged(&relay, 2).await;
    assert_eq!(connection_log.lines().count(), 2, "{connection_log}");

    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn an_invite_of_direct_ways_alone_shows_the_long_string_alone() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = pair_of(&relay, "short-invite-direct-alone").await;
    let issued = workstation
        .client
        .issue_invite(IssueInviteRequest {
            ways: vec![Way::Direct(workstation.serving_address())],
        })
        .await
        .unwrap();
    assert!(issued.short_code.is_none());
    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_wrong_code_forms_nothing_ends_the_code_leaves_the_long_string_and_tells_both_readers() {
    let relay = TestRelay::start().await;
    let (mut workstation, laptop) = pair_of(&relay, "short-invite-wrong").await;
    let (issued, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;

    let refused = laptop
        .fetch(&workstation, &[relay.address()], &another_code(&code))
        .await
        .expect_err("a wrong code delivers nothing");
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeDead);
    let told = error_message(&refused);
    assert!(told.contains("dead"), "{told}");
    assert!(
        told.contains("Do not type it again") && told.contains("new one"),
        "a Relay standing in gets a guess at each retype, which this machine never sees: {told}"
    );
    assert!(told.contains("Relay may be at fault"), "{told}");
    workstation.told_code(ShortCodeState::WrongCodeTried).await;
    assert!(laptop.client.list_remotes().await.unwrap().is_empty());
    assert!(workstation.client.list_peers().await.unwrap().is_empty());

    let too_late = laptop
        .fetch(&workstation, &[relay.address()], &code)
        .await
        .expect_err("the right code, after a wrong one, finds it gone");
    assert_eq!(error_code(&too_late), SessionErrorCode::ShortCodeNotLive);

    laptop
        .redeem_as(issued.invite, REMOTE)
        .await
        .expect("the long string stays good");

    laptop.shutdown().await;
    workstation.shutdown().await;
}

/// What locked out a code's holder before: each of these has yet to finish its
/// opening, so none has anything the holder needs, and the code stands as it
/// did with nothing to report of them.
#[tokio::test]
async fn parties_saying_one_byte_and_stopping_keep_out_no_one_who_holds_the_code() {
    let relay = TestRelay::start().await;
    let timings = relay_timings().with_short_invite_reading_timeout(Duration::from_millis(400));
    let (mut workstation, laptop) = serving_through(&relay, "short-invite-stalling", timings).await;
    let (issued, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    let serving_key = workstation.identity().subject_public_key_info();
    let asker = KeyPair::generate().unwrap();
    relay
        .voice()
        .log_in(&relay.provider, &asker, "583231", "octocat")
        .await;
    let mut stalling = Vec::new();
    for _ in 0..8 {
        let mut stream = relay.voice().joined(&asker, &serving_key).await;
        stream.write_all(&PREAMBLE[..1]).await.unwrap();
        stalling.push(stream);
    }
    let delivered = laptop
        .fetch(&workstation, &[relay.address()], &code)
        .await
        .expect("nothing stands between the right code and its Invite");
    assert_eq!(delivered.invite, issued.invite);
    workstation.told_code(ShortCodeState::Delivered).await;
    laptop.shutdown().await;
    workstation.shutdown().await;
}

/// With no code live one byte is all it takes to be refused, so nothing is held
/// open for what it might go on to say.
#[tokio::test]
async fn one_byte_with_no_code_live_is_refused_with_no_wait() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = pair_of(&relay, "short-invite-one-byte").await;
    let asker = KeyPair::generate().unwrap();
    relay
        .voice()
        .log_in(&relay.provider, &asker, "583231", "octocat")
        .await;
    let mut stream = relay
        .voice()
        .joined(&asker, &workstation.identity().subject_public_key_info())
        .await;
    stream.write_all(&PREAMBLE[..1]).await.unwrap();
    assert_eq!(said(&mut stream).await.unwrap(), [SAYS_REFUSED, NO_CODE]);
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_new_code_keeps_the_invite_it_is_for() {
    let relay = TestRelay::start().await;
    let (mut workstation, laptop) = pair_of(&relay, "short-invite-new-code").await;
    let (issued, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    laptop
        .fetch(&workstation, &[relay.address()], &another_code(&code))
        .await
        .expect_err("a wrong code delivers nothing");
    workstation.told_code(ShortCodeState::WrongCodeTried).await;

    let fresh = workstation.client.new_short_code().await.unwrap();
    let standing = workstation.told_code(ShortCodeState::Live).await;
    assert_eq!(standing.serial, fresh.serial);
    let delivered = laptop
        .fetch(&workstation, &[relay.address()], &fresh.code)
        .await
        .expect("the new code delivers the same Invite");
    assert_eq!(delivered.invite, issued.invite);

    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_code_expires_with_its_invite_and_a_new_one_is_not_had_for_it() {
    let relay = TestRelay::start().await;
    let timings = suru::server::ServerTimings {
        invite_ttl: Duration::from_millis(40),
        ..relay_timings()
    };
    let (mut workstation, laptop) = serving_through(&relay, "short-invite-expiry", timings).await;
    let (_, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    workstation.told_code(ShortCodeState::Ended).await;
    let refused = laptop
        .fetch(&workstation, &[relay.address()], &code)
        .await
        .expect_err("an expired code is not live");
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeNotLive);
    let refused = workstation.client.new_short_code().await.unwrap_err();
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeNotLive);
    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_code_is_superseded_with_its_invite_by_the_next() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = pair_of(&relay, "short-invite-superseded").await;
    let way = || vec![Way::Relay(relay.address())];
    let (_, first) = workstation.invite_with_code(way()).await;
    let (second_invite, second) = workstation.invite_with_code(way()).await;
    assert_ne!(first, second);
    let refused = laptop
        .fetch(&workstation, &[relay.address()], &first)
        .await
        .unwrap_err();
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeDead);
    let refused = laptop
        .fetch(&workstation, &[relay.address()], &second)
        .await
        .unwrap_err();
    assert_eq!(
        error_code(&refused),
        SessionErrorCode::ShortCodeNotLive,
        "the code the attempt with the old one spent"
    );
    // It says how to get one where nobody is at a screen, and what it means if
    // the other machine does show one.
    let said = error_message(&refused);
    assert!(said.contains("suru serve invite"), "{said}");
    assert!(said.contains("does not offer this Relay"), "{said}");
    laptop
        .redeem_as(second_invite.invite, REMOTE)
        .await
        .expect("the Invite goes on");
    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_code_is_spent_with_its_invite_by_the_long_string() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = pair_of(&relay, "short-invite-spent").await;
    let (issued, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    laptop.redeem_as(issued.invite, REMOTE).await.unwrap();
    let other = TestServer::start("short-invite-spent-other").await;
    other.log_in(&relay, "583231", "octocat").await;
    let refused = other
        .fetch(&workstation, &[relay.address()], &code)
        .await
        .unwrap_err();
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeNotLive);
    other.shutdown().await;
    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_code_kept_in_memory_alone_ends_with_the_server_as_its_invite_does() {
    let relay = TestRelay::start().await;
    // Where a join reaches the connection the Server left behind, the attempt
    // is given up on at once rather than at its full bound.
    let timings = relay_timings()
        .with_short_invite_timeout(Duration::from_millis(100))
        .with_serving_handshake_timeout(Duration::from_millis(100));
    let (mut workstation, laptop) = serving_through(&relay, "short-invite-restart", timings).await;
    let (issued, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    workstation.restart().await;
    let refused = timeout(PROGRESS_DEADLINE, async {
        loop {
            match laptop.fetch(&workstation, &[relay.address()], &code).await {
                Err(error) if error_code(&error) == SessionErrorCode::ShortCodeNotLive => {
                    return error;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
                Ok(_) => panic!("a code does not outlive its Server"),
            }
        }
    })
    .await
    .expect("the Server comes back to wait at the Relay");
    assert!(error_message(&refused).contains("Issue an Invite there"));
    assert!(
        laptop.redeem_as(issued.invite, REMOTE).await.is_err(),
        "nor does the Invite"
    );
    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn no_code_live_tells_the_reader_to_issue_an_invite_there_with_nothing_spent() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = pair_of(&relay, "short-invite-none-live").await;
    let refused = laptop
        .fetch(
            &workstation,
            &[relay.address()],
            &ShortCode::parse("K7QF-2MXP").unwrap(),
        )
        .await
        .expect_err("with no code live there is nothing to try");
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeNotLive);
    assert!(error_message(&refused).contains("Issue an Invite there"));
    laptop.shutdown().await;
    workstation.shutdown().await;
}

/// A connection saying it speaks the exchange, with no code live, is answered
/// and let go at once, with nothing more asked of it: no key to read, nothing
/// held open, and a bound far past this test's patience.
#[tokio::test]
async fn an_exchange_with_no_code_live_is_refused_at_once_and_held_open_by_nothing() {
    let relay = TestRelay::start().await;
    let (workstation, laptop) = pair_of(&relay, "short-invite-refused-at-once").await;
    let asker = KeyPair::generate().unwrap();
    relay
        .voice()
        .log_in(&relay.provider, &asker, "583231", "octocat")
        .await;
    let mut stream = relay
        .voice()
        .joined(&asker, &workstation.identity().subject_public_key_info())
        .await;
    let mut opening = PREAMBLE.to_vec();
    opening.push(VERSION);
    stream.write_all(&opening).await.unwrap();
    assert_eq!(said(&mut stream).await.unwrap(), [SAYS_REFUSED, NO_CODE]);
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_stalled_exchange_is_closed_within_its_bound_and_counts_as_its_guess() {
    let relay = TestRelay::start().await;
    let timings = relay_timings().with_short_invite_timeout(Duration::from_millis(50));
    let (mut workstation, laptop) = serving_through(&relay, "short-invite-stalled", timings).await;
    let (issued, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    let asker = KeyPair::generate().unwrap();
    relay
        .voice()
        .log_in(&relay.provider, &asker, "583231", "octocat")
        .await;
    let mut stream = relay
        .voice()
        .joined(&asker, &workstation.identity().subject_public_key_info())
        .await;
    let key = asker.subject_public_key_info();
    let (_, share) = Spake2::<Ed25519Group>::start_a(
        &Password::new(another_code(&code).grouped().replace('-', "")),
        &Identity::new(&key),
        &Identity::new(&workstation.identity().subject_public_key_info()),
    );
    stream.write_all(&opening(&key, &share)).await.unwrap();
    let answer = said(&mut stream).await.unwrap();
    assert_eq!(answer[0], SAYS_SHARE);
    // Says nothing more, and is let go.
    let mut rest = Vec::new();
    timeout(PROGRESS_DEADLINE, stream.read_to_end(&mut rest))
        .await
        .expect("the Serving side lets it go at its bound")
        .unwrap();
    workstation.told_code(ShortCodeState::LeftUnfinished).await;
    laptop
        .redeem_as(issued.invite, REMOTE)
        .await
        .expect("the long string stays good");
    laptop.shutdown().await;
    workstation.shutdown().await;
}

/// A connection dialled to the listener speaks TLS and nothing else, whatever
/// is live: nothing reads what it opens with, so the exchange gets no hearing.
#[tokio::test]
async fn the_exchange_gets_no_hearing_on_the_serving_listener() {
    let relay = TestRelay::start().await;
    let (mut workstation, laptop) = pair_of(&relay, "short-invite-direct").await;
    let direct = Way::Direct(workstation.serving_address());
    let (_, code) = workstation
        .invite_with_code(vec![direct, Way::Relay(relay.address())])
        .await;
    workstation.told_code(ShortCodeState::Live).await;
    let mut dialled = tokio::net::TcpStream::connect(workstation.serving_address())
        .await
        .unwrap();
    let (_, share) = Spake2::<Ed25519Group>::start_a(
        &Password::new(code.grouped().replace('-', "")),
        &Identity::new(b"a key"),
        &Identity::new(&workstation.identity().subject_public_key_info()),
    );
    dialled.write_all(&opening(b"a key", &share)).await.unwrap();
    let mut answer = Vec::new();
    let _ = timeout(PROGRESS_DEADLINE, dialled.read_to_end(&mut answer)).await;
    assert_ne!(answer.first(), Some(&SAYS_SHARE));
    assert!(
        answer.get(0..2) != Some(&[0, 34][..]),
        "it is answered by nothing the exchange says: {answer:?}"
    );
    let delivered = laptop
        .fetch(&workstation, &[relay.address()], &code)
        .await
        .expect("the code is untouched");
    assert!(!delivered.invite.is_empty());
    laptop.shutdown().await;
    workstation.shutdown().await;
}

/// A guess at a machine's code by anyone logged in under its Account: griefing
/// the issue accepts, which costs a new code and shows on the screen, and gains
/// nothing else.
#[tokio::test]
async fn a_party_guessing_as_the_redeemer_learns_nothing_of_the_code_and_only_burns_it() {
    let relay = TestRelay::start().await;
    let (mut workstation, laptop) = pair_of(&relay, "short-invite-griefing").await;
    let (issued, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    let asker = KeyPair::generate().unwrap();
    relay
        .voice()
        .log_in(&relay.provider, &asker, "583231", "octocat")
        .await;
    let serving_key = workstation.identity().subject_public_key_info();
    let mut stream = relay.voice().joined(&asker, &serving_key).await;
    let key = asker.subject_public_key_info();
    let (spake, share) = Spake2::<Ed25519Group>::start_a(
        &Password::new(another_code(&code).grouped().replace('-', "")),
        &Identity::new(&key),
        &Identity::new(&serving_key),
    );
    stream.write_all(&opening(&key, &share)).await.unwrap();
    let answer = said(&mut stream).await.unwrap();
    assert_eq!(answer[0], SAYS_SHARE);
    assert_eq!(answer.len(), 34, "SPAKE2's message and nothing besides");
    let shared = spake.finish(&answer[1..]).unwrap();
    let guessed = keys(
        &shared,
        &relay.address(),
        &key,
        &serving_key,
        &share,
        &answer[1..],
    );
    say(
        &mut stream,
        hmac::sign(&guessed.redeemer, &guessed.transcript).as_ref(),
    )
    .await;
    assert_eq!(said(&mut stream).await.unwrap(), [SAYS_FAILED]);
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty(), "nothing derived from the key comes after");

    workstation.told_code(ShortCodeState::WrongCodeTried).await;
    assert!(workstation.client.list_peers().await.unwrap().is_empty());
    laptop
        .redeem_as(issued.invite, REMOTE)
        .await
        .expect("the long string stays good");
    laptop.shutdown().await;
    workstation.shutdown().await;
}

/// The Relay stands in for the machine the laptop asked for, guessing. The
/// proof it is sent can be tested against its one guess and no other, and the
/// laptop refuses what it says next: no Pairing, no code, no second try.
#[tokio::test]
async fn a_relay_standing_in_for_the_serving_server_forms_nothing_and_learns_no_code() {
    let heard_from_redeemer = Arc::new(Mutex::new(Vec::new()));
    let hearing = heard_from_redeemer.clone();
    let (address, joins, _answering) = substituting_relay(move |mut stream, listed, way| {
        let hearing = hearing.clone();
        async move {
            let mut preamble = vec![0_u8; PREAMBLE.len() + 1];
            stream.read_exact(&mut preamble).await.unwrap();
            let body = said(&mut stream).await.unwrap();
            let (length, rest) = body.split_first_chunk::<2>().unwrap();
            let (redeeming, redeemer_share) =
                rest.split_at(usize::from(u16::from_be_bytes(*length)));
            let (spake, share) = Spake2::<Ed25519Group>::start_b(
                &Password::new("22222222"),
                &Identity::new(redeeming),
                &Identity::new(&listed),
            );
            let shared = spake.finish(redeemer_share).unwrap();
            let guessed = keys(&shared, &way, redeeming, &listed, redeemer_share, &share);
            let mut answer = vec![SAYS_SHARE];
            answer.extend_from_slice(&share);
            say(&mut stream, &answer).await;
            let proof = said(&mut stream).await.unwrap();
            assert!(
                hmac::verify(&guessed.redeemer, &guessed.transcript, &proof).is_err(),
                "a wrong guess proves wrong"
            );
            {
                let mut heard = hearing.lock().unwrap();
                heard.extend_from_slice(&preamble);
                heard.extend_from_slice(&body);
                heard.extend_from_slice(&proof);
            }
            // It presses on as though it had guessed right.
            let mut delivery = vec![SAYS_DELIVERY];
            delivery.extend_from_slice(hmac::sign(&guessed.serving, &guessed.transcript).as_ref());
            delivery.extend_from_slice(&[0; 32]);
            say(&mut stream, &delivery).await;
        }
    })
    .await;
    let laptop = laptop_at(&address, "short-invite-standing-in").await;
    let code = ShortCode::parse("K7QF-2MXP").unwrap();
    let refused = laptop
        .client
        .fetch_short_invite(fetching(b"the key the Relay lists", &address, &code))
        .await
        .expect_err("a Relay guessing delivers nothing");
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeDead);
    assert!(laptop.client.list_remotes().await.unwrap().is_empty());
    assert_eq!(joins.load(Ordering::Acquire), 1, "and is not tried again");
    assert_holds_nothing_of(&heard_from_redeemer.lock().unwrap(), &code);
    laptop.shutdown().await;
}

/// A Relay that passes everything through, having named its own key to the
/// laptop in place of the machine's: the two Servers name different parties,
/// so neither gets anything, though the code the laptop typed was right.
#[tokio::test]
async fn a_relay_passing_the_exchange_on_under_a_key_of_its_own_forms_nothing_on_either_side() {
    let relay = TestRelay::start().await;
    let (mut workstation, onlooker) = pair_of(&relay, "short-invite-passing-on").await;
    let (issued, code) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    let serving_key = workstation.identity().subject_public_key_info();
    let asker = Arc::new(KeyPair::generate().unwrap());
    relay
        .voice()
        .log_in(&relay.provider, &asker, "583231", "octocat")
        .await;
    let voice = relay.voice();
    let (address, _, _answering) = substituting_relay(move |mut near, _, _| {
        let (voice, asker, serving_key) = (voice.clone(), asker.clone(), serving_key.clone());
        async move {
            let mut far = voice.joined(&asker, &serving_key).await;
            let _ = tokio::io::copy_bidirectional(&mut near, &mut far).await;
        }
    })
    .await;
    let laptop = laptop_at(&address, "short-invite-passing-on-laptop").await;
    let refused = laptop
        .client
        .fetch_short_invite(fetching(b"a key of the Relay's own", &address, &code))
        .await
        .expect_err("the two name different parties");
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeDead);
    assert!(laptop.client.list_remotes().await.unwrap().is_empty());
    workstation.told_code(ShortCodeState::WrongCodeTried).await;
    assert!(workstation.client.list_peers().await.unwrap().is_empty());
    onlooker
        .redeem_as(issued.invite, REMOTE)
        .await
        .expect("the long string stays good");
    onlooker.shutdown().await;
    laptop.shutdown().await;
    workstation.shutdown().await;
}

/// Serves a whole exchange as a party knowing `code`, under `listed`, delivering
/// `invite`.
async fn serving_exchange(
    mut stream: DuplexStream,
    code: &str,
    listed: &[u8],
    way: &str,
    invite: &str,
) {
    let mut preamble = vec![0_u8; PREAMBLE.len() + 1];
    stream.read_exact(&mut preamble).await.unwrap();
    let body = said(&mut stream).await.unwrap();
    let (length, rest) = body.split_first_chunk::<2>().unwrap();
    let (redeeming, redeemer_share) = rest.split_at(usize::from(u16::from_be_bytes(*length)));
    let (spake, share) = Spake2::<Ed25519Group>::start_b(
        &Password::new(code),
        &Identity::new(redeeming),
        &Identity::new(listed),
    );
    let shared = spake.finish(redeemer_share).unwrap();
    let agreed = keys(&shared, way, redeeming, listed, redeemer_share, &share);
    let mut answer = vec![SAYS_SHARE];
    answer.extend_from_slice(&share);
    say(&mut stream, &answer).await;
    let proof = said(&mut stream).await.unwrap();
    hmac::verify(&agreed.redeemer, &agreed.transcript, &proof).expect("the code is right");
    let mut sealed = invite.as_bytes().to_vec();
    aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &agreed.invite).unwrap(),
    )
    .seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key([0; 12]),
        aead::Aad::from(&agreed.transcript),
        &mut sealed,
    )
    .unwrap();
    let mut delivery = vec![SAYS_DELIVERY];
    delivery.extend_from_slice(hmac::sign(&agreed.serving, &agreed.transcript).as_ref());
    delivery.extend_from_slice(&sealed);
    say(&mut stream, &delivery).await;
}

/// A party that knows the code and delivers an Invite for another key than it
/// said it was: the list was wrong or lying, and the reader is told so. The
/// same exchange delivering the key it named goes through, so what is refused
/// here is that alone.
#[tokio::test]
async fn a_delivered_invite_whose_key_is_not_the_one_listed_is_refused_saying_so() {
    let (address, _, _answering) = substituting_relay(move |stream, listed, way| async move {
        let delivered_key: &[u8] = if listed == b"the key the Relay lists" {
            b"another key altogether"
        } else {
            &listed
        };
        let invite = invite_naming(delivered_key, &way);
        serving_exchange(stream, "K7QF2MXP", &listed, &way, &invite).await;
    })
    .await;
    let laptop = laptop_at(&address, "short-invite-another-key").await;
    let code = ShortCode::parse("K7QF-2MXP").unwrap();
    let refused = laptop
        .client
        .fetch_short_invite(fetching(b"the key the Relay lists", &address, &code))
        .await
        .expect_err("the key delivered is not the key listed");
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeKeyMismatch);
    assert!(error_message(&refused).contains("wrong or was lying"));
    assert!(laptop.client.list_remotes().await.unwrap().is_empty());

    let agreeing = laptop
        .client
        .fetch_short_invite(fetching(b"a key it holds to", &address, &code))
        .await
        .expect("the key delivered is the key listed");
    assert_eq!(agreeing.preview.hostname, "stand-in");
    laptop.shutdown().await;
}

/// What a machine's Invite says of itself is no more believed by being
/// delivered: it reaches a reader held to what can be shown.
#[tokio::test]
async fn a_delivered_invites_hostname_reaches_the_preview_as_what_can_be_shown() {
    let (address, _, _answering) = substituting_relay(move |stream, listed, way| async move {
        let payload = serde_json::json!({
            "w": [{ "relay": way }],
            "k": URL_SAFE_NO_PAD.encode(&listed),
            "t": URL_SAFE_NO_PAD.encode([9_u8; 32]),
            "h": "lap\u{202e}top\u{200b}",
        });
        let invite = format!(
            "suru-v1-{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        );
        serving_exchange(stream, "K7QF2MXP", &listed, &way, &invite).await;
    })
    .await;
    let laptop = laptop_at(&address, "short-invite-hostname").await;
    let code = ShortCode::parse("K7QF-2MXP").unwrap();
    let delivered = laptop
        .client
        .fetch_short_invite(fetching(b"its key", &address, &code))
        .await
        .expect("the key delivered is the key listed");
    assert_eq!(delivered.preview.hostname, "laptop");
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_machine_at_two_relays_is_one_attempt_and_a_join_that_fails_is_no_guess() {
    let first = Arc::new(AtomicUsize::new(0));
    let hung_up = first.clone();
    // The first hears the attempt and hangs up: the attempt ends there.
    let (at_first, _, _first_answering) = substituting_relay(move |mut stream, _, _| {
        let hung_up = hung_up.clone();
        async move {
            let mut preamble = vec![0_u8; PREAMBLE.len() + 1];
            let _ = stream.read_exact(&mut preamble).await;
            hung_up.fetch_add(1, Ordering::AcqRel);
        }
    })
    .await;
    let (at_second, second_joins, _second_answering) =
        substituting_relay(move |_, _, _| async move {}).await;
    let laptop = laptop_at(&at_first, "short-invite-two-relays").await;
    laptop.client.add_relay(at_second.clone()).await.unwrap();
    laptop.client.begin_relay_login(&at_second).await.unwrap();
    laptop.client.follow_relay_login(&at_second).await.unwrap();
    let code = ShortCode::parse("K7QF-2MXP").unwrap();
    let refused = laptop
        .client
        .fetch_short_invite(FetchShortInviteRequest {
            server_key: URL_SAFE_NO_PAD.encode(b"its key"),
            relays: vec![at_first.clone(), at_second.clone()],
            code: code.clone(),
        })
        .await
        .expect_err("the attempt never finishes");
    assert_eq!(error_code(&refused), SessionErrorCode::ShortCodeDead);
    assert_eq!(first.load(Ordering::Acquire), 1);
    assert_eq!(
        second_joins.load(Ordering::Acquire),
        0,
        "a second Relay is a second guess, once anything of the exchange has been said"
    );

    // Where no exchange byte went out, moving on makes no guess.
    let relay = TestRelay::start().await;
    let other_relay = TestRelay::start().await;
    let (workstation, mover) = pair_of(&relay, "short-invite-moving-on").await;
    other_relay.provider.approve(
        "nothing yet",
        suru_relay::Identity {
            subject: "583231".to_owned(),
            username: "octocat".to_owned(),
        },
    );
    mover.log_in(&other_relay, "583231", "octocat").await;
    let (_, live) = workstation
        .invite_with_code(vec![Way::Relay(relay.address())])
        .await;
    let delivered = mover
        .fetch(
            &workstation,
            // It does not wait at the first, which refuses the join.
            &[other_relay.address(), relay.address()],
            &live,
        )
        .await
        .expect("the second Relay carries the one attempt");
    assert!(!delivered.invite.is_empty());
    laptop.shutdown().await;
    mover.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_relay_the_invite_does_not_offer_carries_no_attempt_and_the_next_one_does() {
    let (offered, other) = (TestRelay::start().await, TestRelay::start().await);
    let (workstation, laptop) = pair_of(&offered, "short-invite-not-offered").await;
    for server in [&workstation, &laptop] {
        server.log_in(&other, "583231", "octocat").await;
    }
    workstation.serve_through(&other, true).await;
    other
        .voice()
        .log_in(
            &other.provider,
            &KeyPair::generate().unwrap(),
            "583231",
            "octocat",
        )
        .await;
    let asker = KeyPair::generate().unwrap();
    other
        .voice()
        .log_in(&other.provider, &asker, "583231", "octocat")
        .await;
    drop(
        other
            .voice()
            .joined(&asker, &workstation.identity().subject_public_key_info())
            .await,
    );
    let (_, code) = workstation
        .invite_with_code(vec![Way::Relay(offered.address())])
        .await;
    let delivered = laptop
        .fetch(&workstation, &[other.address(), offered.address()], &code)
        .await
        .expect("the one the Invite offers takes the attempt");
    assert!(!delivered.invite.is_empty());
    laptop.shutdown().await;
    workstation.shutdown().await;
}

fn assert_holds_nothing_of(seen: &[u8], code: &ShortCode) {
    assert!(!seen.is_empty(), "something was there to look at");
    let forms = forms_of(code);
    if let Some(form) = form_held(seen, &forms) {
        panic!("{form:?} is among what was seen");
    }
}

/// What one connection through the Relay carried once it was joined: what the
/// Server on it said, and what it was told.
#[derive(Default)]
struct Carried {
    said: Vec<u8>,
    told: Vec<u8>,
}

/// Records every message a Server and the Relay say to each other, as a Relay
/// reads them, in both directions.
async fn recording_relay(
    relay: std::net::SocketAddr,
    seen: Arc<Mutex<Vec<u8>>>,
    carried: Arc<Mutex<Vec<Arc<Mutex<Carried>>>>>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let answering = tokio::spawn(async move {
        while let Ok((server, _)) = listener.accept().await {
            let seen = seen.clone();
            let this_one = Arc::new(Mutex::new(Carried::default()));
            carried.lock().unwrap().push(this_one.clone());
            tokio::spawn(async move {
                let Ok(mut server) = tokio_tungstenite::accept_async(server).await else {
                    return;
                };
                let Ok(stream) = tokio::net::TcpStream::connect(relay).await else {
                    return;
                };
                let Ok((mut relay, _)) =
                    tokio_tungstenite::client_async(format!("ws://{relay}/connect"), stream).await
                else {
                    return;
                };
                let note = |message: &Message| match message {
                    Message::Text(text) => {
                        seen.lock().unwrap().extend_from_slice(text.as_bytes());
                    }
                    Message::Binary(bytes) => seen.lock().unwrap().extend_from_slice(bytes),
                    _ => {}
                };
                loop {
                    tokio::select! {
                        said = server.next() => {
                            let Some(Ok(message)) = said else { return };
                            note(&message);
                            if let Message::Binary(bytes) = &message {
                                this_one.lock().unwrap().said.extend_from_slice(bytes);
                            }
                            if relay.send(message).await.is_err() { return; }
                        }
                        answered = relay.next() => {
                            let Some(Ok(message)) = answered else { return };
                            note(&message);
                            if let Message::Binary(bytes) = &message {
                                this_one.lock().unwrap().told.extend_from_slice(bytes);
                            }
                            if server.send(message).await.is_err() { return; }
                        }
                    }
                }
            });
        }
    });
    (address, answering)
}

/// Each thing said in the exchanges carried is exactly as long as the exchange
/// says, and the key claimed is the redeemer's own: nothing derived from the
/// code has room to travel in any of it. The one attempt that delivered and the
/// one that failed are all there were, and each side was told what the other
/// said, unaltered.
fn assert_exactly_what_the_exchange_says(
    carried: &[Arc<Mutex<Carried>>],
    redeemers_key: &[u8],
    invite: usize,
) {
    let carried = carried
        .iter()
        .map(|carried| carried.lock().unwrap())
        .collect::<Vec<_>>();
    let redeemers = carried
        .iter()
        .filter(|carried| carried.said.starts_with(PREAMBLE))
        .collect::<Vec<_>>();
    assert_eq!(redeemers.len(), 2, "one right, one wrong");
    let key_at = PREAMBLE.len() + 1 + 2 + 2;
    let opening = key_at + redeemers_key.len() + 33;
    let share = 2 + 1 + 33;
    let delivery = 2 + 1 + 32 + invite + 16;
    let failed = 2 + 1;
    for (redeemer, told) in redeemers.iter().zip([share + delivery, share + failed]) {
        assert_eq!(redeemer.said.len(), opening + 2 + 32, "what it said");
        assert_eq!(redeemer.told.len(), told, "what it was told");
        assert_eq!(
            &redeemer.said[key_at..key_at + redeemers_key.len()],
            redeemers_key,
            "the key it claims is its own"
        );
        assert!(
            carried
                .iter()
                .any(|serving| serving.said == redeemer.told && serving.told == redeemer.said),
            "and the Relay passed it through as it was"
        );
    }
}

/// A right code and a wrong one, carried by a real Relay with every message
/// recorded, its connection log kept, and both Servers logging all they can:
/// none of it holds the code, a hash of it, or anything else it could be
/// searched by.
#[tokio::test]
async fn no_code_or_anything_derived_from_one_reaches_a_relay_message_or_a_log() {
    let captured = Captured::default();
    let writer = captured.clone();
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish(),
    );
    let relay = TestRelay::start().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let (recording_at, _recording) =
        recording_relay(relay.running().address(), seen.clone(), recorded.clone()).await;
    relay.route.retarget(recording_at);
    let (workstation, laptop) = pair_of(&relay, "short-invite-no-leak").await;
    let laptop_key = laptop.identity().subject_public_key_info();
    let way = || vec![Way::Relay(relay.address())];

    let (_, right) = workstation.invite_with_code(way()).await;
    let delivered = laptop
        .fetch(&workstation, &[relay.address()], &right)
        .await
        .expect("the right code delivers the Invite");
    let token = serde_json::from_slice::<serde_json::Value>(
        &URL_SAFE_NO_PAD
            .decode(delivered.invite.strip_prefix("suru-v1-").unwrap())
            .unwrap(),
    )
    .unwrap()["t"]
        .as_str()
        .unwrap()
        .to_owned();

    let (_, burned) = workstation.invite_with_code(way()).await;
    let attempted = another_code(&burned);
    let refused = laptop
        .fetch(&workstation, &[relay.address()], &attempted)
        .await
        .expect_err("a wrong code delivers nothing");

    laptop.shutdown().await;
    workstation.shutdown().await;
    let connection_log = joins_logged(&relay, 3).await.into_bytes();
    assert_exactly_what_the_exchange_says(
        &recorded.lock().unwrap(),
        &laptop_key,
        delivered.invite.len(),
    );
    let told = refused.to_string().into_bytes();
    let log = captured.0.lock().unwrap().clone();
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.windows(PREAMBLE.len()).any(|at| at == PREAMBLE),
        "the exchange did pass where it was recorded"
    );
    assert!(
        !seen.windows(token.len()).any(|at| at == token.as_bytes()),
        "nor does the Relay see the Invite delivered"
    );
    for code in [&right, &burned, &attempted] {
        assert_holds_nothing_of(&seen, code);
        assert_holds_nothing_of(&log, code);
        assert_holds_nothing_of(&connection_log, code);
        assert_holds_nothing_of(&told, code);
    }
}
