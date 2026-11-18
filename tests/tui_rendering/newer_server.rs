//! What the TUI shows of a Remote newer than itself (ADR-0078): an Activity
//! of a kind it does not know stands as a row holding its place, so the
//! Transcript stays whole, and a change it does not know, on any of that
//! Remote's streams, is passed over and said once, quietly — the Remote is
//! newer and some things may not show — and not again.

use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use shaman::{
    managed_client::{ManagedEvent, SessionEvent, SubagentTreeEvent},
    protocol::{
        Activity, ActivityId, MessageRole, Outlook, SessionId, SessionReference, SessionSnapshot,
        SessionUpdate, TranscriptItem,
    },
    tui::{Application, ApplicationEvent, CommandId, SemanticCommandId},
};

use crate::tui_rendering::support::{
    application_looking_at_studio, navigable_session_snapshot, rendered_application_rows_at,
    rendered_row, workspace_dir,
};

const NEWER: &str = "studio runs a newer Shaman; some things may not show";

fn studio() -> Outlook {
    Outlook::Remote("studio".to_owned())
}

/// `value`, read as a Remote newer than this build sends it.
fn sent_by_a_newer_remote<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
    serde_json::from_value(value).expect("what a newer Remote sends still reads")
}

/// A Session on `studio` open, its one Turn's answer reading `answer`.
fn studio_session(workspace: &std::path::Path, answer: &str) -> (Application, SessionSnapshot) {
    let mut snapshot = navigable_session_snapshot(SessionId::new(), workspace, 1);
    snapshot
        .messages
        .iter_mut()
        .find(|message| message.role == MessageRole::Agent)
        .expect("the fixture has an answer")
        .content = answer.to_owned();
    (studio_showing(snapshot.clone()), snapshot)
}

/// An Application turned toward `studio`, showing `snapshot` of its Session.
fn studio_showing(snapshot: SessionSnapshot) -> Application {
    let mut application = application_looking_at_studio();
    application
        .handle_event(ApplicationEvent::OriginSessionAttached {
            reference: SessionReference::new(studio(), snapshot.session.id),
            snapshot,
        })
        .expect("open the remote's session");
    application
}

/// The update following `snapshot`, carrying `changes` as a newer Remote
/// writes them.
fn next_update(snapshot: &SessionSnapshot, changes: serde_json::Value) -> SessionUpdate {
    sent_by_a_newer_remote(json!({
        "session_id": snapshot.session.id,
        "revision": snapshot.revision.0 + 1,
        "changes": changes,
    }))
}

fn deliver(application: &mut Application, update: SessionUpdate) {
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(update)))
        .expect("take the remote's update");
}

/// Opens every settled Turn's fold, so the work behind it is drawn.
fn open_turn_folds(application: &mut Application) {
    application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            SemanticCommandId::TranscriptTurnsToggle,
        )))
        .expect("open every turn fold");
}

fn screen(application: &Application) -> String {
    rendered_application_rows_at(application, 120, 30).join("\n")
}

/// The reader's first interaction with whatever the screen shows, which
/// dismisses a Notice it has seen.
fn interact(application: &mut Application) {
    application.note_interaction(&InputEvent::Key(KeyEvent::new(
        KeyCode::F(12),
        KeyModifiers::NONE,
    )));
}

#[test]
fn an_activity_of_a_kind_this_build_does_not_know_stands_as_a_row_in_its_place() {
    let workspace = workspace_dir();
    let mut snapshot = navigable_session_snapshot(SessionId::new(), workspace.path(), 1);
    let turn_id = snapshot.turns[0].id;
    let answer = snapshot
        .messages
        .iter_mut()
        .find(|message| message.role == MessageRole::Agent)
        .expect("the fixture has an answer");
    answer.content = "Deployed the preview.".to_owned();
    let answer_id = answer.id;
    let newer: Activity = sent_by_a_newer_remote(json!({
        "kind": "deploy_preview",
        "id": ActivityId::new(),
        "turn_id": turn_id,
        "status": "completed",
        "url": "https://preview.example.test",
    }));
    let answer_position = snapshot
        .transcript
        .iter()
        .position(|item| matches!(item, TranscriptItem::Message { message_id } if *message_id == answer_id))
        .expect("the answer is in the transcript");
    snapshot.transcript.insert(
        answer_position,
        TranscriptItem::Activity {
            activity_id: newer.id(),
        },
    );
    snapshot.activities.push(newer);
    let mut application = studio_showing(snapshot);
    open_turn_folds(&mut application);

    let rows = rendered_application_rows_at(&application, 120, 30);
    assert!(
        rows.iter()
            .any(|row| row.contains("Not shown: deploy preview, from a newer Shaman")),
        "the Activity stands as a row saying what it was sent as: {rows:#?}"
    );
    assert!(
        rendered_row(&rows, "Not shown: deploy preview") < rendered_row(&rows, "Deployed the"),
        "it stands where it was recorded, before the answer after it: {rows:#?}"
    );
    assert!(
        !rows.join("\n").contains(NEWER),
        "a row holding its place is all there is to say of it: {rows:#?}"
    );

    // One arriving live stands at the Transcript's end, after the answer.
    let (mut application, snapshot) = studio_session(workspace.path(), "Answered first.");
    let turn_id = snapshot.turns[0].id;
    deliver(
        &mut application,
        next_update(
            &snapshot,
            json!([{ "type": "activity_added", "activity": {
                "kind": "canary_rollout", "id": ActivityId::new(), "turn_id": turn_id,
            } }]),
        ),
    );
    open_turn_folds(&mut application);
    let rows = rendered_application_rows_at(&application, 120, 30);
    assert!(
        rendered_row(&rows, "Answered first.")
            < rendered_row(&rows, "Not shown: canary rollout, from a newer Shaman"),
        "{rows:#?}"
    );
}

#[test]
fn a_change_a_newer_remote_sent_is_passed_over_and_said_once_quietly() {
    let workspace = workspace_dir();
    let (mut application, snapshot) = studio_session(workspace.path(), "Answered.");
    deliver(
        &mut application,
        next_update(
            &snapshot,
            json!([
                { "type": "session_mood_changed", "mood": "sunny" },
                { "type": "title_changed", "title": "Retitled by studio", "icon": null },
            ]),
        ),
    );

    let shown = screen(&application);
    assert!(
        shown.contains("Retitled by studio"),
        "the rest of the update lands: {shown}"
    );
    assert_eq!(
        shown.matches(NEWER).count(),
        1,
        "the reader is told the Remote is newer: {shown}"
    );
    assert!(
        rendered_application_rows_at(&application, 120, 30)
            .iter()
            .any(|row| row.trim() == format!("i {NEWER}")),
        "quietly, pointing nowhere: {shown}"
    );

    // Another, on the same stream or on the Remote's catalog or tree, says
    // nothing more.
    let mut revised = snapshot.clone();
    revised.revision.0 += 1;
    deliver(
        &mut application,
        next_update(
            &revised,
            json!([{ "type": "session_mood_changed", "mood": "cloudy" }]),
        ),
    );
    application
        .handle_event(ApplicationEvent::OriginCatalog {
            outlook: studio(),
            event: ManagedEvent::ChangeSkipped,
        })
        .expect("take the catalog's skipped change");
    assert_eq!(screen(&application).matches(NEWER).count(), 1);

    // Once the reader has seen and dismissed it, it does not come back.
    interact(&mut application);
    revised.revision.0 += 1;
    deliver(
        &mut application,
        next_update(
            &revised,
            json!([{ "type": "session_mood_changed", "mood": "rainy" }]),
        ),
    );
    application
        .handle_event(ApplicationEvent::SubagentTree {
            through: SessionReference::new(studio(), snapshot.session.id),
            event: SubagentTreeEvent::ChangeSkipped,
        })
        .expect("take the tree's skipped change");
    let shown = screen(&application);
    assert!(!shown.contains(NEWER), "{shown}");
}

#[test]
fn each_newer_remote_is_said_of_in_its_own_right() {
    let workspace = workspace_dir();
    let (mut application, snapshot) = studio_session(workspace.path(), "Answered.");

    // The Remote the reader is not turned toward speaks through its catalog
    // and its trees.
    application
        .handle_event(ApplicationEvent::OriginCatalog {
            outlook: Outlook::Remote("attic".to_owned()),
            event: ManagedEvent::ChangeSkipped,
        })
        .expect("take attic's skipped change");
    application
        .handle_event(ApplicationEvent::SubagentTree {
            through: SessionReference::new(studio(), snapshot.session.id),
            event: SubagentTreeEvent::ChangeSkipped,
        })
        .expect("take studio's skipped tree change");

    let shown = screen(&application);
    assert_eq!(
        shown
            .matches("attic runs a newer Shaman; some things may not show")
            .count(),
        1,
        "{shown}"
    );
    assert_eq!(shown.matches(NEWER).count(), 1, "{shown}");
}
