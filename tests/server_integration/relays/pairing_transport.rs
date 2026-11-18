//! A Client with no Server of its own reading a Serving Server over a
//! Pairing: on one side a real Serving Server in process, and on the other
//! nothing but the redeemer, its records in a directory of their own, and
//! `shaman-client`'s readers and commands over the Pairing transport — as a
//! thin Client reads each of its Remotes. It is reached by a direct way and
//! through a Relay, and outlives the Serving Server restarting, its Peer
//! removed and its way cut as a reader would want.

use std::{path::Path, sync::Arc, time::Duration};

use rcgen::PublicKeyData as _;
use shaman::protocol::{
    Activity, AdmitPromptRequest, AgentId, AgentIdentity, AgentSelection, Approval, ApprovalId,
    ApprovalSubject, CreateSessionRequest, Decision, ExecutionDirectory, InitialPrompt, ModelId,
    Outlook, PROTOCOL_VERSION, Prompt, PromptDelivery, PromptId, ProviderId, RedeemInviteRequest,
    RelayLoginOutcome, Remote, RemoteStatus, SessionChange, SessionCreated, SessionDeleted,
    SessionError, SessionErrorCode, SessionId, SessionSnapshot, Way,
};
use shaman::provider::ProviderEvent;
use shaman_client::{
    PairingTransport, SessionCatalogEvent, SessionCatalogSubscription, SessionCommandTimings,
    SessionCommands, SessionEvent, SessionSubscription,
};
use shaman_pairing::{
    GivenIdentity, Identity, IdentityMaterial, Redeemer, RedeemerTimings,
    relay::{LoginStanding, RelayClient, RelayClientTimings, Unattached},
    signer::{IdentitySigner, SoftwareSigner},
};
use tokio::time::timeout;

use super::{TestRelay, TestServer, relay_timings};
use crate::support::{
    PROGRESS_DEADLINE,
    observed_tcp_proxy::ObservedTcpProxy,
    provider::{ControlledProvider, ControlledProviderSession},
};

/// The name the Client knows the Serving Server by.
const REMOTE: &str = "workstation";

/// A reader's backoff where the test waits on it: short enough to be no
/// wait at all.
const QUICK: Duration = Duration::from_millis(5);

/// A reader's backoff where the test must show it is never waited out.
const HOUR: Duration = Duration::from_secs(3600);

/// The Client side: nothing but a redeemer and the Relay entries it reaches
/// Relay ways through, both kept in one directory as a thin Client keeps
/// them, proving one identity key.
struct NoServer {
    directory: tempfile::TempDir,
    /// The identity key, as a test speaks to a Relay with it.
    key: rcgen::KeyPair,
    redeemer: Redeemer,
    relays: RelayClient<Unattached>,
}

impl NoServer {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("create the Client's directory");
        let key = rcgen::KeyPair::generate().expect("make the identity key");
        let signer = SoftwareSigner::from_pkcs8(&key.serialize_der()).expect("take up the key");
        let identity: Arc<dyn Identity> = Arc::new(GivenIdentity(
            IdentityMaterial::minted(IdentitySigner::new(signer).expect("sign with the key"))
                .expect("mint the key's certificate"),
        ));
        let relays = RelayClient::new(
            directory.path(),
            identity.clone(),
            RelayClientTimings::default(),
            Unattached,
        )
        .expect("open the Relay entries");
        let redeemer = Redeemer::new(
            directory.path(),
            PROTOCOL_VERSION,
            identity,
            RedeemerTimings::default(),
        )
        .expect("open the Remotes' records");
        redeemer.reach_relays_through(Arc::new(relays.joiner()));
        Self {
            directory,
            key,
            redeemer,
            relays,
        }
    }

    /// Adds `relay` and logs in there as the identity the Serving Server's
    /// user logs in as, so the two stand under one Account.
    async fn log_in(&self, relay: &TestRelay) {
        let address = relay.address();
        self.relays.add(&address, |_| ()).expect("add the Relay");
        let lead = self
            .relays
            .log_in(&address, false, |_| ())
            .await
            .expect("begin a login at the Relay");
        let LoginStanding::Begun(login) = lead.standing else {
            panic!("a login is begun where none stands");
        };
        let mut progress = self
            .relays
            .follow_login(&address)
            .expect("follow the login");
        relay.approve(&login.user_code, "583231", "octocat");
        let done = timeout(
            PROGRESS_DEADLINE,
            progress.wait_for(|login| login.outcome.is_settled()),
        )
        .await
        .expect("the login ends in time")
        .expect("the login is followed to its end")
        .outcome
        .clone();
        assert!(
            matches!(done, RelayLoginOutcome::Done { .. }),
            "the login forms a Login: {done:?}"
        );
    }

    /// Redeems `invite`, naming the Remote [`REMOTE`].
    async fn redeem(&self, invite: String) -> Remote {
        self.redeemer
            .redeem_invite(RedeemInviteRequest {
                invite: invite.into(),
                name: Some(REMOTE.to_owned()),
                ways: Vec::new(),
                pair_back: false,
            })
            .await
            .unwrap_or_else(|failure| panic!("the Invite is redeemed: {}", failure.message))
    }

    /// The Pairing transport to the Remote.
    fn transport(&self) -> Arc<PairingTransport> {
        Arc::new(PairingTransport::new(self.redeemer.clone(), REMOTE))
    }

    /// The fingerprint the Serving Server lists this Client's Peer record by.
    fn fingerprint(&self) -> String {
        shaman_relay_protocol::fingerprint(&self.key.subject_public_key_info())
    }

    async fn shutdown(self) {
        self.redeemer.shutdown().await;
        self.relays.shutdown();
        drop(self.directory);
    }
}

/// The commands and readers over `transport`, each reader waiting `backoff`
/// between one failed attempt and the next.
fn commands_over(transport: Arc<PairingTransport>, backoff: Duration) -> SessionCommands {
    SessionCommands::new(
        transport,
        SessionCommandTimings {
            initial_recovery_backoff: backoff,
            max_recovery_backoff: backoff,
            attachment_fetch_timeout: PROGRESS_DEADLINE,
        },
    )
}

/// A Session to be begun in `workspace`.
fn new_session(workspace: &Path, text: &str) -> CreateSessionRequest {
    CreateSessionRequest {
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
    }
}

/// The catalog's next event, past what it pushes beside the catalog itself.
async fn next_catalog_event(catalog: &mut SessionCatalogSubscription) -> SessionCatalogEvent {
    timeout(PROGRESS_DEADLINE, async {
        loop {
            match catalog.next().await {
                Some(
                    SessionCatalogEvent::ModelCatalog(_)
                    | SessionCatalogEvent::WorkspaceListing(_)
                    | SessionCatalogEvent::SkillCatalogUpdated(_),
                ) => {}
                Some(event) => return event,
                None => panic!("the catalog ended"),
            }
        }
    })
    .await
    .expect("the catalog says something in time")
}

/// Reads the catalog until `wanted` says of an event, answering it.
async fn catalog_until(
    catalog: &mut SessionCatalogSubscription,
    what: &str,
    wanted: impl Fn(&SessionCatalogEvent) -> bool,
) -> SessionCatalogEvent {
    timeout(PROGRESS_DEADLINE, async {
        loop {
            let event = next_catalog_event(catalog).await;
            if wanted(&event) {
                return event;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the catalog never said {what}"))
}

/// Reads the Session's stream until a change `wanted` says of arrives.
async fn change_until(
    stream: &mut SessionSubscription,
    what: &str,
    wanted: impl Fn(&SessionChange) -> bool,
) {
    timeout(PROGRESS_DEADLINE, async {
        loop {
            match stream.next().await {
                Some(Ok(SessionEvent::Updated(update))) if update.changes.iter().any(&wanted) => {
                    return;
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => panic!("the Session's stream failed: {error}"),
                None => panic!("the Session's stream ended"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the Session's stream never said {what}"));
}

/// Reads the Session's stream until it opens afresh with a snapshot.
async fn snapshot_until(stream: &mut SessionSubscription) -> SessionSnapshot {
    timeout(PROGRESS_DEADLINE, async {
        loop {
            match stream.next().await {
                Some(Ok(SessionEvent::Snapshot(snapshot))) => return *snapshot,
                Some(Ok(SessionEvent::Updated(_))) => {}
                Some(Err(error)) => panic!("the Session's stream failed: {error}"),
                None => panic!("the Session's stream ended"),
            }
        }
    })
    .await
    .expect("the Session's stream opens with a snapshot in time")
}

/// What the Session's stream ends with, past what it read before.
async fn stream_ending(stream: &mut SessionSubscription) -> shaman_client::SessionStreamError {
    let failure = timeout(PROGRESS_DEADLINE, async {
        loop {
            match stream.next().await {
                Some(Ok(_)) => {}
                Some(Err(error)) => return error,
                None => panic!("the Session's stream ended without saying why"),
            }
        }
    })
    .await
    .expect("the Session's stream ends in time");
    assert!(
        timeout(PROGRESS_DEADLINE, stream.next())
            .await
            .expect("the stream ends in time")
            .is_none(),
        "the stream is not opened again"
    );
    failure
}

/// The code a command's refusal carries.
fn refusal_code(error: &anyhow::Error) -> SessionErrorCode {
    error
        .downcast_ref::<SessionError>()
        .unwrap_or_else(|| panic!("a typed Session error, not {error:#}"))
        .code
}

/// A Serving Server whose Sessions run on a Provider the test drives.
async fn driven_serving_server(channel: &str) -> (TestServer, ControlledProvider) {
    let (runtime, provider) = ControlledProvider::new();
    let server = TestServer::on_provider(channel, relay_timings(), runtime).await;
    server.serve().await;
    (server, provider)
}

/// Lists, reads live, prompts, answers an Approval and interrupts a Session
/// on the Remote, every request over `commands` alone, with `provider` the
/// Provider the Remote's Sessions run on.
async fn read_and_act(commands: &SessionCommands, provider: &mut ControlledProvider) {
    let workspace = tempfile::tempdir().expect("create a Workspace on the Serving Server");
    assert!(
        commands
            .list_sessions(None)
            .await
            .expect("list the Remote's Sessions")
            .is_empty()
    );

    let created = commands
        .create_session(new_session(workspace.path(), "Map the workspace"))
        .await
        .expect("begin a Session on the Remote");
    let session_id = created.session.id;
    let mut provider_session = begin_turn(provider).await;
    let listed = commands
        .list_sessions(None)
        .await
        .expect("list the Remote's Sessions");
    assert_eq!(
        listed.iter().map(|item| item.id()).collect::<Vec<_>>(),
        [session_id],
        "the Session begun is listed"
    );

    let mut stream = commands
        .attach_session(session_id)
        .await
        .expect("read the Session live");
    assert_eq!(snapshot_until(&mut stream).await.session.id, session_id);

    let admitted: Prompt = commands
        .admit_prompt(
            session_id,
            AdmitPromptRequest {
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: "Then the tests".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
                delivery: PromptDelivery::Queue,
            },
        )
        .await
        .expect("send a Prompt");
    change_until(&mut stream, "the Prompt was added", |change| {
        matches!(change, SessionChange::PromptAdded { prompt } if prompt.id == admitted.id)
    })
    .await;

    let approval = Approval {
        id: ApprovalId::new(),
        subject: ApprovalSubject::Command {
            command: "cargo nextest run".into(),
            cwd: None,
            actions: Vec::new(),
        },
        reason: Some("Run the tests".into()),
    };
    provider_session.emit(ProviderEvent::ApprovalRequested {
        approval: approval.clone(),
        tool_activity_id: None,
    });
    change_until(&mut stream, "the Approval was asked", |change| {
        matches!(
            change,
            SessionChange::ActivityAdded {
                activity: Activity::Approval { approval: asked, .. },
            } if asked.id == approval.id
        )
    })
    .await;
    commands
        .submit_decision(session_id, approval.id, Decision::Accept)
        .await
        .expect("answer the Approval");
    assert_eq!(
        timeout(PROGRESS_DEADLINE, provider_session.next_decision())
            .await
            .expect("the Decision reaches the Provider"),
        (approval.id, Decision::Accept)
    );

    let (interrupted, ()) = tokio::join!(commands.interrupt_session(session_id), async {
        timeout(PROGRESS_DEADLINE, provider_session.next_interrupt())
            .await
            .expect("the interrupt reaches the Provider")
            .succeed();
    });
    interrupted.expect("the Turn is interrupted");
}

/// Has the Provider take up the Session begun last and its first Turn,
/// answering the Provider's side of it.
async fn begin_turn(provider: &mut ControlledProvider) -> ControlledProviderSession {
    let start = timeout(PROGRESS_DEADLINE, provider.next_start())
        .await
        .expect("the Provider is asked to begin the Session");
    let mut provider_session = start.succeed(AgentIdentity {
        agent: AgentId::new("controlled-agent"),
        selection: AgentSelection {
            provider: ProviderId::new("controlled"),
            model: ModelId::new("controlled-model"),
            options: Vec::new(),
        },
    });
    timeout(PROGRESS_DEADLINE, provider_session.next_turn())
        .await
        .expect("the first Turn reaches the Provider")
        .succeed();
    provider_session
}

#[tokio::test]
async fn a_client_with_no_server_redeems_reads_and_acts_on_a_remote_by_a_direct_way() {
    let (workstation, mut provider) = driven_serving_server("thin-pairing-direct").await;
    let client = NoServer::new();
    let invite = workstation
        .invite(vec![Way::Direct(workstation.serving_address())])
        .await;

    let remote = client.redeem(invite).await;
    assert_eq!(remote.name, REMOTE);
    assert_eq!(
        workstation
            .client
            .list_peers()
            .await
            .expect("list the Serving Server's Peers")
            .len(),
        1,
        "the Client is paired as an ordinary Peer"
    );
    let commands = commands_over(client.transport(), QUICK);
    read_and_act(&commands, &mut provider).await;

    client.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_client_with_no_server_redeems_reads_and_acts_on_a_remote_through_a_relay() {
    let relay = TestRelay::start().await;
    let (workstation, mut provider) = driven_serving_server("thin-pairing-relay").await;
    let client = NoServer::new();
    workstation.log_in(&relay, "583231", "octocat").await;
    client.log_in(&relay).await;
    workstation.serve_through(&relay, true).await;
    // Joined there once, as the Client, the Serving Server is known to wait
    // at the Relay.
    relay
        .voice()
        .joined(
            &client.key,
            &workstation.identity().subject_public_key_info(),
        )
        .await;
    let invite = workstation.invite(vec![Way::Relay(relay.address())]).await;

    let remote = client.redeem(invite).await;
    assert_eq!(remote.ways, [Way::Relay(relay.address())]);
    let commands = commands_over(client.transport(), QUICK);
    read_and_act(&commands, &mut provider).await;

    client.shutdown().await;
    workstation.shutdown().await;
}

/// A Serving Server, and a Client with no Server paired with it by a direct
/// way the test can cut and point at the Serving Server again once it
/// restarts at another port.
struct PairedDirectly {
    workstation: TestServer,
    route: ObservedTcpProxy,
    client: NoServer,
    transport: Arc<PairingTransport>,
}

impl PairedDirectly {
    async fn start(channel: &str) -> Self {
        let workstation = TestServer::start(channel).await;
        workstation.serve().await;
        let route = ObservedTcpProxy::start(workstation.serving_address()).await;
        let client = NoServer::new();
        client
            .redeem(workstation.invite(vec![Way::Direct(route.address)]).await)
            .await;
        let transport = client.transport();
        Self {
            workstation,
            route,
            client,
            transport,
        }
    }

    /// A Session begun on the Serving Server, whose Provider fails its start
    /// and leaves it standing idle.
    async fn begin_session(&self, commands: &SessionCommands, workspace: &Path) -> SessionId {
        commands
            .create_session(new_session(workspace, "Map the workspace"))
            .await
            .expect("begin a Session on the Remote")
            .session
            .id
    }

    async fn shutdown(self) {
        self.client.shutdown().await;
        self.workstation.shutdown().await;
    }
}

#[tokio::test]
async fn a_stream_over_a_pairing_resumes_by_snapshot_once_the_serving_server_restarts() {
    let mut paired = PairedDirectly::start("thin-pairing-restart").await;
    let workspace = tempfile::tempdir().expect("create a Workspace on the Serving Server");
    let commands = commands_over(paired.transport.clone(), QUICK);
    let session_id = paired.begin_session(&commands, workspace.path()).await;
    let mut stream = commands
        .attach_session(session_id)
        .await
        .expect("read the Session live");
    let before = snapshot_until(&mut stream).await;
    let mut catalog = commands.subscribe_catalog();
    assert!(matches!(
        next_catalog_event(&mut catalog).await,
        SessionCatalogEvent::SessionCatalogReconciled(_)
    ));

    paired.workstation.restart().await;
    paired.route.retarget(paired.workstation.serving_address());

    let after = snapshot_until(&mut stream).await;
    assert_eq!(after.session.id, session_id);
    assert!(
        after.revision >= before.revision,
        "the snapshot it resumes by is the Session as it stands now"
    );
    catalog_until(&mut catalog, "it recovered", |event| {
        matches!(event, SessionCatalogEvent::RemoteRecovered)
    })
    .await;
    assert_eq!(
        commands
            .list_sessions(None)
            .await
            .expect("list the restarted Remote's Sessions")
            .iter()
            .map(|item| item.id())
            .collect::<Vec<_>>(),
        [session_id]
    );

    drop((stream, catalog));
    paired.shutdown().await;
}

#[tokio::test]
async fn a_stream_over_a_pairing_ends_as_revoked_once_its_peer_is_removed() {
    let paired = PairedDirectly::start("thin-pairing-revoked").await;
    let workspace = tempfile::tempdir().expect("create a Workspace on the Serving Server");
    let commands = commands_over(paired.transport.clone(), QUICK);
    let session_id = paired.begin_session(&commands, workspace.path()).await;
    let mut stream = commands
        .attach_session(session_id)
        .await
        .expect("read the Session live");
    snapshot_until(&mut stream).await;
    let mut catalog = commands.subscribe_catalog();
    assert!(matches!(
        next_catalog_event(&mut catalog).await,
        SessionCatalogEvent::SessionCatalogReconciled(_)
    ));

    paired
        .workstation
        .client
        .remove_peer(&paired.client.fingerprint())
        .await
        .expect("remove the Client's Peer record");

    let ended = stream_ending(&mut stream).await;
    assert_eq!(
        ended.remote_status(),
        Some(RemoteStatus::Revoked),
        "{ended}"
    );
    assert!(!ended.is_recoverable());
    assert!(matches!(
        catalog_until(&mut catalog, "the Remote failed", |event| matches!(
            event,
            SessionCatalogEvent::RemoteFailed { .. }
        ))
        .await,
        SessionCatalogEvent::RemoteFailed {
            status: RemoteStatus::Revoked,
            ..
        }
    ));
    let refused = commands
        .list_sessions(None)
        .await
        .expect_err("the Pairing no longer stands");
    assert_eq!(
        refusal_code(&refused),
        SessionErrorCode::PairingAuthenticationFailed
    );

    paired.shutdown().await;
}

#[tokio::test]
async fn a_cut_way_reads_unreachable_until_it_answers_and_try_now_skips_the_backoff() {
    let mut paired = PairedDirectly::start("thin-pairing-cut").await;
    let workspace = tempfile::tempdir().expect("create a Workspace on the Serving Server");
    // Every reader waits an hour before trying again, so only being told to
    // try now brings one back within the test.
    let commands = commands_over(paired.transport.clone(), HOUR);
    let session_id = paired.begin_session(&commands, workspace.path()).await;
    let mut stream = commands
        .attach_session(session_id)
        .await
        .expect("read the Session live");
    snapshot_until(&mut stream).await;
    let mut catalog = commands.subscribe_catalog();
    assert!(matches!(
        next_catalog_event(&mut catalog).await,
        SessionCatalogEvent::SessionCatalogReconciled(_)
    ));

    paired.route.set_online(false).await;

    let SessionCatalogEvent::Recovering(recovering) = next_catalog_event(&mut catalog).await else {
        panic!("the catalog reads the Remote as out of reach");
    };
    assert_eq!(recovering.retry_in, HOUR);
    let refused = commands
        .list_sessions(None)
        .await
        .expect_err("no way reaches the Remote");
    assert_eq!(
        refusal_code(&refused),
        SessionErrorCode::PairingConnectionFailed,
        "the failure to reach comes back in the wire's terms: {refused:#}"
    );

    paired.route.set_online(true).await;
    paired.transport.say_try_now();

    catalog_until(&mut catalog, "it recovered", |event| {
        matches!(event, SessionCatalogEvent::RemoteRecovered)
    })
    .await;
    assert_eq!(
        snapshot_until(&mut stream).await.session.id,
        session_id,
        "the Session's stream, told too, resumes by snapshot"
    );

    drop((stream, catalog));
    paired.shutdown().await;
}

/// What every reader does whatever carries its requests: the catalog's
/// first snapshot holds what stood before it opened and later Sessions are
/// announced, and a Session's stream opens with its snapshot and, the
/// Session deleted, ends saying so.
async fn reader_behaviours(commands: &SessionCommands, workspace: &Path) {
    let first = commands
        .create_session(new_session(workspace, "The first"))
        .await
        .expect("begin a Session")
        .session
        .id;
    let mut catalog = commands.subscribe_catalog();
    let SessionCatalogEvent::SessionCatalogReconciled(snapshot) =
        next_catalog_event(&mut catalog).await
    else {
        panic!("the catalog opens with its snapshot");
    };
    assert!(
        snapshot.session_ids.contains(&first),
        "the Session begun before the catalog opened is in its snapshot"
    );

    let second = commands
        .create_session(new_session(workspace, "The second"))
        .await
        .expect("begin another Session")
        .session
        .id;
    catalog_until(&mut catalog, "the second Session was created", |event| {
        *event == SessionCatalogEvent::SessionCreated(SessionCreated { session_id: second })
    })
    .await;

    let mut stream = commands
        .attach_session(second)
        .await
        .expect("read the second Session live");
    assert_eq!(snapshot_until(&mut stream).await.session.id, second);
    commands
        .delete_session(second)
        .await
        .expect("delete the second Session");
    catalog_until(&mut catalog, "the second Session was deleted", |event| {
        *event == SessionCatalogEvent::SessionDeleted(SessionDeleted { session_id: second })
    })
    .await;
    let ended = stream_ending(&mut stream).await;
    assert_eq!(ended.deleted_session(), Some(second), "{ended}");
}

#[tokio::test]
async fn the_readers_behave_alike_over_loopback_and_over_a_pairing() {
    let paired = PairedDirectly::start("thin-pairing-alike").await;

    let over_loopback = tempfile::tempdir().expect("create a Workspace on the Serving Server");
    reader_behaviours(
        &paired
            .workstation
            .client
            .session_commands_for(Outlook::Local),
        over_loopback.path(),
    )
    .await;
    let over_pairing = tempfile::tempdir().expect("create a Workspace on the Serving Server");
    reader_behaviours(
        &commands_over(paired.transport.clone(), QUICK),
        over_pairing.path(),
    )
    .await;

    paired.shutdown().await;
}
