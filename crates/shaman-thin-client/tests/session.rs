//! An open Session over a real Serving Server in process: framed at once by
//! what the listing knows, its conversation landing in one piece once the
//! Session has been read and changing by item after, the Outlook turned
//! toward it, and nothing on the Server cancelled when it closes; and its
//! own Approvals and Questionnaires presented, dismissed, reviewed and
//! answered.

mod support;

use shaman::protocol::{
    Activity, Answer, Approval, ApprovalId, ApprovalOutcome, ApprovalSubject, Decision,
    PromptDelivery, Question, QuestionAnswer, QuestionChoice, Questionnaire, QuestionnaireId,
    QuestionnaireOutcome, QuestionnaireSubmission, SessionId, SessionStatus, TurnStatus, Way,
};
use shaman_thin_client::{
    ActError, ActiveRow, BarKind, BodyStyle, ConversationChange, ConversationItem, DecisionKind,
    GivenAnswer, Intervention, ItemKind, Outlook, PairingOutcome, PendingApproval,
    PendingQuestionnaire, SessionFrame, Tap, Tone, TurnOutcome, WorkKind,
};

use shaman::provider::{ProviderActivityId, ProviderCommandStatus, ProviderEvent};
use support::{Gate, Phone, SCRIPTED, Script, ServingServer, WORKING};

/// Has the scripted Session's Provider send `events`, in order.
fn say(script: &Script, events: impl IntoIterator<Item = ProviderEvent>) {
    for event in events {
        script
            .send(Ok(event.into()))
            .expect("the scripted Session hears");
    }
}

fn cards(items: &[ConversationItem]) -> Vec<shaman_thin_client::TurnCard> {
    items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Turn { card } => Some(card.clone()),
            _ => None,
        })
        .collect()
}

async fn pair(phone: &Phone, server: &ServingServer, name: &str) {
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
    let paired = phone
        .core
        .confirm_pairing(invite, name.to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(
        paired,
        PairingOutcome::Paired {
            name: name.to_owned()
        }
    );
}

fn workspace(holder: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let path = holder.path().join(name);
    std::fs::create_dir_all(&path).expect("make the Workspace");
    path
}

/// The row the listing shows for the Session titled `title`, once it has
/// settled into a Standing.
async fn row(phone: &Phone, title: &str) -> ActiveRow {
    let listing = phone
        .listing_until(|listing| {
            listing
                .active
                .iter()
                .any(|row| row.title == title && row.standing.is_some())
        })
        .await;
    listing
        .active
        .into_iter()
        .find(|row| row.title == title)
        .expect("listed")
}

/// What each item is, in a word and its words, in the order they stand.
fn read_as(items: &[ConversationItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match &item.kind {
            ItemKind::Loading { .. } => "loading".to_owned(),
            ItemKind::UserMessage { text, .. } => format!("user: {text}"),
            ItemKind::Delegation { text, from } => format!("delegation from {from}: {text}"),
            ItemKind::AgentMessage { markdown, .. } => format!("agent: {markdown}"),
            ItemKind::Turn { card } => format!("card: {}", card.line),
            ItemKind::Error { text } => format!("error: {text}"),
            ItemKind::Compaction { words, .. } => format!("compaction: {words}"),
            ItemKind::Refused { words, .. } => format!("refused: {words}"),
        })
        .collect()
}

fn loaded(items: &[ConversationItem]) -> bool {
    !items
        .iter()
        .any(|item| matches!(item.kind, ItemKind::Loading { .. }))
}

/// Whether the Session's Turn has failed, its Error standing.
fn failed_turn(items: &[ConversationItem]) -> bool {
    items
        .iter()
        .any(|item| matches!(item.kind, ItemKind::Error { .. }))
}

fn session_id(reference: &str) -> SessionId {
    let (_, id) = reference.split_once('/').expect("a reference");
    serde_json::from_value(serde_json::Value::String(id.to_owned())).expect("a Session id")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_opens_framed_by_the_listing_and_its_conversation_lands_in_one_piece() {
    let desk = ServingServer::start("thin-open-failed").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let row = row(&phone, "Map the workspace").await;

    let reading = phone.open(&row.session);
    let frame = reading.frame().await;
    assert_eq!(
        frame.title, "Map the workspace",
        "at once, from the listing"
    );
    assert_eq!(
        frame.place, "shaman",
        "with one Remote no Origin is named, and outside source control no branch"
    );
    assert_eq!(frame.gone, None);

    let conversation = reading.conversation_until(loaded).await;
    let [first, landed, ..] = &conversation.told[..] else {
        panic!(
            "told the loading mark whole and then the conversation: {:?}",
            conversation.told
        );
    };
    assert!(
        matches!(first, ConversationChange::Whole { items } if !loaded(items)),
        "{first:?}"
    );
    assert!(
        matches!(landed, ConversationChange::Changed { items, order } if items.len() == order.len()),
        "the whole conversation lands at once, in place of the loading mark: {landed:?}"
    );
    let snapshot = desk.read(session_id(&row.session)).await;
    assert_eq!(snapshot.turns[0].status, TurnStatus::Failed);
    let error = snapshot
        .activities
        .iter()
        .find_map(|activity| match activity {
            shaman::protocol::Activity::Error { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("the failed Turn's Error");
    assert_eq!(
        read_as(&conversation.items),
        [
            "user: Map the workspace".to_owned(),
            format!("error: {error}"),
        ],
        "a failed Turn that did no work is its Prompt and its Error"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_turn_reads_as_it_works_its_answer_streaming_and_settles_into_its_card() {
    let desk = ServingServer::start("thin-open-live").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(
        &workspace(&holder, SCRIPTED),
        "Draw the invite as a QR code",
    )
    .await;
    let script = desk.script().await;
    let row = row(&phone, "Draw the invite as a QR code").await;
    let reading = phone.open(&row.session);
    let command = ProviderActivityId::new("qr");
    say(
        &script,
        [
            ProviderEvent::AgentMessageStarted,
            ProviderEvent::AgentMessageDelta {
                content: "Looking at the overlay first.".to_owned(),
            },
            ProviderEvent::AgentMessageCompleted,
            ProviderEvent::CommandStarted {
                activity_id: command.clone(),
                command: "cargo test qr".to_owned(),
                cwd: None,
            },
        ],
    );

    let working = reading
        .conversation_until(|items| cards(items).iter().any(|card| card.line == "cargo test qr"))
        .await;
    assert_eq!(
        read_as(&working.items),
        ["user: Draw the invite as a QR code", "card: cargo test qr"],
        "the Agent's Message along the way is work behind the card"
    );
    let card = &cards(&working.items)[0];
    assert_eq!(card.outcome, TurnOutcome::Working);
    assert_eq!(card.count.as_deref(), Some("1 command"));

    say(
        &script,
        [
            ProviderEvent::CommandCompleted {
                activity_id: command,
                status: ProviderCommandStatus::Completed,
                exit_status: Some(0),
            },
            ProviderEvent::AgentMessageStarted,
            ProviderEvent::AgentMessageDelta {
                content: "The code is".to_owned(),
            },
        ],
    );
    let streaming = reading
        .conversation_until(|items| {
            read_as(items)
                .last()
                .is_some_and(|last| last == "agent: The code is")
        })
        .await;
    let told = streaming.told.len();
    say(
        &script,
        [ProviderEvent::AgentMessageDelta {
            content: " drawn in half blocks.".to_owned(),
        }],
    );
    let streamed = reading
        .conversation_until(|items| {
            read_as(items)
                .last()
                .is_some_and(|last| last == "agent: The code is drawn in half blocks.")
        })
        .await;
    let ConversationChange::Changed { items, order } = &streamed.told[told] else {
        panic!("told the change alone: {:?}", streamed.told[told]);
    };
    assert_eq!(
        items.len(),
        1,
        "a streaming Message crosses alone: {items:?}"
    );
    assert_eq!(order.len(), 3);
    let ItemKind::AgentMessage { streaming: on, .. } = &items[0].kind else {
        panic!("the agent Message");
    };
    assert!(on, "still arriving");

    say(
        &script,
        [
            ProviderEvent::AgentMessageCompleted,
            ProviderEvent::TurnCompleted,
        ],
    );
    let settled = reading
        .conversation_until(|items| {
            cards(items)
                .first()
                .is_some_and(|card| card.outcome == TurnOutcome::Worked)
        })
        .await;
    assert_eq!(
        read_as(&settled.items)[..2],
        ["user: Draw the invite as a QR code".to_owned(), {
            let line = &cards(&settled.items)[0].line;
            assert!(line.starts_with("Worked for "), "{line}");
            format!("card: {line}")
        }]
    );
    assert_eq!(
        read_as(&settled.items)[2],
        "agent: The code is drawn in half blocks."
    );
    assert_eq!(cards(&settled.items)[0].count.as_deref(), Some("1 command"));

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turns_work_and_a_running_commands_detail_cross_while_their_screens_are_up() {
    let desk = ServingServer::start("thin-open-work").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, SCRIPTED), "Build the relay")
        .await;
    let script = desk.script().await;
    let row = row(&phone, "Build the relay").await;
    let reading = phone.open(&row.session);
    let build = ProviderActivityId::new("build");
    say(
        &script,
        [
            ProviderEvent::AgentMessageStarted,
            ProviderEvent::AgentMessageDelta {
                content: "Building first.".to_owned(),
            },
            ProviderEvent::AgentMessageCompleted,
            ProviderEvent::CommandStarted {
                activity_id: build.clone(),
                command: "cargo build -p shaman-relay".to_owned(),
                cwd: None,
            },
            ProviderEvent::CommandOutputDelta {
                activity_id: build.clone(),
                content: "   Compiling shaman-relay\n".to_owned(),
            },
        ],
    );
    let working = reading
        .conversation_until(|items| {
            cards(items)
                .iter()
                .any(|card| card.line == "cargo build -p shaman-relay")
        })
        .await;
    let card = &cards(&working.items)[0];

    // The card pushes the Turn's work.
    let work = reading.work(&card.work);
    let listed = work.until(|work| work.items.len() == 2).await;
    assert_eq!(listed.title, "Working");
    let WorkKind::AgentMessage { markdown, .. } = &listed.items[0].kind else {
        panic!("the Agent's Message along the way: {listed:?}");
    };
    assert_eq!(markdown, "Building first.");
    let WorkKind::Row { row: command } = &listed.items[1].kind else {
        panic!("the command's row: {listed:?}");
    };
    assert_eq!(command.title, "cargo build -p shaman-relay");
    assert_eq!(command.tone, Tone::Running);
    let Tap::Detail { activity } = &command.tap else {
        panic!("the command pushes its detail");
    };

    // Its row pushes the command's detail, whose output grows while it is up.
    let detail = reading.detail(activity);
    let output = |detail: &shaman_thin_client::ActivityDetail| {
        detail
            .sections
            .iter()
            .find(|section| section.heading.as_deref() == Some("Output"))
            .map(|section| section.body.clone())
    };
    let first = detail
        .until(|detail| output(detail).is_some_and(|output| output.contains("Compiling")))
        .await;
    assert_eq!(first.title, "Command");
    assert_eq!(
        first.sections[0].style,
        BodyStyle::Monospace,
        "output keeps its lines"
    );
    say(
        &script,
        [ProviderEvent::CommandOutputDelta {
            activity_id: build.clone(),
            content: "    Finished `dev` profile\n".to_owned(),
        }],
    );
    let grown = detail
        .until(|detail| output(detail).is_some_and(|output| output.contains("Finished")))
        .await;
    assert_eq!(
        output(&grown).as_deref(),
        Some("   Compiling shaman-relay\n    Finished `dev` profile")
    );

    // Popped, the detail crosses no more; the work still does while it is up.
    detail.subscription.cancel();
    let told = detail.told();
    say(
        &script,
        [ProviderEvent::CommandCompleted {
            activity_id: build,
            status: ProviderCommandStatus::Completed,
            exit_status: Some(0),
        }],
    );
    work.until(|work| {
        work.items
            .iter()
            .any(|item| matches!(&item.kind, WorkKind::Row { row } if row.tone == Tone::Succeeded))
    })
    .await;
    assert_eq!(
        detail.told(),
        told,
        "nothing crosses once its screen is gone"
    );

    // A Turn's work settles with it.
    say(&script, [ProviderEvent::TurnCompleted]);
    let settled = work
        .until(|work| work.title.starts_with("Worked for "))
        .await;
    assert_eq!(
        settled.items.len(),
        1,
        "its last Message now its answer, outside the card: {settled:?}"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_turn_reads_how_long_it_ran_behind_its_card() {
    let desk = ServingServer::start("thin-open-stopped").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = desk
        .begin(&workspace(&holder, SCRIPTED), "Add that CLI flag too")
        .await;
    let script = desk.script().await;
    say(
        &script,
        [ProviderEvent::CommandStarted {
            activity_id: ProviderActivityId::new("build"),
            command: "cargo build".to_owned(),
            cwd: None,
        }],
    );
    let row = row(&phone, "Add that CLI flag too").await;
    let reading = phone.open(&row.session);
    reading
        .conversation_until(|items| cards(items).iter().any(|card| card.line == "cargo build"))
        .await;

    desk.interrupt(session).await;
    let stopped = reading
        .conversation_until(|items| {
            cards(items)
                .first()
                .is_some_and(|card| card.outcome == TurnOutcome::Stopped)
        })
        .await;
    let card = &cards(&stopped.items)[0];
    assert!(card.line.starts_with("Stopped after "), "{}", card.line);
    assert_eq!(card.count.as_deref(), Some("1 command"));

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_working_session_reads_live_and_closing_it_cancels_nothing_on_the_server() {
    let desk = ServingServer::start("thin-open-working").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Tidy the build")
        .await;
    let row = row(&phone, "Tidy the build").await;

    let reading = phone.open(&row.session);
    let conversation = reading.conversation_until(loaded).await;
    assert_eq!(
        read_as(&conversation.items),
        ["user: Tidy the build"],
        "a Prompt not yet delivered stands as the Message it will be"
    );

    assert!(reading.session.is_reading());
    reading.session.close();
    assert!(!reading.session.is_reading(), "closed, it reads no more");
    let snapshot = desk.read(session_id(&row.session)).await;
    assert_eq!(
        snapshot.session.status,
        SessionStatus::Active,
        "closing it stopped nothing"
    );
    assert!(snapshot.session.working_since.is_some());

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_a_session_turns_the_outlook_toward_its_origin_and_workspace_and_keeps_it() {
    let desk = ServingServer::start("thin-open-desk").await;
    let lab = ServingServer::start("thin-open-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let Outlook::Remote { name, begins_in } = phone.outlook().await else {
        panic!("turned toward a Remote");
    };
    assert_eq!(name, "lab", "the Remote last paired");
    assert_eq!(begins_in.place, "lab");

    let mapped = row(&phone, "Map the workspace").await;
    let reading = phone.open(&mapped.session);
    assert_eq!(
        reading.frame().await.place,
        "desk · shaman",
        "the Origin is named where the phone holds more than one Remote"
    );
    let turned = |outlook: &Outlook| matches!(outlook, Outlook::Remote { begins_in, .. } if begins_in.place == "desk · shaman");
    let outlook = phone.outlook_until(turned).await;
    let Outlook::Remote { name, begins_in } = outlook else {
        unreachable!()
    };
    assert_eq!(name, "desk");
    assert_eq!(begins_in.said, "New sessions begin in desk · shaman");
    drop(reading);

    let phone = phone.relaunched();
    assert!(
        turned(&phone.outlook().await),
        "kept from one run to the next"
    );

    // Where no Workspace is remembered for a Remote, its Session's own is.
    lab.begin(&workspace(&holder, "tidy"), "Tidy the build")
        .await;
    let tidy = row(&phone, "Tidy the build").await;
    let reading = phone.open(&tidy.session);
    phone
        .outlook_until(|outlook| {
            matches!(outlook, Outlook::Remote { begins_in, .. } if begins_in.place == "lab · tidy")
        })
        .await;
    drop(reading);

    // Where one is, it stays: opening another of that Remote's Sessions
    // turns only the Remote.
    desk.begin(&workspace(&holder, "dotfiles"), "Trim the shell")
        .await;
    let trim = row(&phone, "Trim the shell").await;
    let reading = phone.open(&trim.session);
    assert_eq!(reading.frame().await.place, "desk · dotfiles");
    let outlook = phone
        .outlook_until(|outlook| matches!(outlook, Outlook::Remote { name, .. } if name == "desk"))
        .await;
    assert!(
        turned(&outlook),
        "the Workspace remembered for desk stays: {outlook:?}"
    );

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_deleted_while_it_is_open_says_it_is_gone_and_is_read_no_more() {
    let desk = ServingServer::start("thin-open-deleted").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let row = row(&phone, "Map the workspace").await;
    let reading = phone.open(&row.session);
    // A Session is deleted once its Turn has failed, and no longer works.
    reading.conversation_until(failed_turn).await;

    desk.delete(session_id(&row.session)).await;
    let frame = reading.frame_until(|frame| frame.gone.is_some()).await;
    assert_eq!(frame.gone.as_deref(), Some("This session no longer exists"));
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while reading.session.is_reading() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("read no more");

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_whose_pairing_was_ended_says_so_in_the_servers_words() {
    let desk = ServingServer::start("thin-open-revoked").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let row = row(&phone, "Map the workspace").await;

    desk.remove_peer(&phone.fingerprint()).await;
    let reading = phone.open(&row.session);
    let frame = reading.frame_until(|frame| frame.gone.is_some()).await;
    assert_eq!(
        frame.title, "Map the workspace",
        "framed by what the listing knew"
    );
    assert!(
        frame
            .gone
            .as_deref()
            .is_some_and(|gone| gone.contains("revoked")),
        "{frame:?}"
    );
    assert!(
        !loaded(&reading.conversation_now().items),
        "never read, so the loading mark stands"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_its_server_no_longer_holds_says_it_is_gone() {
    let desk = ServingServer::start("thin-open-gone").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    phone.listing_until(|listing| listing.empty.is_some()).await;

    let reading = phone.open(&format!("{}/{}", desk.fingerprint(), SessionId::new()));
    let frame = reading.frame_until(|frame| frame.gone.is_some()).await;
    assert_eq!(frame.gone.as_deref(), Some("This session no longer exists"));

    let reading = phone.open("not a reference");
    assert!(reading.frame().await.gone.is_some());
    assert!(!reading.session.is_reading());

    phone.shutdown();
    desk.shutdown().await;
}

/// The text of every Prompt the Server holds of the Session `session`, as
/// it was delivered.
async fn prompts(server: &ServingServer, session: SessionId) -> Vec<(String, PromptDelivery)> {
    server
        .read(session)
        .await
        .prompts
        .into_iter()
        .map(|prompt| (prompt.text, prompt.delivery))
        .collect()
}

fn sending(items: &[ConversationItem]) -> Vec<(String, bool)> {
    items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::UserMessage { text, sending, .. } => Some((text.clone(), *sending)),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_sent_to_a_live_turn_steers_it_and_the_bar_says_it_works_until_stopped() {
    let desk = ServingServer::start("thin-send-steer").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = desk
        .begin(
            &workspace(&holder, SCRIPTED),
            "Draw the invite as a QR code",
        )
        .await;
    let script = desk.script().await;
    say(
        &script,
        [ProviderEvent::CommandStarted {
            activity_id: ProviderActivityId::new("qr"),
            command: "cargo test qr".to_owned(),
            cwd: None,
        }],
    );
    let row = row(&phone, "Draw the invite as a QR code").await;
    let reading = phone.open(&row.session);
    let frame = reading.frame_until(|frame| frame.bar.is_some()).await;
    let bar = frame.bar.expect("a bar");
    assert_eq!(bar.kind, BarKind::Working);
    assert_eq!(bar.words, "Working");
    assert_eq!(bar.stop.as_deref(), Some("Stop"));
    let composer = frame.composer.expect("a composer");
    assert_eq!(composer.queue.as_deref(), Some("Queue for after this turn"));
    assert_eq!(composer.unreachable, None);

    reading
        .session
        .send("  Give it a quiet zone too.\n".to_owned())
        .await
        .expect("sent");
    assert_eq!(
        prompts(&desk, session).await.last(),
        Some(&(
            "Give it a quiet zone too.".to_owned(),
            PromptDelivery::Steer
        )),
        "delivered as a steer, trimmed"
    );
    let steered = reading
        .conversation_until(|items| {
            sending(items).contains(&("Give it a quiet zone too.".to_owned(), false))
        })
        .await;
    assert_eq!(
        sending(&steered.items)
            .iter()
            .filter(|(text, _)| text == "Give it a quiet zone too.")
            .count(),
        1,
        "it stands once, as the Server holds it: {:?}",
        read_as(&steered.items)
    );

    let refused = reading.session.send("  \n".to_owned()).await;
    assert_eq!(
        refused,
        Err(ActError::Invalid {
            message: "Write something to send".to_owned()
        })
    );

    reading.session.stop().await.expect("stopped");
    let stopped = reading
        .conversation_until(|items| {
            cards(items)
                .first()
                .is_some_and(|card| card.outcome == TurnOutcome::Stopped)
        })
        .await;
    assert!(!stopped.items.is_empty());
    reading.frame_until(|frame| frame.bar.is_none()).await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_prompt_waits_at_the_foot_and_is_steered_now_or_cancelled() {
    let desk = ServingServer::start("thin-send-queue").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = desk
        .begin(&workspace(&holder, SCRIPTED), "Add that CLI flag too")
        .await;
    let script = desk.script().await;
    say(
        &script,
        [ProviderEvent::CommandStarted {
            activity_id: ProviderActivityId::new("build"),
            command: "cargo build".to_owned(),
            cwd: None,
        }],
    );
    let row = row(&phone, "Add that CLI flag too").await;
    let reading = phone.open(&row.session);
    reading.frame_until(|frame| frame.bar.is_some()).await;

    reading
        .session
        .queue("Then the changelog".to_owned())
        .await
        .expect("queued");
    let frame = reading
        .frame_until(|frame| frame.queued.iter().any(|queued| queued.label == "Queued"))
        .await;
    let [queued] = &frame.queued[..] else {
        panic!("one queued: {:?}", frame.queued);
    };
    assert_eq!(queued.text, "Then the changelog");
    assert_eq!(queued.steer_now.as_deref(), Some("Steer now"));
    assert_eq!(queued.cancel.as_deref(), Some("Cancel"));
    assert!(
        !sending(&reading.conversation_now().items)
            .iter()
            .any(|(text, _)| text == "Then the changelog"),
        "it waits at the foot, not among what was said"
    );

    reading
        .session
        .steer_now(queued.prompt.clone())
        .await
        .expect("steered now");
    reading.frame_until(|frame| frame.queued.is_empty()).await;
    assert!(
        prompts(&desk, session)
            .await
            .contains(&("Then the changelog".to_owned(), PromptDelivery::Steer)),
        "delivered as a steer"
    );

    reading
        .session
        .queue("And the docs".to_owned())
        .await
        .expect("queued");
    let frame = reading
        .frame_until(|frame| frame.queued.iter().any(|queued| queued.label == "Queued"))
        .await;
    reading
        .session
        .cancel_queued(frame.queued[0].prompt.clone())
        .await
        .expect("cancelled");
    reading.frame_until(|frame| frame.queued.is_empty()).await;
    let snapshot = desk.read(session).await;
    let cancelled = snapshot
        .prompts
        .iter()
        .find(|prompt| prompt.text == "And the docs")
        .expect("held");
    assert_eq!(cancelled.status, shaman::protocol::PromptStatus::Cancelled);
    assert_eq!(
        phone.core.draft(row.session.clone()).text,
        "",
        "a cancelled Prompt is not owed back"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_on_a_session_working_only_for_a_prompt_not_yet_delivered_gives_its_text_back() {
    let desk = ServingServer::start("thin-stop-withdraw").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Tidy the build")
        .await;
    let row = row(&phone, "Tidy the build").await;
    let reading = phone.open(&row.session);
    let frame = reading.frame_until(|frame| frame.bar.is_some()).await;
    assert_eq!(frame.bar.map(|bar| bar.words).as_deref(), Some("Working"));
    phone
        .core
        .set_draft(row.session.clone(), "and the docs".to_owned(), 0);

    reading.session.stop().await.expect("stopped");
    assert_eq!(
        phone.core.draft(row.session.clone()).text,
        "Tidy the build\nand the docs",
        "ahead of anything already written there"
    );
    reading.frame_until(|frame| frame.bar.is_none()).await;
    reading
        .conversation_until(|items| sending(items).is_empty())
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_that_fails_goes_from_the_conversation_and_its_text_goes_back_to_the_composer() {
    let desk = ServingServer::start("thin-send-gone").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = desk
        .begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let row = row(&phone, "Map the workspace").await;
    let reading = phone.open(&row.session);
    reading.conversation_until(failed_turn).await;
    phone
        .core
        .set_draft(row.session.clone(), "written meanwhile".to_owned(), 0);

    desk.delete(session).await;
    let failed = reading.session.send("One more thing".to_owned()).await;
    assert!(matches!(failed, Err(ActError::Gone { .. })), "{failed:?}");
    assert_eq!(
        phone.core.draft(row.session.clone()).text,
        "One more thing\nwritten meanwhile"
    );
    // Its bubble goes.
    reading
        .conversation_until(|items| {
            !sending(items)
                .iter()
                .any(|(text, _)| text == "One more thing")
        })
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn on_an_unreachable_origin_a_send_waits_marked_as_sending_and_goes_when_it_answers() {
    let desk = ServingServer::start("thin-send-redial").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::new();
    let invite = desk.invite(vec![Way::Direct(gate.address())]).await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = desk
        .begin(
            &workspace(&holder, SCRIPTED),
            "Draw the invite as a QR code",
        )
        .await;
    let script = desk.script().await;
    say(
        &script,
        [ProviderEvent::CommandStarted {
            activity_id: ProviderActivityId::new("qr"),
            command: "cargo test qr".to_owned(),
            cwd: None,
        }],
    );
    let row = row(&phone, "Draw the invite as a QR code").await;
    let reading = phone.open(&row.session);
    reading.frame_until(|frame| frame.bar.is_some()).await;
    phone.acts_until(false).await;

    gate.shut_now();
    let frame = reading
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.is_some())
        })
        .await;
    assert_eq!(
        frame
            .composer
            .and_then(|composer| composer.unreachable)
            .as_deref(),
        Some("The server is unreachable. Nothing sent goes anywhere until it answers.")
    );

    let sent = {
        let session = reading.session.clone();
        tokio::spawn(async move { session.send("Give it a quiet zone".to_owned()).await })
    };
    reading
        .conversation_until(|items| {
            sending(items).contains(&("Give it a quiet zone".to_owned(), true))
        })
        .await;
    phone.acts_until(true).await;
    // It is tried while the way is shut, and is not given up.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(!sent.is_finished(), "it waits");

    gate.open_now();
    sent.await
        .expect("the send ends")
        .expect("sent once it answered");
    phone.acts_until(false).await;
    assert_eq!(
        prompts(&desk, session)
            .await
            .iter()
            .filter(|(text, _)| text == "Give it a quiet zone")
            .count(),
        1,
        "sent once"
    );
    reading
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.is_none())
        })
        .await;
    reading
        .conversation_until(|items| {
            sending(items).contains(&("Give it a quiet zone".to_owned(), false))
        })
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

/// A phone paired with `desk` through `gate`, with the Session titled
/// `title` begun in a Workspace of `holder`'s named `workspace`, open and
/// read, on a phone whose core waits as `timings` say.
async fn opened_through(
    desk: &ServingServer,
    gate: &Gate,
    timings: shaman_thin_client::CoreTimings,
    holder: &tempfile::TempDir,
    workspace_name: &str,
    title: &str,
) -> (Phone, SessionId, ActiveRow, support::Reading) {
    let phone = Phone::with(timings);
    let invite = desk.invite(vec![Way::Direct(gate.address())]).await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let session = desk.begin(&workspace(holder, workspace_name), title).await;
    let row = row(&phone, title).await;
    let reading = phone.open(&row.session);
    reading.conversation_until(loaded).await;
    (phone, session, row, reading)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_still_dialling_when_its_bound_passes_was_not_sent_and_its_text_goes_back() {
    let desk = ServingServer::start("thin-send-undialled").await;
    let gate = Gate::open(desk.serving_address()).await;
    let mut timings = support::timings();
    // A dial that hears nothing is still under way when the act's bound
    // passes, and fails a moment later: nothing was sent.
    timings.redeemer.handshake_timeout = std::time::Duration::from_millis(500);
    timings.act_bound = std::time::Duration::from_millis(200);
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, session, row, reading) = opened_through(
        &desk,
        &gate,
        timings,
        &holder,
        "shaman",
        "Map the workspace",
    )
    .await;

    // Once the connection it held is known gone, a send has to dial.
    gate.shut_now();
    reading
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.is_some())
        })
        .await;
    let failed = reading.session.send("One more thing".to_owned()).await;
    assert_eq!(
        failed,
        Err(ActError::Unreachable {
            message: "The server did not answer in time".to_owned()
        })
    );
    assert_eq!(phone.core.draft(row.session.clone()).text, "One more thing");
    assert_eq!(
        reading
            .frame()
            .await
            .composer
            .and_then(|composer| composer.note),
        None,
        "it was not sent, and nothing says it may have been"
    );

    gate.open_now();
    reading
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.is_none())
        })
        .await;
    assert!(
        !prompts(&desk, session)
            .await
            .iter()
            .any(|(text, _)| text == "One more thing"),
        "never sent"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_delivered_whose_answer_is_lost_is_not_sent_again_and_says_it_may_have_been() {
    let desk = ServingServer::start("thin-send-lost").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, session, row, reading) = opened_through(
        &desk,
        &gate,
        support::timings(),
        &holder,
        "shaman",
        "Map the workspace",
    )
    .await;

    // What the phone says reaches the Server; nothing the Server says comes
    // back, so the connection is given up as its keepalive goes unanswered.
    gate.mute_now();
    let failed = reading.session.send("One more thing".to_owned()).await;
    assert_eq!(
        failed,
        Err(ActError::Unreachable {
            message: "The server did not answer".to_owned()
        })
    );
    assert_eq!(
        phone.core.draft(row.session.clone()).text,
        "One more thing",
        "its text stays in the composer"
    );
    let frame = reading
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.note.is_some())
        })
        .await;
    assert_eq!(
        frame.composer.and_then(|composer| composer.note).as_deref(),
        Some("The server did not answer, so this prompt may have been sent")
    );
    assert_eq!(
        prompts(&desk, session)
            .await
            .iter()
            .filter(|(text, _)| text == "One more thing")
            .count(),
        1,
        "it was delivered, once, and never sent again"
    );

    phone.shutdown();
    desk.shutdown().await;
}

/// Has the scripted Session's Provider ask the Approval `approval`.
fn ask(script: &Script, approval: &Approval) {
    say(
        script,
        [ProviderEvent::ApprovalRequested {
            approval: approval.clone(),
            tool_activity_id: None,
        }],
    );
}

fn command_approval(command: &str, reason: Option<&str>) -> Approval {
    Approval {
        id: ApprovalId::new(),
        subject: ApprovalSubject::Command {
            command: command.to_owned(),
            cwd: None,
            actions: Vec::new(),
        },
        reason: reason.map(str::to_owned),
    }
}

/// The Approval the frame presents, where it presents one.
fn presented(frame: &SessionFrame) -> Option<&PendingApproval> {
    match &frame.intervention {
        Some(Intervention::Approval { approval }) => Some(approval),
        _ => None,
    }
}

/// Whether the frame presents the Approval `approval`.
fn presents(frame: &SessionFrame, approval: &Approval) -> bool {
    presented(frame).is_some_and(|presented| presented.reference == approval.id.to_string())
}

/// Whether the frame's bar is the orange one, an Intervention waiting out of
/// sight, with "Review" or without.
fn waiting(frame: &SessionFrame, review: bool) -> bool {
    frame.intervention.is_none()
        && frame.bar.as_ref().is_some_and(|bar| {
            bar.kind == BarKind::Waiting
                && bar.words == "Approval waiting"
                && bar.review.is_some() == review
        })
}

/// How the Server holds the Approval `approval` now.
async fn outcome(
    server: &ServingServer,
    session: SessionId,
    approval: &Approval,
) -> ApprovalOutcome {
    server
        .read(session)
        .await
        .activities
        .into_iter()
        .find_map(|activity| match activity {
            Activity::Approval {
                approval: asked,
                outcome,
                ..
            } if asked.id == approval.id => Some(outcome),
            _ => None,
        })
        .expect("the Server holds the Approval")
}

/// A phone paired with `desk` through `gate`, reading a scripted Session
/// open on it, and what its Provider sends.
async fn scripted_through(
    desk: &ServingServer,
    gate: &Gate,
    holder: &tempfile::TempDir,
    title: &str,
) -> (Phone, SessionId, ActiveRow, support::Reading, Script) {
    let phone = Phone::new();
    let invite = desk.invite(vec![Way::Direct(gate.address())]).await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let session = desk.begin(&workspace(holder, SCRIPTED), title).await;
    let script = desk.script().await;
    let row = row(&phone, title).await;
    let reading = phone.open(&row.session);
    reading.conversation_until(loaded).await;
    (phone, session, row, reading, script)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approval_presents_itself_is_dismissed_reviewed_and_answered_against_a_serving_server() {
    let desk = ServingServer::start("thin-approval").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, session, row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Draw the invite as a QR code").await;
    let asked = command_approval(
        "cargo nextest run -p shaman-relay",
        Some("Needs network access outside the sandbox."),
    );
    ask(&script, &asked);

    let frame = reading.frame_until(|frame| presents(frame, &asked)).await;
    let sheet = presented(&frame).expect("presented");
    assert_eq!(sheet.title, "Run this command?");
    assert_eq!(
        sheet
            .facts
            .iter()
            .map(|fact| (fact.label.as_str(), fact.value.as_str()))
            .collect::<Vec<_>>(),
        [
            ("Command", "cargo nextest run -p shaman-relay"),
            ("Reason", "Needs network access outside the sandbox."),
        ]
    );
    assert_eq!(
        frame.bar.as_ref().map(|bar| bar.kind),
        Some(BarKind::Working),
        "the bar beneath the sheet stays as it was"
    );
    let conversation = reading
        .conversation_until(|items| {
            cards(items)
                .iter()
                .any(|card| card.outcome == TurnOutcome::Waiting)
        })
        .await;
    assert_eq!(
        cards(&conversation.items)[0].line,
        "Waiting on you",
        "the live Turn's card says it waits on the reader"
    );

    // Swiped away, it waits behind the orange bar, and Review brings it back.
    reading.session.dismiss();
    reading.frame_until(|frame| waiting(frame, true)).await;
    reading.session.review();
    reading.frame_until(|frame| presents(frame, &asked)).await;

    // Opened again on another screen, it presents itself however it was
    // dismissed on this one.
    reading.session.dismiss();
    reading.frame_until(|frame| waiting(frame, true)).await;
    let again = phone.open(&row.session);
    again.frame_until(|frame| presents(frame, &asked)).await;
    drop(again);

    reading.session.review();
    let reference = reading
        .frame_until(|frame| presents(frame, &asked))
        .await
        .intervention
        .and_then(|intervention| match intervention {
            Intervention::Approval { approval } => Some(approval.reference),
            _ => None,
        })
        .expect("presented");
    reading
        .session
        .decide(reference, DecisionKind::AcceptOnce)
        .await
        .expect("answered once the Server has");
    assert_eq!(desk.decisions(), [(asked.id, Decision::Accept)]);
    assert_eq!(
        outcome(&desk, session, &asked).await,
        ApprovalOutcome::Decided
    );
    let answered = reading
        .frame_until(|frame| frame.intervention.is_none())
        .await;
    assert_eq!(
        answered.bar.map(|bar| bar.kind),
        Some(BarKind::Working),
        "nothing waits on the reader any more"
    );
    phone.acts_until(false).await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn several_pending_present_oldest_first_one_by_one_and_one_asked_after_a_dismissal_presents_itself()
 {
    let desk = ServingServer::start("thin-approvals").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, _session, _row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Tidy the build").await;
    let first = command_approval("cargo build", None);
    let second = command_approval("cargo test", None);
    ask(&script, &first);
    ask(&script, &second);
    // Both are asked before the first is answered.
    reading
        .conversation_until(|items| {
            cards(items)
                .iter()
                .any(|card| card.outcome == TurnOutcome::Waiting)
        })
        .await;
    reading.frame_until(|frame| presents(frame, &first)).await;

    reading
        .session
        .decide(first.id.to_string(), DecisionKind::Decline)
        .await
        .expect("declined");
    reading.frame_until(|frame| presents(frame, &second)).await;

    reading.session.dismiss();
    reading.frame_until(|frame| waiting(frame, true)).await;
    let third = command_approval("cargo clippy", None);
    ask(&script, &third);
    reading.frame_until(|frame| presents(frame, &third)).await;
    reading
        .session
        .decide(third.id.to_string(), DecisionKind::AcceptForSession)
        .await
        .expect("accepted for the session");
    reading.frame_until(|frame| waiting(frame, true)).await;
    reading.session.review();
    reading.frame_until(|frame| presents(frame, &second)).await;
    reading
        .session
        .decide(second.id.to_string(), DecisionKind::DeclineAndInterrupt)
        .await
        .expect("declined and interrupted");
    assert_eq!(
        desk.decisions_until(3).await,
        [
            (first.id, Decision::Decline),
            (third.id, Decision::AcceptForSession),
            (second.id, Decision::DeclineAndInterrupt),
        ]
    );
    reading
        .frame_until(|frame| frame.intervention.is_none() && !waiting(frame, true))
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approval_on_an_unreachable_origin_waits_out_of_sight_and_presents_itself_when_it_answers()
 {
    let desk = ServingServer::start("thin-approval-away").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, _session, _row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Tidy the build").await;
    let asked = command_approval("cargo build", None);
    ask(&script, &asked);
    reading.frame_until(|frame| presents(frame, &asked)).await;

    gate.shut_now();
    reading.frame_until(|frame| waiting(frame, false)).await;

    gate.open_now();
    reading.frame_until(|frame| presents(frame, &asked)).await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_decision_delivered_whose_answer_is_lost_is_not_sent_again() {
    let desk = ServingServer::start("thin-approval-lost").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, session, _row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Tidy the build").await;
    let asked = command_approval("cargo build", None);
    ask(&script, &asked);
    reading.frame_until(|frame| presents(frame, &asked)).await;

    // What the phone says reaches the Server; nothing it says comes back.
    gate.mute_now();
    let failed = reading
        .session
        .decide(asked.id.to_string(), DecisionKind::AcceptOnce)
        .await;
    assert_eq!(
        failed,
        Err(ActError::Unreachable {
            message: "The server did not answer, so the decision may have been sent".to_owned()
        })
    );
    assert_eq!(
        desk.decisions(),
        [(asked.id, Decision::Accept)],
        "it was delivered, once, and never sent again"
    );
    assert_eq!(
        outcome(&desk, session, &asked).await,
        ApprovalOutcome::Decided
    );
    assert!(
        reading.frame().await.intervention.is_none(),
        "it does not spring back to be answered twice"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_decision_on_an_approval_no_longer_waiting_is_refused_in_the_servers_words() {
    let desk = ServingServer::start("thin-approval-gone").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, _session, _row, reading, _script) =
        scripted_through(&desk, &gate, &holder, "Tidy the build").await;
    let refused = reading
        .session
        .decide(ApprovalId::new().to_string(), DecisionKind::AcceptOnce)
        .await;
    assert!(
        matches!(refused, Err(ActError::Refused { .. })),
        "{refused:?}"
    );
    assert_eq!(
        reading
            .session
            .decide("not an approval".to_owned(), DecisionKind::AcceptOnce)
            .await,
        Err(ActError::Refused {
            message: "That approval is no longer waiting".to_owned()
        })
    );
    assert!(desk.decisions().is_empty());

    phone.shutdown();
    desk.shutdown().await;
}

/// Has the scripted Session's Provider ask the Questionnaire `questionnaire`.
fn ask_questionnaire(script: &Script, questionnaire: &Questionnaire) {
    say(
        script,
        [ProviderEvent::QuestionnaireRequested {
            questionnaire: questionnaire.clone(),
        }],
    );
}

/// Two Questions: where the tests run, one of two machines with a note
/// beside it, required; and a deploy token, secret, that may be skipped.
fn where_and_token() -> Questionnaire {
    let choice = |id: &str, label: &str, description: &str, recommended| QuestionChoice {
        id: id.to_owned(),
        label: label.to_owned(),
        description: Some(description.to_owned()),
        recommended,
    };
    Questionnaire {
        id: QuestionnaireId::new(),
        questions: vec![
            Question {
                id: "machine".to_owned(),
                title: Some("Machine".to_owned()),
                text: "Where should the tests run?".to_owned(),
                choices: vec![
                    choice("staging", "Staging", "The shared box.", true),
                    choice("local", "Local", "This laptop.", false),
                ],
                multiple: false,
                freeform: true,
                combine_freeform: true,
                secret: false,
                required: true,
            },
            Question {
                id: "token".to_owned(),
                title: None,
                text: "The deploy token, if there is one?".to_owned(),
                choices: Vec::new(),
                multiple: false,
                freeform: true,
                combine_freeform: false,
                secret: true,
                required: false,
            },
        ],
    }
}

/// The Questionnaire the frame presents, where it presents one.
fn presented_questionnaire(frame: &SessionFrame) -> Option<&PendingQuestionnaire> {
    match &frame.intervention {
        Some(Intervention::Questionnaire { questionnaire }) => Some(questionnaire),
        _ => None,
    }
}

/// Whether the frame presents the Questionnaire `questionnaire`.
fn presents_questionnaire(frame: &SessionFrame, questionnaire: &Questionnaire) -> bool {
    presented_questionnaire(frame)
        .is_some_and(|presented| presented.reference == questionnaire.id.to_string())
}

/// Whether the frame's bar says a Questionnaire waits out of sight, with
/// "Review".
fn questionnaire_waiting(frame: &SessionFrame) -> bool {
    frame.intervention.is_none()
        && frame.bar.as_ref().is_some_and(|bar| {
            bar.kind == BarKind::Waiting
                && bar.words == "Questionnaire waiting"
                && bar.review.as_ref().is_some_and(|review| {
                    review.label == "Review"
                        && review.opens == shaman_thin_client::ReviewOpens::Interventions
                })
        })
}

/// How the Server holds the Questionnaire `questionnaire` now, and the
/// Answer it keeps of it.
async fn questionnaire_outcome(
    server: &ServingServer,
    session: SessionId,
    questionnaire: &Questionnaire,
) -> (QuestionnaireOutcome, Option<Answer>) {
    server
        .read(session)
        .await
        .activities
        .into_iter()
        .find_map(|activity| match activity {
            Activity::Questionnaire {
                questionnaire: asked,
                outcome,
                answer,
                ..
            } if asked.id == questionnaire.id => Some((outcome, answer)),
            _ => None,
        })
        .expect("the Server holds the Questionnaire")
}

fn given(choices: &[&str], text: &str) -> GivenAnswer {
    GivenAnswer {
        choices: choices.iter().map(|choice| (*choice).to_owned()).collect(),
        text: text.to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_questionnaire_presents_itself_is_dismissed_reviewed_and_answered_against_a_serving_server()
 {
    let desk = ServingServer::start("thin-questionnaire").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, session, row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Run the tests somewhere").await;
    let asked = where_and_token();
    ask_questionnaire(&script, &asked);

    let frame = reading
        .frame_until(|frame| presents_questionnaire(frame, &asked))
        .await;
    let sheet = presented_questionnaire(&frame).expect("presented");
    assert_eq!(
        sheet
            .questions
            .iter()
            .map(|question| question.position.as_str())
            .collect::<Vec<_>>(),
        ["Question 1 of 2", "Question 2 of 2"]
    );
    let machine = &sheet.questions[0];
    assert_eq!(machine.text, "Where should the tests run?");
    assert_eq!(
        machine
            .choices
            .iter()
            .map(|card| (card.label.as_str(), card.badge.as_deref()))
            .collect::<Vec<_>>(),
        [("Staging", Some("Recommended")), ("Local", None)]
    );
    assert_eq!(
        machine.field.as_ref().map(|field| field.label.as_str()),
        Some("Additional text")
    );
    assert_eq!(machine.skip, None, "a required Question holds Next");
    let token = &sheet.questions[1];
    assert!(token.field.as_ref().is_some_and(|field| field.secure));
    assert_eq!(token.skip.as_deref(), Some("Skip"));
    // The frame is told before the conversation, so the card is waited for.
    reading
        .conversation_until(|items| {
            cards(items).last().map(|card| card.line.as_str()) == Some("Waiting on you")
        })
        .await;

    // Swiped away, it waits behind the orange bar, and Review brings it back.
    reading.session.dismiss();
    reading.frame_until(questionnaire_waiting).await;
    reading.session.review();
    reading
        .frame_until(|frame| presents_questionnaire(frame, &asked))
        .await;

    // Opened again on another screen, it presents itself however it was
    // dismissed on this one.
    reading.session.dismiss();
    reading.frame_until(questionnaire_waiting).await;
    let again = phone.open(&row.session);
    again
        .frame_until(|frame| presents_questionnaire(frame, &asked))
        .await;
    drop(again);
    reading.session.review();
    reading
        .frame_until(|frame| presents_questionnaire(frame, &asked))
        .await;

    reading
        .session
        .answer(
            asked.id.to_string(),
            vec![given(&["staging"], "after lunch"), given(&[], "hunter2")],
        )
        .await
        .expect("answered once the Server has");
    let delivered = desk.submissions();
    assert_eq!(delivered.len(), 1, "delivered once");
    let (id, QuestionnaireSubmission::Answer { answer }) = &delivered[0] else {
        panic!("an Answer delivered");
    };
    assert_eq!(*id, asked.id);
    assert_eq!(
        answer.questions,
        [
            QuestionAnswer::SelectedWithFreeform {
                choices: vec!["staging".to_owned()],
                text: "after lunch".to_owned(),
            },
            QuestionAnswer::Freeform {
                text: "hunter2".to_owned(),
            },
        ]
    );
    let (outcome, _) = questionnaire_outcome(&desk, session, &asked).await;
    assert_eq!(outcome, QuestionnaireOutcome::Answered);
    reading
        .frame_until(|frame| frame.intervention.is_none() && !questionnaire_waiting(frame))
        .await;

    // Settled, it is a row among the Turn's work, its secret never shown
    // again.
    let conversation = reading
        .conversation_until(|items| {
            cards(items)
                .last()
                .is_some_and(|card| card.outcome != TurnOutcome::Waiting)
        })
        .await;
    let card = cards(&conversation.items).last().cloned().expect("a card");
    let work = reading.work(&card.work);
    let listed = work
        .until(|work| {
            work.items.iter().any(|item| {
                matches!(&item.kind, WorkKind::Row { row } if row.title == "Questionnaire · Answered")
            })
        })
        .await;
    let activity = listed
        .items
        .iter()
        .find_map(|item| match &item.kind {
            WorkKind::Row { row } if row.title == "Questionnaire · Answered" => match &row.tap {
                Tap::Detail { activity } => Some(activity.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("its row pushes what was asked and answered");
    let detail = reading.detail(&activity).until(|_| true).await;
    assert_eq!(
        detail
            .sections
            .iter()
            .map(|section| (section.heading.clone(), section.body.clone()))
            .collect::<Vec<_>>(),
        [
            (
                Some("Where should the tests run?".to_owned()),
                "Staging; after lunch".to_owned()
            ),
            (
                Some("The deploy token, if there is one?".to_owned()),
                "Answered (secret)".to_owned()
            ),
        ]
    );
    phone.acts_until(false).await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_questionnaire_declined_is_declined_whole() {
    let desk = ServingServer::start("thin-questionnaire-decline").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, session, _row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Run the tests somewhere").await;
    let asked = where_and_token();
    ask_questionnaire(&script, &asked);
    reading
        .frame_until(|frame| presents_questionnaire(frame, &asked))
        .await;

    reading
        .session
        .decline(asked.id.to_string())
        .await
        .expect("declined once the Server has");
    assert!(matches!(
        desk.submissions().as_slice(),
        [(id, QuestionnaireSubmission::Decline)] if *id == asked.id
    ));
    assert_eq!(
        questionnaire_outcome(&desk, session, &asked).await.0,
        QuestionnaireOutcome::Declined
    );
    reading
        .frame_until(|frame| frame.intervention.is_none())
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_a_required_question_lacks_are_refused_and_nothing_is_delivered() {
    let desk = ServingServer::start("thin-questionnaire-refused").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, _session, _row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Run the tests somewhere").await;
    let asked = where_and_token();
    ask_questionnaire(&script, &asked);
    reading
        .frame_until(|frame| presents_questionnaire(frame, &asked))
        .await;

    let refused = reading
        .session
        .answer(asked.id.to_string(), vec![given(&[], ""), given(&[], "")])
        .await;
    let Err(ActError::Refused { message }) = refused else {
        panic!("refused in the Server's words: {refused:?}");
    };
    assert!(message.contains("required"), "{message}");
    assert!(desk.submissions().is_empty(), "nothing was delivered");
    assert!(
        presents_questionnaire(&reading.frame().await, &asked),
        "it still asks"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_delivered_whose_answer_is_lost_are_not_sent_again() {
    let desk = ServingServer::start("thin-questionnaire-lost").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, session, _row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Run the tests somewhere").await;
    let asked = where_and_token();
    ask_questionnaire(&script, &asked);
    reading
        .frame_until(|frame| presents_questionnaire(frame, &asked))
        .await;

    // What the phone says reaches the Server; nothing it says comes back.
    gate.mute_now();
    let failed = reading
        .session
        .answer(
            asked.id.to_string(),
            vec![given(&["local"], ""), given(&[], "")],
        )
        .await;
    assert_eq!(
        failed,
        Err(ActError::Unreachable {
            message: "The server did not answer, so the answers may have been sent".to_owned()
        })
    );
    assert_eq!(desk.submissions().len(), 1, "delivered once, never again");
    assert_eq!(
        questionnaire_outcome(&desk, session, &asked).await.0,
        QuestionnaireOutcome::Answered
    );
    assert!(
        reading.frame().await.intervention.is_none(),
        "it does not spring back to be answered twice"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_to_a_questionnaire_no_longer_waiting_are_refused() {
    let desk = ServingServer::start("thin-questionnaire-gone").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, _session, _row, reading, _script) =
        scripted_through(&desk, &gate, &holder, "Run the tests somewhere").await;
    let refused = reading
        .session
        .decline("not a questionnaire".to_owned())
        .await;
    assert_eq!(
        refused,
        Err(ActError::Refused {
            message: "That questionnaire is no longer waiting".to_owned()
        })
    );
    let refused = reading
        .session
        .decline(QuestionnaireId::new().to_string())
        .await;
    assert!(
        matches!(refused, Err(ActError::Refused { .. })),
        "refused in the Server's words: {refused:?}"
    );
    assert!(desk.submissions().is_empty());

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_its_provider_refuses_present_it_again_saying_so_and_it_stays_listed_until_answered()
 {
    let desk = ServingServer::start("thin-questionnaire-rejected").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (phone, session, _row, reading, script) =
        scripted_through(&desk, &gate, &holder, "Run the tests somewhere").await;
    let asked = where_and_token();
    ask_questionnaire(&script, &asked);
    let first = reading
        .frame_until(|frame| presents_questionnaire(frame, &asked))
        .await;
    assert_eq!(first.waiting, [asked.id.to_string()]);
    assert_eq!(
        presented_questionnaire(&first).and_then(|q| q.note.clone()),
        None
    );

    desk.refuse_submissions(1);
    let answers = || vec![given(&["local"], ""), given(&[], "hunter2")];
    let refused = reading
        .session
        .answer(asked.id.to_string(), answers())
        .await;
    assert!(
        matches!(refused, Err(ActError::Refused { .. })),
        "refused in the Server's words: {refused:?}"
    );
    assert!(desk.submissions().is_empty(), "nothing was taken");
    assert_eq!(
        questionnaire_outcome(&desk, session, &asked).await.0,
        QuestionnaireOutcome::SubmissionRejected
    );
    // It presents itself again, saying the answers did not go, and stays
    // listed, so the answers half given on the phone are kept to send again.
    let again = reading
        .frame_until(|frame| {
            presented_questionnaire(frame).is_some_and(|presented| presented.note.is_some())
        })
        .await;
    assert_eq!(
        presented_questionnaire(&again).and_then(|presented| presented.note.clone()),
        Some("Your answers were not delivered. Look them over and send them again.".to_owned())
    );
    assert_eq!(again.waiting, [asked.id.to_string()]);

    reading
        .session
        .answer(asked.id.to_string(), answers())
        .await
        .expect("answered again, and taken");
    assert_eq!(desk.submissions().len(), 1);
    // Answered, it is listed no more, once its Server is seen to have
    // delivered it: on its way, it is no longer presented and still listed.
    reading
        .frame_until(|frame| frame.intervention.is_none() && frame.waiting.is_empty())
        .await;

    phone.shutdown();
    desk.shutdown().await;
}
