//! The core pairing with a real Serving Server in process by pasting its
//! Invite, as the phone does: on one side a Server of Shaman's own, Serving
//! on loopback, and on the other nothing but the core in a directory of its
//! own, proving a key the test holds apart from it, as the app holds a
//! phone's.

mod support;

use std::{
    net::{Ipv6Addr, SocketAddr, SocketAddrV6},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use shaman::protocol::Way;
use shaman_thin_client::{
    BeginsIn, InviteReading, Outlook, PairingOutcome, ScanWords, TryInstead, WayState,
};

use support::{Gate, Phone, ServingServer};

/// The name this phone gives the Serving Server.
const DESK: &str = "desk";

/// An Invite's text, as `read_invite` reads it.
fn readable(reading: InviteReading) -> shaman_thin_client::ReadInvite {
    match reading {
        InviteReading::Readable { invite } => invite,
        InviteReading::Unreadable { message } => panic!("the Invite is unreadable: {message}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pasted_invite_is_read_for_the_confirmation_screen_dialling_nothing() {
    let server = ServingServer::start("thin-read").await;
    let gate = Gate::open(server.serving_address()).await;
    let invite = server.invite(vec![Way::Direct(gate.address())]).await;
    let phone = Phone::new();

    let read = readable(phone.core.read_invite(format!("  {invite}\n")));
    assert_eq!(read.hostname, server.hostname());
    assert_eq!(read.fingerprint, server.fingerprint());
    assert_eq!(read.ways.len(), 1);
    assert_eq!(read.ways[0].address, gate.address().to_string());
    assert_eq!(read.ways[0].note, None);
    assert_eq!(
        read.remote_name,
        server.hostname(),
        "offered from the hostname"
    );
    assert_eq!(read.held, None);
    assert_eq!(read.device_name, "iPhone");

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(gate.dialled(), 0, "nothing is dialled before Pair");
    assert!(server.peers().await.is_empty());
    assert_eq!(phone.outlook().await, Outlook::Nowhere);
    phone.shutdown();
    server.shutdown().await;
}

#[test]
fn a_malformed_invite_reads_as_its_parse_error() {
    let phone = Phone::new();
    let InviteReading::Unreadable { message } = phone.core.read_invite("shaman-v1-@@@".to_owned())
    else {
        panic!("a malformed Invite was read");
    };
    assert!(message.contains("Invite"), "{message}");
    let InviteReading::Unreadable { message } = phone.core.read_invite("hello".to_owned()) else {
        panic!("a malformed Invite was read");
    };
    assert_eq!(
        message,
        "Invite must start with `shaman-`; was all of it copied?"
    );
    phone.shutdown();
}

/// A scanner stops at the first code that is an Invite, a malformed one or
/// one from a newer Shaman included, so that the confirmation screen says
/// what is wrong with it, and passes over anything else it reads.
#[tokio::test(flavor = "multi_thread")]
async fn a_scanned_code_is_an_invite_by_its_form() {
    let server = ServingServer::start("thin-scan").await;
    let invite = server
        .invite(vec![Way::Direct("192.0.2.7:7791".parse().unwrap())])
        .await;
    let phone = Phone::new();
    for scanned in [
        invite.clone(),
        format!("{invite}\n"),
        "shaman-v1-@@@".to_owned(),
        "shaman-v2-eyJ3IjpbXX0".to_owned(),
    ] {
        assert!(phone.core.is_invite(scanned.clone()), "{scanned:?}");
    }
    for scanned in [
        "https://example.com".to_owned(),
        format!("https://example.com/?invite={invite}"),
        "WIFI:S:home;T:WPA;P:secret;;".to_owned(),
        "shaman".to_owned(),
        String::new(),
    ] {
        assert!(!phone.core.is_invite(scanned.clone()), "{scanned:?}");
    }
    phone.shutdown();
    server.shutdown().await;
}

#[test]
fn where_pairing_begins_says_to_scan_the_servers_code_or_paste_its_invite() {
    let phone = Phone::new();
    assert_eq!(
        phone.core.pairing_prompt(),
        "On the server, run /serve in Shaman and press q, or run `shaman serve invite --qr`, \
         then scan the code it draws or paste the invite here."
    );
    phone.shutdown();
}

#[test]
fn the_scanner_says_what_it_looks_for_and_why_it_cannot_scan() {
    let phone = Phone::new();
    assert_eq!(
        phone.core.scan_words(),
        ScanWords {
            looking: "Looking for a Shaman invite".to_owned(),
            passed_over: "That code is not a Shaman invite".to_owned(),
            camera_refused: "Shaman may not use the camera. Allow it in the Settings app to scan, \
                             or paste the invite."
                .to_owned(),
            no_camera: "This phone has no camera that can scan. Paste the invite instead."
                .to_owned(),
            camera_failed: "The camera stopped scanning. Paste the invite instead.".to_owned(),
        }
    );
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_names_both_sides_turns_the_outlook_and_survives_a_relaunch() {
    let server = ServingServer::start("thin-pair").await;
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    let phone = Phone::new();

    let paired = phone
        .core
        .confirm_pairing(invite, format!(" {DESK} "), "Pocket".to_owned())
        .await;
    assert_eq!(
        paired,
        PairingOutcome::Paired {
            name: DESK.to_owned()
        }
    );
    let outlook = Outlook::Remote {
        name: DESK.to_owned(),
        begins_in: BeginsIn::remote(DESK, None),
    };
    assert_eq!(phone.outlook().await, outlook);
    let servers = phone
        .servers_until(|servers| {
            servers.remotes.iter().all(|remote| {
                remote
                    .ways
                    .iter()
                    .all(|way| way.state != WayState::Checking)
            })
        })
        .await;
    assert_eq!(servers.remotes.len(), 1);
    let remote = &servers.remotes[0];
    assert_eq!(remote.name, DESK);
    assert_eq!(remote.fingerprint, server.fingerprint());
    assert_eq!(remote.ways.len(), 1);
    assert_eq!(remote.ways[0].state, WayState::Answering);
    assert_eq!(remote.ways[0].words, "answering");
    assert_eq!(
        phone.device().await.name,
        "Pocket",
        "the name chosen is kept"
    );

    let peers = server.peers().await;
    assert_eq!(peers.len(), 1);
    assert_eq!(
        peers[0].name, "Pocket",
        "the phone names itself so to the Server"
    );
    assert_eq!(peers[0].fingerprint, phone.fingerprint());
    assert!(
        server.offers().await.is_empty(),
        "the phone makes no offer to pair back"
    );

    let phone = phone.relaunched();
    assert_eq!(phone.outlook().await, outlook);
    let servers = phone
        .servers_until(|servers| {
            servers
                .remotes
                .first()
                .is_some_and(|remote| remote.ways[0].state == WayState::Answering)
        })
        .await;
    assert_eq!(servers.remotes.len(), 1);
    assert_eq!(servers.remotes[0].name, DESK);
    assert_eq!(phone.device().await.name, "Pocket");
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_keeps_dialling_until_a_way_answers() {
    let server = ServingServer::start("thin-keeps-dialling").await;
    let gate = Gate::held(server.serving_address()).await;
    let invite = server.invite(vec![Way::Direct(gate.address())]).await;
    let phone = Phone::new();

    let opening = gate.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        opening.open_now();
    });
    let paired = phone
        .core
        .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
        .await;
    assert_eq!(
        paired,
        PairingOutcome::Paired {
            name: DESK.to_owned()
        },
        "dialled again once the way answered, with no second tap"
    );
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_no_way_answers_fails_offering_to_try_again_and_keeps_nothing() {
    let server = ServingServer::start("thin-unanswered").await;
    let gate = Gate::held(server.serving_address()).await;
    let invite = server.invite(vec![Way::Direct(gate.address())]).await;
    let phone = Phone::new();

    let began = tokio::time::Instant::now();
    let failed = phone
        .core
        .confirm_pairing(invite.clone(), DESK.to_owned(), "Pocket".to_owned())
        .await;
    let PairingOutcome::Failed { message, again } = failed else {
        panic!("paired where no way answered: {failed:?}");
    };
    assert_eq!(again, TryInstead::TryAgain);
    assert!(
        message.starts_with("No way this invite offers answered"),
        "{message}"
    );
    // It dials again only where the next dial would begin in time, so it
    // gives up no more than one wait between dials early.
    let timings = support::timings();
    assert!(
        began.elapsed() >= timings.pairing_keeps_dialling - timings.pairing_redial,
        "kept dialling first"
    );
    assert!(phone.servers().await.remotes.is_empty(), "nothing is kept");
    assert_eq!(phone.outlook().await, Outlook::Nowhere);
    assert_eq!(phone.device().await.name, "iPhone", "nor the name");
    assert!(server.peers().await.is_empty());

    // The Invite was never spent, so trying again with it pairs.
    gate.open_now();
    let paired = phone
        .core
        .confirm_pairing(invite, DESK.to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(
        paired,
        PairingOutcome::Paired {
            name: DESK.to_owned()
        }
    );
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spent_invite_fails_in_the_servers_words_offering_to_scan_again() {
    let server = ServingServer::start("thin-spent").await;
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    let first = Phone::new();
    assert!(matches!(
        first
            .core
            .confirm_pairing(invite.clone(), DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));

    let second = Phone::new();
    let failed = second
        .core
        .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
        .await;
    assert_eq!(
        failed,
        PairingOutcome::Failed {
            message: "Invite has already been spent".to_owned(),
            again: TryInstead::ScanAgain,
        }
    );
    assert!(second.servers().await.remotes.is_empty());
    first.shutdown();
    second.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn names_either_side_would_not_keep_are_refused_beside_them_dialling_nothing() {
    let server = ServingServer::start("thin-names").await;
    let gate = Gate::open(server.serving_address()).await;
    let invite = server.invite(vec![Way::Direct(gate.address())]).await;
    let phone = Phone::new();

    let refused = phone
        .core
        .confirm_pairing(invite.clone(), DESK.to_owned(), "everywhere".to_owned())
        .await;
    assert!(
        matches!(refused, PairingOutcome::DeviceNameRefused { .. }),
        "{refused:?}"
    );
    let refused = phone
        .core
        .confirm_pairing(invite, "  ".to_owned(), "iPhone".to_owned())
        .await;
    assert!(
        matches!(refused, PairingOutcome::RemoteNameRefused { .. }),
        "{refused:?}"
    );
    assert_eq!(gate.dialled(), 0);
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invite_from_a_server_already_held_says_so_and_replaces_its_pairing() {
    let server = ServingServer::start("thin-replace").await;
    let phone = Phone::new();
    let first = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(first, DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));

    let again = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    let read = readable(phone.core.read_invite(again.clone()));
    assert_eq!(
        read.remote_name, DESK,
        "pre-filled with the name it is held by"
    );
    let held = read.held.expect("says it is held already");
    assert!(held.contains(&format!("as {DESK}")), "{held}");
    assert!(held.contains("replaces"), "{held}");

    let paired = phone
        .core
        .confirm_pairing(again, "studio".to_owned(), "iPhone".to_owned())
        .await;
    assert_eq!(
        paired,
        PairingOutcome::Paired {
            name: "studio".to_owned()
        }
    );
    let servers = phone.servers().await;
    assert_eq!(
        servers
            .remotes
            .iter()
            .map(|remote| remote.name.as_str())
            .collect::<Vec<_>>(),
        ["studio"],
        "one Remote for each Server"
    );
    assert_eq!(
        phone.outlook().await,
        Outlook::Remote {
            name: "studio".to_owned(),
            begins_in: BeginsIn::remote("studio", None),
        }
    );
    assert_eq!(server.peers().await.len(), 1, "updated in place there");

    // Under the name it was held by, too.
    let third = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(third, "studio".to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    assert_eq!(phone.servers().await.remotes.len(), 1);
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_pairing_with_a_server_already_held_keeps_the_pairing_it_had() {
    let server = ServingServer::start("thin-replace-failed").await;
    let phone = Phone::new();
    let first = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(first, DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let gate = Gate::held(server.serving_address()).await;
    let unanswered = server.invite(vec![Way::Direct(gate.address())]).await;

    let failed = phone
        .core
        .confirm_pairing(unanswered, "studio".to_owned(), "iPhone".to_owned())
        .await;
    assert!(
        matches!(failed, PairingOutcome::Failed { .. }),
        "{failed:?}"
    );
    let servers = phone.servers().await;
    assert_eq!(servers.remotes.len(), 1);
    assert_eq!(servers.remotes[0].name, DESK);
    assert_eq!(
        servers.remotes[0].ways[0].address,
        server.serving_address().to_string()
    );
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_local_way_is_shown_and_never_dialled() {
    let server = ServingServer::start("thin-link-local").await;
    let link_local = Way::Direct(SocketAddr::V6(SocketAddrV6::new(
        Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
        server.serving_address().port(),
        0,
        1,
    )));
    let direct = Way::Direct(server.serving_address());
    let invite = server
        .invite(vec![link_local.clone(), direct.clone()])
        .await;
    let phone = Phone::new();

    let read = readable(phone.core.read_invite(invite.clone()));
    assert_eq!(read.ways.len(), 2, "every offered way is shown");
    assert_eq!(read.ways[0].address, link_local.to_string());
    assert_eq!(
        read.ways[0].note.as_deref(),
        Some("not dialled from a phone")
    );
    assert_eq!(read.ways[1].note, None);

    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let servers = phone.servers().await;
    assert_eq!(
        servers.remotes[0]
            .ways
            .iter()
            .map(|way| way.address.clone())
            .collect::<Vec<_>>(),
        [direct.to_string()],
        "the Remote keeps no way the phone will not dial"
    );
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invite_offering_only_a_link_local_way_offers_to_scan_again() {
    let server = ServingServer::start("thin-only-link-local").await;
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    // The same Invite, offering a link-local way in place of the one it did.
    let encoded = invite.strip_prefix("shaman-v1-").expect("a v1 Invite");
    let mut payload: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
    payload["w"] = serde_json::json!([{ "direct": "[fe80::1%1]:7000" }]);
    let invite = format!(
        "shaman-v1-{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    );
    let phone = Phone::new();
    let read = readable(phone.core.read_invite(invite.clone()));
    assert_eq!(read.ways.len(), 1, "{:?}", read.ways);

    let failed = phone
        .core
        .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
        .await;
    assert_eq!(
        failed,
        PairingOutcome::Failed {
            message: "This invite offers no way a phone can dial.".to_owned(),
            again: TryInstead::ScanAgain,
        }
    );
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_way_that_does_not_answer_reads_as_not_answering() {
    let server = ServingServer::start("thin-ways").await;
    let gate = Gate::held(server.serving_address()).await;
    let invite = server
        .invite(vec![
            Way::Direct(server.serving_address()),
            Way::Direct(gate.address()),
        ])
        .await;
    let phone = Phone::new();
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let servers = phone
        .servers_until(|servers| {
            servers.remotes[0]
                .ways
                .iter()
                .all(|way| way.state != WayState::Checking)
        })
        .await;
    let ways = &servers.remotes[0].ways;
    assert_eq!(
        ways.iter()
            .map(|way| (way.state, way.words.as_str()))
            .collect::<Vec<_>>(),
        [
            (WayState::Answering, "answering"),
            (WayState::NotAnswering, "not answering")
        ]
    );
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_way_answering_late_leaves_the_outlook_at_the_remote_paired_since() {
    let first = ServingServer::start("thin-outlook-first").await;
    let second = ServingServer::start("thin-outlook-second").await;
    // Asking the way that holds its connection takes this phone a second.
    let mut timings = support::timings();
    timings.redeemer.handshake_timeout = Duration::from_secs(1);
    let phone = Phone::with(timings);
    let slow = Gate::held(first.serving_address()).await;
    let invite = first
        .invite(vec![
            Way::Direct(first.serving_address()),
            Way::Direct(slow.address()),
        ])
        .await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, "first".to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let invite = second
        .invite(vec![Way::Direct(second.serving_address())])
        .await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, "second".to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));

    // The first Remote's slow way is heard from after the second is paired.
    phone
        .servers_until(|servers| {
            servers.remotes.iter().any(|remote| {
                remote.name == "first" && remote.ways[1].state == WayState::NotAnswering
            })
        })
        .await;
    assert_eq!(
        phone.outlook().await,
        Outlook::Remote {
            name: "second".to_owned(),
            begins_in: BeginsIn::remote("second", None),
        },
        "the Outlook stays where it was last turned"
    );
    let phone = phone.relaunched();
    assert!(matches!(phone.outlook().await, Outlook::Remote { name, .. } if name == "second"));
    phone.shutdown();
    first.shutdown().await;
    second.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_made_whose_record_cannot_be_kept_is_shown_paired_and_kept_by_the_next_change() {
    let server = ServingServer::start("thin-unkept").await;
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    let phone = Phone::new();
    // The thin Client's own file cannot be replaced: a directory stands in
    // its place.
    let record = phone.directory().join("thin-client.json");
    std::fs::remove_file(&record).unwrap();
    std::fs::create_dir(&record).unwrap();
    std::fs::write(record.join("in the way"), b"").unwrap();

    let paired = phone
        .core
        .confirm_pairing(invite, DESK.to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(
        paired,
        PairingOutcome::Paired {
            name: DESK.to_owned()
        },
        "a Pairing made is never shown as failed"
    );
    assert_eq!(
        phone.outlook().await,
        Outlook::Remote {
            name: DESK.to_owned(),
            begins_in: BeginsIn::remote(DESK, None),
        }
    );
    assert_eq!(phone.device().await.name, "Pocket");
    assert_eq!(phone.servers().await.remotes[0].name, DESK);
    assert_eq!(server.peers().await.len(), 1);

    std::fs::remove_dir_all(&record).unwrap();
    phone
        .core
        .dismiss_pair_again()
        .await
        .expect("the next change is kept");
    let phone = phone.relaunched();
    assert!(matches!(phone.outlook().await, Outlook::Remote { name, .. } if name == DESK));
    assert_eq!(phone.device().await.name, "Pocket");
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_whose_commit_never_landed_is_dropped_as_the_core_opens() {
    let server = ServingServer::start("thin-uncommitted").await;
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    let phone = Phone::new();
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    // As the app ended mid-commit leaves it: the Remote marked uncommitted,
    // with nothing of the core left running to write over that.
    let phone = phone.relaunched_after(|directory| {
        let path = directory.join("remotes.json");
        let mut remotes: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        remotes[0]["uncommitted"] = true.into();
        std::fs::write(&path, serde_json::to_vec(&remotes).unwrap()).unwrap();
    });
    assert!(phone.servers().await.remotes.is_empty());
    assert_eq!(phone.outlook().await, Outlook::Nowhere);
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_replaced_pairings_slow_way_says_is_not_shown_of_the_pairing_in_its_place() {
    let server = ServingServer::start("thin-stale-ways").await;
    // Asking the way that holds its connection takes this phone a moment.
    let mut timings = support::timings();
    timings.redeemer.handshake_timeout = Duration::from_millis(500);
    let phone = Phone::with(timings);
    let before = Gate::open(server.serving_address()).await;
    let slow = Gate::held(server.serving_address()).await;
    let invite = server
        .invite(vec![
            Way::Direct(before.address()),
            Way::Direct(slow.address()),
        ])
        .await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    // Paired again at once, under the same name, by a way of its own, while
    // the first Pairing's slow way is still being asked.
    let after = Gate::open(server.serving_address()).await;
    let invite = server.invite(vec![Way::Direct(after.address())]).await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let answering = |servers: &shaman_thin_client::Servers| {
        servers.remotes.len() == 1
            && servers.remotes[0].ways.len() == 1
            && servers.remotes[0].ways[0].state == WayState::Answering
    };
    phone.servers_until(answering).await;

    // Past when the first Pairing's slow way is given up on.
    tokio::time::sleep(Duration::from_millis(800)).await;
    let servers = phone.servers().await;
    assert!(answering(&servers), "{servers:?}");
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_under_way_is_not_shown_and_one_whose_commit_fails_leaves_what_was_shown() {
    let server = ServingServer::start("thin-under-way").await;
    // The commit hears nothing for a while before it is given up on.
    let mut timings = support::timings();
    timings.redeemer.handshake_timeout = Duration::from_secs(1);
    timings.pairing_keeps_dialling = Duration::from_millis(100);
    let phone = Phone::with(timings);
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, DESK.to_owned(), "iPhone".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let once = Gate::once(server.serving_address()).await;
    let invite = server.invite(vec![Way::Direct(once.address())]).await;
    let core = phone.core.clone();
    let pairing = tokio::spawn(async move {
        core.confirm_pairing(invite, "studio".to_owned(), "iPhone".to_owned())
            .await
    });

    let records = phone.directory().join("remotes.json");
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while !String::from_utf8_lossy(&std::fs::read(&records).unwrap()).contains("uncommitted") {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the Remote is stored ahead of its commit");
    // Anything shown meanwhile shows the Remotes as they stood.
    phone
        .core
        .rename_device("Pocket".to_owned())
        .await
        .expect("renamed");
    let names = |servers: shaman_thin_client::Servers| {
        servers
            .remotes
            .into_iter()
            .map(|remote| remote.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(phone.servers().await), [DESK]);
    assert!(
        matches!(
            phone.outlook().await,
            Outlook::Remote { name, .. } if name == DESK
        ),
        "the Outlook stays"
    );

    let failed = pairing.await.unwrap();
    assert!(
        matches!(failed, PairingOutcome::Failed { .. }),
        "{failed:?}"
    );
    assert_eq!(names(phone.servers().await), [DESK]);
    assert!(
        !String::from_utf8_lossy(&std::fs::read(&records).unwrap()).contains("uncommitted"),
        "nothing kept of it"
    );
    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invite_from_a_newer_shaman_says_to_update_the_app_read_or_confirmed() {
    let server = ServingServer::start("thin-newer-invite").await;
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    let encoded = invite.strip_prefix("shaman-v1-").expect("a v1 Invite");
    let mut payload: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
    // Offering only a kind of way a newer Shaman knows.
    payload["w"] = serde_json::json!([{ "carrier_pigeon": "loft 7" }]);
    let only_newer_ways = format!(
        "shaman-v1-{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    );
    let phone = Phone::new();
    let words = "That invite is from a newer Shaman than this app can pair with; update the app";

    for newer in [format!("shaman-v2-{encoded}"), only_newer_ways] {
        let InviteReading::Unreadable { message } = phone.core.read_invite(newer.clone()) else {
            panic!("an Invite from a newer Shaman was read");
        };
        assert_eq!(message, words);
        let failed = phone
            .core
            .confirm_pairing(newer, DESK.to_owned(), "iPhone".to_owned())
            .await;
        assert_eq!(
            failed,
            PairingOutcome::Failed {
                message: words.to_owned(),
                again: TryInstead::ScanAgain,
            }
        );
    }

    phone.shutdown();
    server.shutdown().await;
}
