//! `suru relay` and `suru serve`, through the real binary, against a real
//! Server it starts as a Client would and a real Relay whose identity provider
//! the test scripts.
//!
//! Nothing here reaches the real user's Server. Each machine is its own state
//! directory, config root, home and Channel, keeps its identity key in a file
//! and Serves on a port of the system's choosing, and everything launched
//! against it goes with it.

use std::{
    path::{Path, PathBuf},
    process::{Output, Stdio},
};

use serde_json::{Value, json};
use suru::protocol::{
    InviteRelays, OfferInviteRequest, RedeemInviteRequest, RelayLoginLead, RelayLoginOutcome,
    RelayLoginStanding, RelayMachinesAnswer, SettingMutation, Way,
};
use suru_relay::{SCRIPTED_VERIFICATION_URI, ScriptedProvider};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, ChildStdout, Command},
    time::{Duration, timeout},
};

use super::{TestRelay, TestServer};
use crate::support::{PROGRESS_DEADLINE, detached_servers::DetachedServers};

/// How the commands exit, as `docs/relay/README.md` says.
const FAILED: i32 = 1;
const BAD_COMMAND_LINE: i32 = 2;
const REMOVAL_UNCONFIRMED: i32 = 3;
const LOGIN_REFUSED: i32 = 4;
const LOGIN_LAPSED: i32 = 5;
#[cfg(unix)]
const INTERRUPTED: i32 = 130;

/// A machine with no one at its screen: a home and a Channel of its own, whose
/// Config Document has it Serve on a loopback port of the system's choosing, so
/// that no test takes a port from another or from the Server on this machine.
struct Machine {
    // Declared first, so what it launched is gone before anything else is.
    servers: DetachedServers,
    channel: &'static str,
}

impl Machine {
    fn new(channel: &'static str) -> Self {
        let machine = Self {
            servers: DetachedServers::new(),
            channel,
        };
        std::fs::create_dir_all(machine.config_root()).expect("create the config root");
        machine.pin(json!({ "serving": { "port": 0, "bindAddress": "127.0.0.1" } }));
        machine
    }

    fn config_root(&self) -> PathBuf {
        self.servers.state_dir().join("config")
    }

    fn document(&self) -> PathBuf {
        self.config_root().join("suru.jsonc")
    }

    /// Has the Config Document say `settings`, as a hand or a rollout would.
    fn pin(&self, settings: Value) {
        std::fs::write(self.document(), settings.to_string()).expect("write the Config Document");
    }

    /// `suru <arguments>` on this machine, as a script there would run it.
    fn suru(&self, arguments: &[&str]) -> Command {
        let state_dir = self.servers.state_dir();
        let mut command = Command::new(env!("CARGO_BIN_EXE_suru"));
        command
            .args(arguments)
            .env("SURU_STATE_DIR", state_dir)
            .env("SURU_DATA_DIR", state_dir)
            .env("SURU_CONFIG_DIR", self.config_root())
            .env("SURU_CHANNEL", self.channel)
            .env("SURU_IDENTITY_STORE", "file")
            .env("HOME", state_dir)
            .env("USERPROFILE", state_dir)
            .env("XDG_CONFIG_HOME", state_dir.join("xdg"))
            .env("NO_PROXY", "127.0.0.1,localhost")
            .envs(self.servers.isolated_environment())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        command
    }

    async fn run(&self, arguments: &[&str]) -> Output {
        timeout(PROGRESS_DEADLINE * 2, self.suru(arguments).output())
            .await
            .unwrap_or_else(|_| panic!("`suru {arguments:?}` did not settle"))
            .expect("run suru")
    }

    /// Runs `suru <arguments>`, failing the test should it not succeed.
    async fn succeed(&self, arguments: &[&str]) -> Output {
        let output = self.run(arguments).await;
        assert_eq!(
            output.status.code(),
            Some(0),
            "`suru {arguments:?}` failed: {}",
            said(&output.stderr)
        );
        output
    }

    /// Runs `suru <arguments>`, failing the test should it not exit `code`.
    async fn exits(&self, code: i32, arguments: &[&str]) -> Output {
        let output = self.run(arguments).await;
        assert_eq!(
            output.status.code(),
            Some(code),
            "`suru {arguments:?}` exited otherwise: {}{}",
            said(&output.stdout),
            said(&output.stderr)
        );
        output
    }

    /// `suru relay list --json`, parsed.
    async fn listing(&self) -> Value {
        let output = self.succeed(&["relay", "list", "--json"]).await;
        serde_json::from_slice(&output.stdout).expect("the listing is JSON")
    }

    /// Waits for the Relay at `address` to read `state` in the listing.
    async fn wait_for_state(&self, address: &str, state: &str) {
        let reading = timeout(PROGRESS_DEADLINE * 2, async {
            loop {
                let listing = self.listing().await;
                let reads = listing["relays"]
                    .as_array()
                    .expect("the listing lists its Relays")
                    .iter()
                    .any(|relay| relay["address"] == address && relay["state"] == state);
                if reads {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        reading
            .await
            .unwrap_or_else(|_| panic!("the Relay at {address} never read {state}"));
    }

    /// `suru relay login <address>`, under way, its streams piped.
    fn begin_login(&self, address: &str) -> Child {
        self.suru(&["relay", "login", address])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run suru relay login")
    }

    /// Adds the Relay at `address` and logs in there through the command line,
    /// the Relay approving it as `octocat`.
    async fn log_in(&self, relay: &TestRelay, address: &str) {
        self.succeed(&["relay", "add", address]).await;
        let mut login = self.begin_login(address);
        let (_, code) = shown_login(&mut login).await;
        relay.approve(&code, "1", "octocat");
        let output = settled(login).await;
        assert_eq!(output.status.code(), Some(0), "{}", said(&output.stderr));
    }

    /// Every Log this machine's Servers have written, as one text.
    fn logs(&self) -> String {
        let channel_dir = self.servers.state_dir().join(self.channel);
        let mut logs = String::new();
        let mut read = |path: &Path| {
            if let Ok(log) = std::fs::read_to_string(path) {
                logs.push_str(&log);
            }
        };
        read(&channel_dir.join("server.log"));
        for entry in std::fs::read_dir(channel_dir.join("log"))
            .into_iter()
            .flatten()
            .flatten()
        {
            read(&entry.path());
        }
        logs
    }
}

fn said(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Reads the line `suru relay login` printed on standard output, answering the
/// address and the code it says.
async fn shown_login(login: &mut Child) -> (String, String) {
    let stdout = login.stdout.as_mut().expect("standard output is piped");
    let mut lines = BufReader::new(stdout).lines();
    let line = timeout(PROGRESS_DEADLINE * 2, lines.next_line())
        .await
        .expect("the login shows where to go before anything is done there")
        .expect("read standard output")
        .expect("the login says where to go");
    parsed_login(&line)
}

/// The address and the code a login's line says, as the docs say it does.
fn parsed_login(line: &str) -> (String, String) {
    let (address, code) = line
        .strip_prefix("Visit ")
        .and_then(|line| line.split_once(" and enter the code "))
        .unwrap_or_else(|| panic!("not a login's line: {line}"));
    (address.to_owned(), code.to_owned())
}

/// Waits for `suru relay login` to end.
async fn settled(login: Child) -> Output {
    timeout(PROGRESS_DEADLINE * 2, login.wait_with_output())
        .await
        .expect("the login's command ends with the login")
        .expect("wait for suru relay login")
}

/// The ways of an Invite as `--json` lists them, each as `kind address`.
fn ways_of(invite: &Value) -> Vec<String> {
    invite["ways"]
        .as_array()
        .expect("an Invite lists its ways")
        .iter()
        .map(|way| {
            format!(
                "{} {}",
                way["kind"].as_str().unwrap(),
                way["address"].as_str().unwrap()
            )
        })
        .collect()
}

/// Waits for a Server the Client of `server` is logged in beside to be waiting
/// at the Relay at `address`, so an Invite offering it can be redeemed.
async fn wait_for_a_machine_waiting(server: &TestServer, address: &str) {
    timeout(PROGRESS_DEADLINE * 2, async {
        loop {
            if let RelayMachinesAnswer::Listed { machines, .. } = server
                .client
                .list_relay_machines(address)
                .await
                .expect("ask the Relay for its machines")
                .answer
                && machines.iter().any(|machine| machine.waiting)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the machine comes to wait at the Relay");
}

/// The set-up `docs/relay/README.md` shows, command for command: from nothing to
/// an Invite that redeems in `/pair`, by the route `/pair` redeems through.
#[tokio::test]
async fn a_headless_machine_goes_from_nothing_to_an_invite_that_redeems() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("headless");

    machine.succeed(&["relay", "add", &address]).await;

    let mut login = machine.begin_login(&address);
    let (visit, code) = shown_login(&mut login).await;
    assert_eq!(visit, SCRIPTED_VERIFICATION_URI);
    relay.approve(&code, "1", "octocat");
    let logged_in = settled(login).await;
    assert_eq!(logged_in.status.code(), Some(0));
    let told = said(&logged_in.stderr);
    assert!(
        told.contains(&format!("Logged in at {address} as octocat")),
        "{told}"
    );

    let served_through = machine
        .succeed(&["relay", "serve-through", &address, "on"])
        .await;
    let told = said(&served_through.stderr);
    assert!(told.contains("reachable through this Relay"), "{told}");
    assert!(
        told.contains("once Serving is on"),
        "Serving is not on yet: {told}"
    );

    let issued = machine
        .succeed(&["serve", "invite", "--no-addresses"])
        .await;
    let told = said(&issued.stderr);
    assert!(told.contains("Serving is now on"), "{told}");
    assert!(told.contains(&format!("Relay {address}")), "{told}");
    let invite = said(&issued.stdout);
    let invite = invite
        .strip_suffix('\n')
        .expect("the Invite is one line, newline-terminated");
    assert!(invite.starts_with("suru-v"), "{invite}");
    assert!(!invite.contains('\n'), "only the Invite: {invite}");
    assert!(!invite.contains('\u{1b}'), "no colour: {invite}");

    let laptop = TestServer::start("headless-laptop").await;
    laptop.log_in(&relay, "1", "octocat").await;
    wait_for_a_machine_waiting(&laptop, &address).await;
    let preview = laptop
        .client
        .preview_invite(invite)
        .await
        .expect("preview the Invite");
    assert_eq!(preview.ways, vec![Way::Relay(address.clone())]);
    let remote = laptop
        .client
        .redeem_invite(RedeemInviteRequest {
            invite: invite.into(),
            name: Some("headless".to_owned()),
            ways: Vec::new(),
            pair_back: false,
        })
        .await
        .expect("redeem the Invite through the Relay");
    assert_eq!(remote.name, "headless");

    assert!(
        !machine.logs().contains(invite),
        "an Invite reaches no Log (ADR-0008)"
    );
    laptop.shutdown().await;
}

/// What `docs/relay/README.md` shows is what [`Machine`]s here are run through,
/// and how the commands exit is what it says.
#[test]
fn the_docs_show_the_commands_and_the_statuses_that_are_tested() {
    let docs = include_str!("../../../docs/relay/README.md");
    for line in [
        "suru relay add https://relay.example.com",
        "suru relay login https://relay.example.com",
        "#   Visit https://github.com/login/device and enter the code WDJB-MJHT",
        "suru relay serve-through https://relay.example.com on",
        "INVITE=$(suru serve invite --no-addresses)",
    ] {
        assert!(docs.lines().any(|shown| shown == line), "{line}");
    }
    for status in [
        0,
        FAILED,
        BAD_COMMAND_LINE,
        REMOVAL_UNCONFIRMED,
        LOGIN_REFUSED,
        LOGIN_LAPSED,
        130,
    ] {
        assert!(
            docs.contains(&format!("\n| {status} | ")),
            "status {status}"
        );
        assert!(
            suru::cli::EXIT_STATUS.contains(&format!("\n  {status} ")),
            "status {status}"
        );
    }
}

#[tokio::test]
async fn a_short_code_comes_on_standard_output_with_json_and_is_left_out_of_a_piped_standard_error()
{
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("short-code");
    machine.log_in(&relay, &address).await;
    machine
        .succeed(&["relay", "serve-through", &address, "on"])
        .await;

    // The code is as secret as the Invite, and a provisioning script's standard
    // error is what its log keeps.
    let plain = machine
        .succeed(&["serve", "invite", "--no-addresses"])
        .await;
    let told = said(&plain.stderr);
    assert!(
        told.contains("short code") && told.contains("--json"),
        "says it has one, and how to have it: {told}"
    );

    let issued = machine
        .succeed(&["serve", "invite", "--no-addresses", "--json"])
        .await;
    let invite: Value = serde_json::from_slice(&issued.stdout).expect("the Invite as JSON");
    let mut fields = invite.as_object().unwrap().keys().collect::<Vec<_>>();
    fields.sort();
    assert_eq!(fields, vec!["invite", "short_code", "ways"], "{invite}");
    assert_eq!(ways_of(&invite), vec![format!("relay {address}")]);
    let code = invite["short_code"].as_str().expect("a short code");
    assert_eq!(code.len(), 9, "two groups: {code}");
    assert!(!said(&issued.stderr).contains(code));
    let logs = machine.logs();
    assert!(!logs.contains(code), "a code reaches no Log");
    assert!(!logs.contains(invite["invite"].as_str().unwrap()));
}

#[tokio::test]
async fn the_listing_says_how_each_relay_stands_as_documented() {
    let mut relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("listing");

    let empty = machine.succeed(&["relay", "list"]).await;
    assert_eq!(said(&empty.stdout), "", "nothing to list");
    assert!(said(&empty.stderr).contains("holds no Relay"));
    assert_eq!(
        machine.listing().await,
        json!({ "serving": false, "relays": [] })
    );

    machine.succeed(&["relay", "add", &address]).await;
    assert_eq!(
        machine.listing().await,
        json!({
            "serving": false,
            "relays": [{
                "address": address,
                "state": "login_needed",
                "account": null,
                "unreachable_reason": null,
                "serve_through": false,
                "login_under_way": false,
                "pinned_in": null,
            }],
        })
    );
    let shown = said(&machine.succeed(&["relay", "list"]).await.stdout);
    assert!(shown.contains(&address), "{shown}");
    assert!(shown.contains("Login needed"), "{shown}");

    // A login under way is said to be, and its code is left to `relay login`.
    let mut login = machine.begin_login(&address);
    let (_, code) = shown_login(&mut login).await;
    let listing = machine.listing().await;
    assert_eq!(listing["relays"][0]["login_under_way"], true);
    assert!(!listing.to_string().contains(&code), "{listing}");
    let shown = said(&machine.succeed(&["relay", "list"]).await.stdout);
    assert!(!shown.contains(&code), "{shown}");
    assert!(shown.contains("relay login"), "{shown}");
    relay.approve(&code, "1", "octocat");
    settled(login).await;

    machine
        .succeed(&["relay", "serve-through", &address, "on"])
        .await;
    assert_eq!(
        machine.listing().await["relays"][0],
        json!({
            "address": address,
            "state": "logged_in",
            "account": { "provider": "scripted", "username": "octocat" },
            "unreachable_reason": null,
            "serve_through": true,
            "login_under_way": false,
            "pinned_in": null,
        })
    );
    let shown = said(&machine.succeed(&["relay", "list"]).await.stdout);
    assert!(shown.contains("Logged in as octocat"), "{shown}");
    assert!(
        shown.contains("Serves through once Serving is on"),
        "{shown}"
    );

    relay.stop().await;
    machine.wait_for_state(&address, "unreachable").await;
    let listing = machine.listing().await;
    assert!(
        listing["relays"][0]["unreachable_reason"].is_string(),
        "{listing}"
    );
    let shown = said(&machine.succeed(&["relay", "list"]).await.stdout);
    assert!(shown.contains("Unreachable"), "{shown}");

    // As in `/relay`, a login cannot help a Relay merely Unreachable: the Login
    // stands, and no login is begun.
    let standing = machine.succeed(&["relay", "login", &address]).await;
    assert_eq!(said(&standing.stdout), "", "there is nothing to visit");
    assert!(said(&standing.stderr).contains("Unreachable"));
}

#[tokio::test]
async fn a_relay_is_added_written_the_one_way_and_adding_it_again_is_success() {
    let machine = Machine::new("adding");

    let added = machine
        .succeed(&["relay", "add", "HTTPS://Relay.Example.COM:443/"])
        .await;
    assert_eq!(said(&added.stdout), "", "everything else is standard error");
    assert!(said(&added.stderr).contains("https://relay.example.com"));
    assert_eq!(
        machine.listing().await["relays"][0]["address"],
        "https://relay.example.com"
    );

    // Running a script twice is normal: what it asked for holds.
    let again = machine
        .succeed(&["relay", "add", "https://relay.example.com"])
        .await;
    assert!(said(&again.stderr).contains("already"), "{again:?}");
    assert_eq!(
        machine.listing().await["relays"].as_array().unwrap().len(),
        1
    );

    // An international name is held as it is dialled.
    machine.succeed(&["relay", "add", "bücher.example"]).await;
    assert_eq!(
        machine.listing().await["relays"][1]["address"],
        "https://xn--bcher-kva.example"
    );

    for bad in [
        "https://user:secret@relay.example.com",
        "ftp://relay.example.com",
        "not an address",
    ] {
        let refused = machine
            .exits(BAD_COMMAND_LINE, &["relay", "add", bad])
            .await;
        let told = said(&refused.stderr);
        assert!(told.contains("a Relay's address is"), "{told}");
        assert!(!told.contains("secret"), "{told}");
    }
}

#[tokio::test]
async fn a_login_already_under_way_is_followed_and_a_standing_login_is_said_to_stand() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("followed-login");
    machine.succeed(&["relay", "add", &address]).await;

    let mut first = machine.begin_login(&address);
    let (_, code) = shown_login(&mut first).await;

    // A second command follows that login rather than giving it up for another.
    let mut second = machine.begin_login(&address);
    let (_, code_again) = shown_login(&mut second).await;
    assert_eq!(code_again, code);

    relay.approve(&code, "1", "octocat");
    for login in [first, second] {
        let output = settled(login).await;
        assert_eq!(output.status.code(), Some(0), "{}", said(&output.stderr));
    }

    let standing = machine.succeed(&["relay", "login", &address]).await;
    assert_eq!(said(&standing.stdout), "", "there is nothing to visit");
    let told = said(&standing.stderr);
    assert!(told.contains("Logged in as octocat"), "{told}");
}

#[tokio::test]
async fn a_login_that_ends_otherwise_says_how_by_its_exit_status() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("refused-login");
    machine.succeed(&["relay", "add", &address]).await;

    let mut denied = machine.begin_login(&address);
    let (_, code) = shown_login(&mut denied).await;
    assert!(relay.provider.deny(&code));
    let output = settled(denied).await;
    assert_eq!(output.status.code(), Some(LOGIN_REFUSED));
    let told = said(&output.stderr);
    assert!(told.contains("The login at"), "{told}");

    // A refusal says what the Client's list would, the operator included.
    relay.provider.set_admitted("2", false);
    let mut unadmitted = machine.begin_login(&address);
    let (_, code) = shown_login(&mut unadmitted).await;
    relay.approve(&code, "2", "stranger");
    let output = settled(unadmitted).await;
    assert_eq!(output.status.code(), Some(LOGIN_REFUSED));
    let told = said(&output.stderr);
    assert!(told.contains("ask the Relay's operator"), "{told}");
    let shown = said(&machine.succeed(&["relay", "list"]).await.stdout);
    assert!(shown.contains("ask the Relay's operator"), "{shown}");

    let unknown = machine
        .exits(FAILED, &["relay", "login", "https://elsewhere.example"])
        .await;
    assert!(
        said(&unknown.stderr).contains("relay add"),
        "{}",
        said(&unknown.stderr)
    );
}

#[tokio::test]
async fn a_login_nobody_finishes_lapses_by_its_own_exit_status() {
    let relay =
        TestRelay::with_provider(ScriptedProvider::new().with_expiry(Duration::from_millis(50)))
            .await;
    let address = relay.address();
    let machine = Machine::new("lapsed-login");
    machine.succeed(&["relay", "add", &address]).await;

    let output = timeout(
        PROGRESS_DEADLINE * 2,
        machine.suru(&["relay", "login", &address]).output(),
    )
    .await
    .expect("a login that expires ends")
    .expect("run suru relay login");
    assert_eq!(
        output.status.code(),
        Some(LOGIN_LAPSED),
        "{}",
        said(&output.stderr)
    );
}

#[tokio::test]
async fn a_relay_that_cannot_be_reached_fails_a_login_as_a_failure() {
    let mut relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("unreachable-login");
    machine.succeed(&["relay", "add", &address]).await;
    relay.stop().await;

    let refused = machine.exits(FAILED, &["relay", "login", &address]).await;
    assert_eq!(said(&refused.stdout), "", "no login began");
    assert!(
        said(&refused.stderr).contains("Could not begin a login"),
        "{}",
        said(&refused.stderr)
    );
}

/// A login is the Server's, so its command ending takes nothing from it: the
/// next command finds the same code, as a Client would (ADR-0048).
#[tokio::test]
async fn killing_the_command_leaves_the_login_under_way_where_a_client_finds_it() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("killed-login");
    machine.succeed(&["relay", "add", &address]).await;

    let mut login = machine.begin_login(&address);
    let (_, code) = shown_login(&mut login).await;
    login.kill().await.expect("kill the command");
    login.wait().await.expect("the command ends");

    let mut found = machine.begin_login(&address);
    let (_, found_code) = shown_login(&mut found).await;
    assert_eq!(found_code, code, "the same login, still under way");

    // A Client attached to the same Server finds it, by the Server's own list.
    let client = suru::managed_client::ManagedClient::connect(
        machine
            .servers
            .client_config(machine.channel)
            .with_identity_store(suru::IdentityStoreChoice::File),
    )
    .await
    .expect("attach a Client");
    let listing = client.list_relays().await.expect("list the Relays");
    let under_way = listing.relays[0].login.as_ref().expect("a login under way");
    assert_eq!(under_way.user_code, code);
    assert_eq!(under_way.outcome, RelayLoginOutcome::Pending);

    relay.approve(&code, "1", "octocat");
    assert_eq!(settled(found).await.status.code(), Some(0));
}

#[cfg(unix)]
#[tokio::test]
async fn interrupting_the_command_says_the_login_goes_on_and_exits_as_interrupted() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("interrupted-login");
    machine.succeed(&["relay", "add", &address]).await;

    let mut login = machine.begin_login(&address);
    let (_, code) = shown_login(&mut login).await;
    let pid = i32::try_from(login.id().expect("the command runs")).unwrap();
    // SAFETY: `kill` is given the pid of a child this test has not yet waited on.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGINT) }, 0);
    let output = settled(login).await;
    assert_eq!(
        output.status.code(),
        Some(INTERRUPTED),
        "{}",
        said(&output.stderr)
    );
    let told = said(&output.stderr);
    assert!(told.contains("under way"), "{told}");
    assert!(told.contains("relay login"), "{told}");

    let mut found = machine.begin_login(&address);
    assert_eq!(shown_login(&mut found).await.1, code);
    relay.approve(&code, "1", "octocat");
    assert_eq!(settled(found).await.status.code(), Some(0));
}

/// `suru relay login | head -1`: whatever reads the line goes, and the command
/// follows the login to its end all the same. The address and the code share
/// that one line, so whoever read it has both.
#[tokio::test]
async fn a_reader_that_goes_after_the_line_costs_the_login_nothing() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("closed-after-line");
    machine.succeed(&["relay", "add", &address]).await;

    let mut login = machine.begin_login(&address);
    let (_, code) = shown_login(&mut login).await;
    drop(login.stdout.take());
    relay.approve(&code, "1", "octocat");
    let output = settled(login).await;
    assert_eq!(output.status.code(), Some(0), "{}", said(&output.stderr));
    assert!(!said(&output.stderr).contains("panicked"));
}

/// A standard output closed before the line is written: the code still reaches
/// whoever is running this, on standard error, and the login is still followed.
#[tokio::test]
async fn a_standard_output_closed_before_the_line_leaves_it_on_standard_error() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("closed-before-line");
    machine.succeed(&["relay", "add", &address]).await;

    let mut login = machine.begin_login(&address);
    drop::<Option<ChildStdout>>(login.stdout.take());
    let stderr = login.stderr.take().expect("standard error is piped");
    let mut lines = BufReader::new(stderr).lines();
    let code = timeout(PROGRESS_DEADLINE * 2, async {
        loop {
            let line = lines
                .next_line()
                .await
                .expect("read standard error")
                .expect("the command says where to go before it ends");
            if line.starts_with("Visit ") {
                break parsed_login(&line).1;
            }
        }
    })
    .await
    .expect("the line comes");
    relay.approve(&code, "1", "octocat");
    let status = timeout(PROGRESS_DEADLINE * 2, login.wait())
        .await
        .expect("the command ends with the login")
        .expect("wait for suru relay login");
    assert_eq!(status.code(), Some(0));
}

/// A Server stopped by its user takes the login with it, and the command leaves
/// as a Client does, starting no other in its place.
#[tokio::test]
async fn a_server_stopped_mid_follow_ends_the_command_as_a_failure() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("stopped-mid-follow");
    machine.succeed(&["relay", "add", &address]).await;

    let mut login = machine.begin_login(&address);
    shown_login(&mut login).await;
    machine.succeed(&["server", "stop"]).await;
    let output = settled(login).await;
    assert_eq!(
        output.status.code(),
        Some(FAILED),
        "{}",
        said(&output.stderr)
    );
    let told = said(&output.stderr);
    assert!(told.contains("Stopped following the login"), "{told}");
    assert_eq!(
        machine.servers.servers_running(),
        0,
        "none starts in its place"
    );
}

#[tokio::test]
async fn serve_through_says_what_it_does_and_may_be_run_twice() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("serve-through");
    machine.succeed(&["relay", "add", &address]).await;

    // As in `/relay`, the choice is remembered ahead of the login it waits on.
    let on = machine
        .succeed(&["relay", "serve-through", &address, "on"])
        .await;
    assert_eq!(said(&on.stdout), "");
    let told = said(&on.stderr);
    assert!(told.contains("once it is logged in there"), "{told}");
    assert!(told.contains("reachable through this Relay"), "{told}");
    assert_eq!(machine.listing().await["relays"][0]["serve_through"], true);

    let twice = machine
        .succeed(&["relay", "serve-through", &address, "on"])
        .await;
    assert_eq!(machine.listing().await["relays"][0]["serve_through"], true);
    assert!(said(&twice.stderr).contains("reachable through this Relay"));

    let off = machine
        .succeed(&["relay", "serve-through", &address, "off"])
        .await;
    let told = said(&off.stderr);
    assert!(told.contains("no longer Serves through"), "{told}");
    assert!(told.contains("Login"), "says what stays: {told}");
    assert_eq!(machine.listing().await["relays"][0]["serve_through"], false);

    machine
        .exits(
            BAD_COMMAND_LINE,
            &["relay", "serve-through", &address, "maybe"],
        )
        .await;
    let unknown = machine
        .exits(
            FAILED,
            &["relay", "serve-through", "https://elsewhere.example", "on"],
        )
        .await;
    assert!(said(&unknown.stderr).contains("relay add"));
}

#[tokio::test]
async fn removing_a_relay_says_what_became_of_the_login_at_it() {
    let mut relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("removal");
    machine.log_in(&relay, &address).await;

    let removed = machine.succeed(&["relay", "remove", &address]).await;
    assert_eq!(said(&removed.stdout), "");
    let told = said(&removed.stderr);
    assert!(told.contains(&format!("Removed {address}")), "{told}");
    assert_eq!(
        machine.listing().await["relays"],
        json!([]),
        "gone from here"
    );

    // Removing what is not held is told: a mistyped address would otherwise
    // leave the machine logged in where the script took it to have left.
    machine.exits(FAILED, &["relay", "remove", &address]).await;

    machine.log_in(&relay, &address).await;
    relay.stop().await;
    let unconfirmed = machine
        .exits(REMOVAL_UNCONFIRMED, &["relay", "remove", &address])
        .await;
    let told = said(&unconfirmed.stderr);
    assert!(told.contains("the Relay did not answer"), "{told}");
    assert_eq!(machine.listing().await["relays"], json!([]));
}

#[tokio::test]
async fn a_pinned_relay_is_listed_as_pinned_and_refuses_its_removal() {
    let machine = Machine::new("pinned");
    machine.pin(json!({
        "serving": {
            "port": 0,
            "bindAddress": "127.0.0.1",
            "relays": ["https://pinned.example.com"],
        },
    }));

    let listing = machine.listing().await;
    let pinned_in = listing["relays"][0]["pinned_in"]
        .as_str()
        .expect("the document is named")
        .to_owned();
    assert!(pinned_in.ends_with("suru.jsonc"), "{pinned_in}");
    let shown = said(&machine.succeed(&["relay", "list"]).await.stdout);
    assert!(shown.contains(&pinned_in), "{shown}");

    // What a script asked for holds: the Relay is there.
    machine
        .succeed(&["relay", "add", "https://pinned.example.com"])
        .await;
    let refused = machine
        .exits(FAILED, &["relay", "remove", "https://pinned.example.com"])
        .await;
    let told = said(&refused.stderr);
    assert!(told.contains("is pinned by serving.relays"), "{told}");
    assert!(told.contains(&pinned_in), "{told}");
}

#[tokio::test]
async fn an_invite_offers_every_way_unless_told_otherwise() {
    let relay = TestRelay::start().await;
    let other = TestRelay::start().await;
    let (address, other_address) = (relay.address(), other.address());
    let machine = Machine::new("every-way");
    machine.pin(json!({ "serving": { "port": 0 } }));
    machine.log_in(&relay, &address).await;
    machine.log_in(&other, &other_address).await;
    for served in [&address, &other_address] {
        machine
            .succeed(&["relay", "serve-through", served, "on"])
            .await;
    }

    let every = machine.succeed(&["serve", "invite", "--json"]).await;
    let every: Value = serde_json::from_slice(&every.stdout).unwrap();
    let ways = ways_of(&every);
    assert!(ways.contains(&format!("relay {address}")), "{ways:?}");
    assert!(ways.contains(&format!("relay {other_address}")), "{ways:?}");
    let direct = ways
        .iter()
        .filter_map(|way| way.strip_prefix("direct "))
        .map(|way| way.parse::<std::net::SocketAddr>().unwrap())
        .collect::<Vec<_>>();
    assert!(
        direct
            .iter()
            .all(|way| !way.ip().is_loopback() && way.port() != 0),
        "an address another machine could reach, at the port listened on: {direct:?}"
    );

    let no_addresses = machine
        .succeed(&["serve", "invite", "--no-addresses", "--json"])
        .await;
    let no_addresses: Value = serde_json::from_slice(&no_addresses.stdout).unwrap();
    assert_eq!(
        ways_of(&no_addresses),
        vec![format!("relay {address}"), format!("relay {other_address}")]
    );

    // Exactly the Relay asked for, however it is written.
    let written_otherwise = address.replace("http://", "HTTP://") + "/";
    let only = machine
        .succeed(&[
            "serve",
            "invite",
            "--no-addresses",
            "--relay",
            &written_otherwise,
            "--json",
        ])
        .await;
    let only: Value = serde_json::from_slice(&only.stdout).unwrap();
    assert_eq!(ways_of(&only), vec![format!("relay {address}")]);

    let no_relays = machine
        .run(&["serve", "invite", "--no-relays", "--json"])
        .await;
    if direct.is_empty() {
        assert_eq!(no_relays.status.code(), Some(FAILED));
    } else {
        let no_relays: Value = serde_json::from_slice(&no_relays.stdout).unwrap();
        assert!(
            ways_of(&no_relays)
                .iter()
                .all(|way| way.starts_with("direct ")),
            "{no_relays}"
        );
        assert_eq!(no_relays["short_code"], Value::Null, "no joined connection");
    }
}

#[tokio::test]
async fn an_invite_issued_again_replaces_the_one_before_it() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("issued-again");
    machine.log_in(&relay, &address).await;
    machine
        .succeed(&["relay", "serve-through", &address, "on"])
        .await;

    let first = said(&machine.succeed(&["serve", "invite"]).await.stdout);
    let second = said(&machine.succeed(&["serve", "invite"]).await.stdout);
    assert_ne!(first, second);

    let laptop = TestServer::start("issued-again-laptop").await;
    laptop.log_in(&relay, "1", "octocat").await;
    wait_for_a_machine_waiting(&laptop, &address).await;
    let refused = laptop
        .client
        .redeem_invite(RedeemInviteRequest {
            invite: first.trim().into(),
            name: Some("first".to_owned()),
            ways: Vec::new(),
            pair_back: false,
        })
        .await
        .expect_err("the earlier Invite is replaced");
    assert_eq!(
        super::error_code(&refused),
        suru::protocol::SessionErrorCode::InviteSuperseded
    );
    laptop.shutdown().await;
}

#[tokio::test]
async fn asking_for_a_relay_the_server_cannot_offer_fails_naming_why_and_opens_nothing() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("cannot-offer");

    let not_held = machine
        .exits(FAILED, &["serve", "invite", "--relay", &address])
        .await;
    assert_eq!(said(&not_held.stdout), "", "no Invite");
    let told = said(&not_held.stderr);
    assert!(told.contains("holds no Relay at"), "{told}");
    assert!(told.contains("relay add"), "{told}");

    machine.succeed(&["relay", "add", &address]).await;
    let login_needed = machine
        .exits(FAILED, &["serve", "invite", "--relay", &address])
        .await;
    let told = said(&login_needed.stderr);
    assert!(told.contains("Login needed"), "{told}");
    assert!(told.contains("relay login"), "{told}");

    machine.log_in(&relay, &address).await;
    let not_served_through = machine
        .exits(FAILED, &["serve", "invite", "--relay", &address])
        .await;
    let told = said(&not_served_through.stderr);
    assert!(told.contains("does not Serve through"), "{told}");
    assert!(told.contains("relay serve-through"), "{told}");

    let bad = machine
        .exits(
            BAD_COMMAND_LINE,
            &[
                "serve",
                "invite",
                "--relay",
                "https://user:secret@a.example",
            ],
        )
        .await;
    assert!(
        !said(&bad.stderr).contains("secret"),
        "{}",
        said(&bad.stderr)
    );

    // `--no-addresses` with no Relay it can offer has nothing to offer either.
    let nothing = machine
        .exits(FAILED, &["serve", "invite", "--no-addresses"])
        .await;
    assert!(said(&nothing.stderr).contains("no way to offer"));

    assert_eq!(
        machine.listing().await["serving"],
        false,
        "a refusal that could be told beforehand turns nothing on"
    );
}

#[tokio::test]
async fn a_listener_only_this_machine_can_reach_is_not_offered() {
    let machine = Machine::new("loopback-listener");

    let refused = machine
        .exits(FAILED, &["serve", "invite", "--no-relays"])
        .await;
    assert_eq!(said(&refused.stdout), "");
    let told = said(&refused.stderr);
    assert!(told.contains("no way to offer"), "{told}");
    assert!(
        told.contains("127.0.0.1"),
        "names what it listens at: {told}"
    );
    assert!(
        told.contains("Serving is now on"),
        "says what was done on the way: {told}"
    );
    assert_eq!(machine.listing().await["serving"], true);

    let again = machine
        .exits(FAILED, &["serve", "invite", "--no-relays"])
        .await;
    assert!(
        !said(&again.stderr).contains("Serving is now on"),
        "it was on already: {}",
        said(&again.stderr)
    );
}

#[tokio::test]
async fn a_listener_that_cannot_listen_is_said_and_a_relay_is_offered_all_the_same() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let machine = Machine::new("failed-listener");
    machine.log_in(&relay, &address).await;
    machine
        .succeed(&["relay", "serve-through", &address, "on"])
        .await;
    let taken = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = taken.local_addr().unwrap().port();
    machine.pin(json!({ "serving": { "port": port, "bindAddress": "127.0.0.1" } }));
    machine.succeed(&["server", "stop"]).await;

    let refused = machine
        .exits(FAILED, &["serve", "invite", "--no-relays"])
        .await;
    let told = said(&refused.stderr);
    assert!(told.contains("not listening"), "{told}");

    let issued = machine.succeed(&["serve", "invite", "--json"]).await;
    let told = said(&issued.stderr);
    assert!(told.contains("not listening"), "{told}");
    let issued: Value = serde_json::from_slice(&issued.stdout).unwrap();
    assert_eq!(ways_of(&issued), vec![format!("relay {address}")]);
}

/// ADR-0067: a document may say Serving is on while it is not in force. Serving
/// is off, so it is turned on; what the document said on its own does nothing.
#[tokio::test]
async fn an_outside_edit_turning_serving_on_is_not_taken_for_serving_being_on() {
    let machine = Machine::new("outside-edit");
    machine.succeed(&["relay", "list"]).await;
    machine.pin(json!({
        "serving": { "enabled": true, "port": 0, "bindAddress": "127.0.0.1" },
    }));
    assert_eq!(machine.listing().await["serving"], false);

    let refused = machine
        .exits(FAILED, &["serve", "invite", "--no-relays"])
        .await;
    assert!(said(&refused.stderr).contains("Serving is now on"));
    assert_eq!(machine.listing().await["serving"], true);
}

/// ADR-0050: a key the Server cannot read is never made anew, and Serving fails
/// where its user can see, as `/serve` does: by the Server's own words.
#[tokio::test]
async fn an_identity_key_that_cannot_be_read_fails_as_it_does_everywhere() {
    let machine = Machine::new("unreadable-key");
    machine.succeed(&["relay", "list"]).await;
    let turned_on = machine
        .exits(FAILED, &["serve", "invite", "--no-relays"])
        .await;
    assert!(said(&turned_on.stderr).contains("no way to offer"));
    machine.succeed(&["server", "stop"]).await;

    let data_dir = machine.servers.state_dir().join(machine.channel);
    std::fs::remove_file(data_dir.join("server-identity.pk8"))
        .expect("take the key away from under its marker");
    assert!(data_dir.join("server-identity.json").exists());

    // Serving was left on, so this is what the next start finds.
    let refused = machine.exits(FAILED, &["serve", "invite"]).await;
    assert_eq!(said(&refused.stdout), "");
    let told = said(&refused.stderr);
    assert!(told.contains("identity key"), "{told}");
    assert!(!told.contains("Serving is now on"), "{told}");

    // With Serving off to begin with, what is asked for is not taken for done.
    machine.succeed(&["server", "stop"]).await;
    machine.pin(json!({
        "serving": { "enabled": false, "port": 0, "bindAddress": "127.0.0.1" },
    }));
    let refused = machine.exits(FAILED, &["serve", "invite"]).await;
    assert_eq!(said(&refused.stdout), "");
    let told = said(&refused.stderr);
    assert!(told.contains("identity key"), "{told}");
    assert!(told.contains("could not start"), "{told}");
    assert!(!told.contains("Serving is now on"), "{told}");

    let shown = said(&machine.succeed(&["relay", "list"]).await.stderr);
    assert!(
        !shown.contains("identity key"),
        "reading it needs no key: {shown}"
    );
    assert!(
        !data_dir.join("server-identity.pk8").exists(),
        "no key is made in its place"
    );
}

#[tokio::test]
async fn each_command_starts_a_server_as_a_client_would_and_says_so_on_standard_error() {
    let machine = Machine::new("starting");

    let started = machine.succeed(&["relay", "list", "--json"]).await;
    let told = said(&started.stderr);
    assert!(told.contains("Started a Suru server"), "{told}");
    assert!(
        serde_json::from_slice::<Value>(&started.stdout).is_ok(),
        "standard output is the listing alone"
    );

    let attached = machine.succeed(&["relay", "list"]).await;
    assert!(!said(&attached.stderr).contains("Started"));

    // A stop holds a Keeper back, and no Client: one starts a Server again.
    machine.succeed(&["server", "stop"]).await;
    let after_stop = machine.succeed(&["relay", "list"]).await;
    assert!(said(&after_stop.stderr).contains("Started a Suru server"));
}

#[tokio::test]
async fn a_command_line_that_cannot_be_read_exits_as_one_and_help_hides_nothing_it_should_not() {
    let machine = Machine::new("command-line");
    for bad in [
        &["relay"][..],
        &["serve"],
        &["relay", "frobnicate"],
        &["relay", "add"],
        &["relay", "serve-through", "https://relay.example.com"],
        &[
            "serve",
            "invite",
            "--no-relays",
            "--relay",
            "https://a.example",
        ],
        &["serve", "invite", "--no-addresses", "--no-relays"],
        &["serve", "invite", "--nonsense"],
    ] {
        machine.exits(BAD_COMMAND_LINE, bad).await;
    }
    let nothing = machine
        .exits(
            BAD_COMMAND_LINE,
            &["serve", "invite", "--no-addresses", "--no-relays"],
        )
        .await;
    assert!(
        said(&nothing.stderr).contains("no way to offer"),
        "{}",
        said(&nothing.stderr)
    );
    assert_eq!(
        machine.servers.servers_running(),
        0,
        "none of those needed a Server"
    );

    for command in [
        &["--help"][..],
        &["relay", "--help"],
        &["serve", "--help"],
        &["serve", "invite", "--help"],
        &["relay", "login", "--help"],
    ] {
        let help = said(&machine.succeed(command).await.stdout);
        assert!(!help.contains("-ms"), "no test's flag: {help}");
        assert!(!help.contains("__server"), "{help}");
    }
    let top = said(&machine.succeed(&["--help"]).await.stdout);
    assert!(top.contains("relay") && top.contains("serve"), "{top}");
    let relay_help = said(&machine.succeed(&["relay", "--help"]).await.stdout);
    assert!(relay_help.contains("Exit status"), "{relay_help}");
    let serve_help = said(&machine.succeed(&["serve", "invite", "--help"]).await.stdout);
    assert!(serve_help.contains("Exit status"), "{serve_help}");
}

/// What `/serve` asks of the Server as it opens, which the command line shares:
/// Serving turned on where it was off and left alone where it is on, and only the
/// addresses another machine could reach.
#[tokio::test]
async fn the_server_decides_what_preparing_to_issue_an_invite_turns_on_and_lists() {
    let server = TestServer::start("preparation").await;
    for mutation in [
        SettingMutation::ServingPort { value: Some(0) },
        SettingMutation::ServingBindAddress {
            value: Some(std::net::Ipv4Addr::LOCALHOST.into()),
        },
    ] {
        server.client.mutate_setting(mutation).await.unwrap();
    }

    let first = server.client.prepare_serving().await.unwrap();
    assert!(first.turned_on);
    assert!(
        first
            .settings
            .expect("the Setting changed")
            .settings
            .serving
            .enabled
    );
    assert_eq!(
        first.addresses,
        Vec::new(),
        "loopback reaches this machine alone"
    );

    let again = server.client.prepare_serving().await.unwrap();
    assert!(!again.turned_on);
    assert_eq!(again.settings, None, "nothing changed");
    assert!(server.client.serving_standing().await.unwrap().enabled);

    let offered = server
        .client
        .offer_invite(OfferInviteRequest {
            addresses: true,
            relays: InviteRelays::Every,
        })
        .await
        .expect_err("nothing to offer");
    assert_eq!(
        super::error_code(&offered),
        suru::protocol::SessionErrorCode::InvalidInviteWays
    );
    server.shutdown().await;
}

/// Two Clients asking at once share one login, whichever gets there first, and
/// no asking ever ends one under way: ADR-0048 has it that a login is the
/// Server's, so a Client's code never dies by another's opening `/relay`.
#[tokio::test]
async fn clients_asking_for_a_login_at_once_share_it_and_only_afresh_replaces_a_login() {
    let relay = TestRelay::start().await;
    let address = relay.address();
    let server = TestServer::start("login-lead").await;
    server.client.add_relay(address.clone()).await.unwrap();

    let (first, second) = tokio::join!(
        server.client.log_in_at_relay(&address, false),
        server.client.log_in_at_relay(&address, false),
    );
    let code_of = |lead: RelayLoginLead| match lead.standing {
        RelayLoginStanding::Begun { login } => (true, login.user_code),
        RelayLoginStanding::UnderWay { login } => (false, login.user_code),
        RelayLoginStanding::Standing { relay } => panic!("no Login stands yet: {relay:?}"),
    };
    let (first, second) = (code_of(first.unwrap()), code_of(second.unwrap()));
    assert_eq!(first.1, second.1, "one login");
    assert_ne!(first.0, second.0, "begun once");

    // Nor does asking afresh end one under way.
    let afresh = server.client.log_in_at_relay(&address, true).await.unwrap();
    assert_eq!(code_of(afresh), (false, first.1.clone()));

    relay.approve(&first.1, "1", "octocat");
    let ended = server.client.follow_relay_login(&address).await.unwrap();
    assert!(matches!(ended.outcome, RelayLoginOutcome::Done { .. }));

    let standing = server
        .client
        .log_in_at_relay(&address, false)
        .await
        .unwrap();
    let RelayLoginStanding::Standing { relay: stands } = standing.standing else {
        panic!("a Login stands, and none is begun: {standing:?}");
    };
    assert_eq!(stands.account.unwrap().username, "octocat");

    // To stand under another Account a reader asks for one where a Login stands.
    let afresh = server.client.log_in_at_relay(&address, true).await.unwrap();
    let (begun, code) = code_of(afresh);
    assert!(begun);
    relay.approve(&code, "2", "hubot");
    let ended = server.client.follow_relay_login(&address).await.unwrap();
    assert!(matches!(ended.outcome, RelayLoginOutcome::Done { .. }));
    assert_eq!(
        server
            .relay(&address)
            .await
            .unwrap()
            .account
            .unwrap()
            .username,
        "hubot"
    );
    server.shutdown().await;
}

/// The one way to begin a login is the one that cannot end another.
#[tokio::test]
async fn no_route_begins_a_login_regardless() {
    let machine = Machine::new("no-other-route");
    machine
        .succeed(&["relay", "add", "https://relay.example.com"])
        .await;
    let descriptor = crate::support::read_runtime_descriptor(
        machine
            .servers
            .state_dir()
            .join(machine.channel)
            .join("runtime.json"),
    );
    let begun = crate::support::loopback_client()
        .post(format!(
            "{}/v1/relays/https:%2F%2Frelay.example.com/login",
            descriptor.base_url
        ))
        .bearer_auth(&descriptor.token)
        .send()
        .await
        .unwrap();
    assert_eq!(begun.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
}
