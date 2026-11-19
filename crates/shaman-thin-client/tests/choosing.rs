//! Choosing where a Session begins, over real Serving Servers in process:
//! the Workspace Picker listing every Remote's Workspaces, choosing one
//! turning the Outlook and leaving the listing's scope, the Worktree choice
//! the Landing offers by what the Workspace is, and a Session begun in a new
//! Worktree its Server prepares first.

mod support;

use std::path::{Path, PathBuf};

use shaman::protocol::Way;
use shaman_thin_client::{
    ActError, ItemKind, Landing, PairingOutcome, WorkspacePicker, WorkspaceRow,
};
use support::{Phone, ServingServer, WORKING};

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

/// `path` as the file system resolves it, so a temporary directory reached
/// through a link reads as itself.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).expect("the directory stands")
}

fn git(directory: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Shaman")
        .env("GIT_AUTHOR_EMAIL", "shaman@example.com")
        .env("GIT_COMMITTER_NAME", "Shaman")
        .env("GIT_COMMITTER_EMAIL", "shaman@example.com")
        .status()
        .expect("run git");
    assert!(status.success(), "git {arguments:?} failed");
}

/// A Repository named `name` in `root`, on the branch `branch` with a first
/// commit.
fn repository(root: &Path, name: &str, branch: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(&path).expect("make the Repository");
    git(&path, &["init", "-b", branch]);
    git(
        &path,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    path
}

/// A bare Repository named `name` in `root`.
fn bare(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    git(
        root,
        &["init", "--bare", "-b", "main", &path.to_string_lossy()],
    );
    path
}

/// The directory the Workspace Roots of a test's Server name, canonical so
/// what the Server lists reads as what the test made.
fn roots() -> (tempfile::TempDir, PathBuf) {
    let holder = tempfile::tempdir().expect("make the Workspace Root");
    let root = shaman::paths::canonical(holder.path()).expect("the Workspace Root stands");
    (holder, root)
}

fn row<'a>(
    picker: &'a WorkspacePicker,
    remote: Option<&str>,
    name: &str,
) -> Option<&'a WorkspaceRow> {
    picker
        .sections
        .iter()
        .filter(|section| remote.is_none_or(|remote| section.header.as_deref() == Some(remote)))
        .flat_map(|section| &section.rows)
        .find(|row| row.name == name)
}

fn names(picker: &WorkspacePicker) -> Vec<(Option<String>, Vec<String>)> {
    picker
        .sections
        .iter()
        .map(|section| {
            (
                section.header.clone(),
                section.rows.iter().map(|row| row.name.clone()).collect(),
            )
        })
        .collect()
}

/// Turns the Outlook toward the Workspace `name`, as choosing its row of the
/// picker does, and waits on the Landing standing there.
async fn choose(phone: &Phone, remote: Option<&str>, name: &str) -> Landing {
    let picker = phone
        .workspaces_until("", |picker| row(picker, remote, name).is_some())
        .await;
    let chosen = row(&picker, remote, name)
        .expect("offered")
        .workspace
        .clone();
    phone.core.choose_workspace(chosen).await.expect("chosen");
    phone
        .landing_until(|landing| {
            landing
                .workspace
                .as_ref()
                .is_some_and(|workspace| workspace.name == name)
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_picker_lists_every_remotes_workspaces_and_choosing_one_turns_the_outlook_alone() {
    let desk = ServingServer::start_offering_a_model("thin-choose-desk").await;
    let lab = ServingServer::start("thin-choose-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let (_desk_holder, desk_root) = roots();
    repository(&desk_root, "shaman", "main");
    repository(&desk_root, "dotfiles", "main");
    desk.set_workspace_roots(&[&desk_root]).await;
    let (_lab_holder, lab_root) = roots();
    repository(&lab_root, "home-assistant", "main");
    lab.set_workspace_roots(&[&lab_root]).await;

    let picker = phone
        .workspaces_until("", |picker| {
            row(picker, Some("desk"), "dotfiles").is_some()
                && row(picker, Some("lab"), "home-assistant").is_some()
        })
        .await;
    assert_eq!(picker.title, "Begin in");
    assert_eq!(
        names(&picker),
        [
            (
                Some("desk".to_owned()),
                vec!["dotfiles".to_owned(), "shaman".to_owned()]
            ),
            (Some("lab".to_owned()), vec!["home-assistant".to_owned()]),
        ],
        "under a header for each Remote"
    );
    assert!(
        picker
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .all(|row| !row.chosen),
        "the Outlook, at the Remote paired last, stands in no Workspace yet"
    );

    // The listing is narrowed to the lab, and stays so.
    let listing = phone
        .listing_until(|listing| {
            listing
                .scopes
                .iter()
                .flat_map(|section| &section.choices)
                .any(|choice| choice.words == "All of lab")
        })
        .await;
    let lab_whole = listing
        .scopes
        .iter()
        .flat_map(|section| &section.choices)
        .find(|choice| choice.words == "All of lab")
        .expect("offered")
        .scope
        .clone();
    phone
        .core
        .narrow_listing(lab_whole)
        .await
        .expect("narrowed");
    phone.listing_until(|listing| listing.title == "lab").await;
    let landing = phone.landing_until(|landing| landing.agent.is_some()).await;
    assert_eq!(landing.agent.as_deref(), Some("Agent unavailable"));

    let landing = choose(&phone, Some("desk"), "shaman").await;
    assert_eq!(landing.place, "desk · shaman");
    assert_eq!(landing.elsewhere.as_deref(), Some("Begin somewhere else"));
    phone
        .landing_until(|landing| landing.agent.as_deref() == Some("none · Scripted"))
        .await;
    let picker = phone
        .workspaces_until("", |picker| {
            row(picker, Some("desk"), "shaman").is_some_and(|row| row.chosen)
        })
        .await;
    assert!(!row(&picker, Some("desk"), "dotfiles").unwrap().chosen);
    assert_eq!(
        phone.listing_until(|_| true).await.title,
        "lab",
        "the listing's scope stays where it was"
    );

    let searched = phone
        .workspaces_until("HOME", |picker| picker.sections.len() == 1)
        .await;
    assert_eq!(
        names(&searched),
        [(Some("lab".to_owned()), vec!["home-assistant".to_owned()])]
    );
    let nothing = phone
        .workspaces_until("kernel", |picker| picker.empty.is_some())
        .await;
    assert_eq!(nothing.empty.as_deref(), Some("No workspaces found"));
    assert!(nothing.sections.is_empty());

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_whose_server_cannot_read_it_is_dimmed_and_cannot_be_chosen() {
    let desk = ServingServer::start("thin-choose-gone").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let (_holder, root) = roots();
    let gone = root.join("photo-archive");
    std::fs::create_dir_all(&gone).expect("make the Workspace");
    desk.begin(&gone, "Sort the photos").await;
    std::fs::remove_dir_all(&gone).expect("take the Workspace's directory away");
    // Each listing has the Server read every directory behind it, which the
    // listing after says, so the picker asks afresh each time it opens.
    let dimmed = |picker: &WorkspacePicker| {
        row(picker, None, "photo-archive").is_some_and(|row| row.unavailable.is_some())
    };
    let mut opened = 0;
    let picker = loop {
        if let Some(picker) = phone
            .workspaces_within(std::time::Duration::from_millis(200), dimmed)
            .await
        {
            break picker;
        }
        opened += 1;
        assert!(opened < 100, "the picker never dimmed the Workspace");
    };
    let archive = row(&picker, None, "photo-archive").expect("listed");
    assert_eq!(
        archive.unavailable.as_deref(),
        Some("Unavailable: its directory is gone")
    );
    let refused = phone
        .core
        .choose_workspace(archive.workspace.clone())
        .await
        .expect_err("refused");
    assert_eq!(
        refused,
        ActError::Invalid {
            message: "Unavailable: its directory is gone".to_owned()
        }
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_worktree_choice_names_the_main_worktree_and_a_workspace_outside_one_offers_none() {
    let desk = ServingServer::start("thin-choose-worktree").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let (_holder, root) = roots();
    repository(&root, "shaman", "develop");
    bare(&root, "mirror.git");
    let notes = root.join("notes");
    std::fs::create_dir_all(&notes).expect("make the Workspace");
    desk.begin(&notes, "Gather the notes").await;
    desk.set_workspace_roots(&[&root]).await;

    choose(&phone, None, "shaman").await;
    let landing = phone
        .landing_until(|landing| {
            landing
                .worktree
                .as_ref()
                .is_some_and(|choice| choice.main != "On the main worktree")
        })
        .await;
    let choice = landing.worktree.expect("a Worktree choice");
    assert_eq!(choice.main, "On develop", "its Checkout State");
    assert_eq!(choice.new, "In a new worktree");
    assert_eq!(landing.working_copy, None);
    assert!(landing.can_send);

    let landing = choose(&phone, None, "notes").await;
    assert_eq!(landing.worktree, None, "outside source control");
    assert_eq!(landing.working_copy, None);
    assert!(landing.can_send);
    let refused = phone
        .core
        .begin_session("Gather more".to_owned(), true)
        .err()
        .expect("refused");
    assert!(matches!(refused, ActError::Invalid { .. }), "{refused:?}");

    choose(&phone, None, "mirror.git").await;
    let landing = phone
        .landing_until(|landing| landing.working_copy.is_some())
        .await;
    assert_eq!(landing.worktree, None, "a bare Repository");
    assert!(!landing.can_send, "it has no main Worktree to begin on");
    let refused = phone
        .core
        .begin_session("Mirror the tags".to_owned(), false)
        .err()
        .expect("refused");
    assert_eq!(
        refused,
        ActError::Invalid {
            message: landing.working_copy.clone().unwrap()
        }
    );

    phone.shutdown();
    desk.shutdown().await;
}

fn refused(items: &[shaman_thin_client::ConversationItem]) -> Option<String> {
    items.iter().find_map(|item| match &item.kind {
        ItemKind::Refused { words, .. } => Some(words.clone()),
        _ => None,
    })
}

fn session_id(reference: &str) -> shaman::protocol::SessionId {
    let (_, id) = reference.split_once('/').expect("a reference");
    serde_json::from_value(serde_json::Value::String(id.to_owned())).expect("a Session id")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_begun_in_a_new_worktree_has_it_prepared_first() {
    let desk = ServingServer::start_offering_a_model("thin-choose-new-worktree").await;
    let gate = support::Gate::open(desk.serving_address()).await;
    let phone = Phone::new();
    let invite = desk.invite(vec![Way::Direct(gate.address())]).await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    let (_holder, root) = roots();
    let main = repository(&root, WORKING, "main");
    desk.set_workspace_roots(&[&root]).await;
    choose(&phone, None, WORKING).await;
    phone
        .landing_until(|landing| landing.worktree.is_some() && landing.agent.is_some())
        .await;

    // The Server is not reached until the way opens, so the preparation is
    // seen as it stands.
    gate.shut_now();
    let reading = phone.begin_in_new_worktree("Tidy the build");
    let frame = reading.frame().await;
    let bar = frame.bar.expect("a bar");
    assert_eq!(
        (bar.words.as_str(), bar.since, bar.stop),
        ("Creating worktree", None, None)
    );
    gate.open_now();
    let frame = reading
        .frame_until(|frame| frame.bar.as_ref().is_some_and(|bar| bar.since.is_some()))
        .await;
    assert_eq!(frame.bar.expect("a bar").words, "Working");

    let held = desk.read(session_id(&reading.session.reference())).await;
    let directory = canonical(&held.session.execution_directory.path);
    assert_ne!(directory, canonical(&main), "not the main Worktree");
    assert!(
        directory.join(".git").is_file(),
        "a linked Worktree of the Repository: {}",
        directory.display()
    );
    phone
        .listing_until(|listing| {
            listing
                .active
                .iter()
                .any(|row| row.session == reading.session.reference())
        })
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_worktree_where_the_server_names_no_agent_is_refused_and_nothing_begins() {
    let desk = ServingServer::start("thin-choose-no-agent").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let (_holder, root) = roots();
    repository(&root, WORKING, "main");
    desk.set_workspace_roots(&[&root]).await;
    choose(&phone, None, WORKING).await;
    phone
        .landing_until(|landing| landing.worktree.is_some())
        .await;

    let reading = phone.begin_in_new_worktree("Tidy the build");
    let conversation = reading
        .conversation_until(|items| refused(items).is_some())
        .await;
    assert_eq!(
        refused(&conversation.items).as_deref(),
        Some("Could not create session: the server names no agent to prepare a new worktree for")
    );
    phone.acts_until(false).await;

    phone.shutdown();
    desk.shutdown().await;
}
