//! Each composer's unsent text, by the Session it is written in, and the
//! Landing's under a key of its own, kept from one run to the next.
//!
//! Swift reads a draft when its screen appears and hands the core each change.
//! What the core puts back is text it took away: a Prompt whose send failed, or
//! one an interrupt withdrew, which stands ahead of anything written meanwhile.
//! That is the one change the core makes on its own, and it always wins: each
//! is a new version, which the core tells Swift of, and a change Swift hands in
//! written over an older version has what was given back since kept ahead of
//! it, however many keystrokes reach the core before Swift hears.
//!
//! Every draft is kept as text alone in one file of the core's directory,
//! written a moment after the last change and at once as the app leaves the
//! front, so it is there after iOS ends the process. A Prompt on its way is
//! kept in its composer's draft, ahead of what is written there, until its
//! Server answers: so the text of one whose answer never came is in the
//! composer the next run. A draft is dropped as its Prompt is answered, as its
//! Remote is no longer held, and as its Session is absent from a listing its
//! Remote answered with, asked after the draft was last written; a Remote that
//! does not answer drops nothing. Nothing dropped is told to Swift, so the core
//! never puts text into a field being edited.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use shaman_protocol::SessionId;
use tokio::sync::{Notify, watch};

/// The file of drafts in the core's directory.
pub(crate) const DRAFTS_FILE: &str = "drafts.json";

/// A composer's draft as Swift holds it: its text, and the version of it the
/// core last told of, which Swift hands back with each change.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct Draft {
    pub text: String,
    pub version: u64,
}

/// One composer's draft as the core holds it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Held {
    text: String,
    version: u64,
    /// The text given back at each version Swift may not have heard of yet.
    given: Vec<(u64, String)>,
    /// The Prompts taken from it and on their way, kept ahead of its text
    /// until their Server answers.
    sending: Vec<(Sent, String)>,
    /// When it was last changed in this run; none where it is as the file
    /// kept it.
    touched: Option<Instant>,
}

impl Held {
    /// The draft as Swift holds it.
    pub fn draft(&self) -> Draft {
        Draft {
            text: self.text.clone(),
            version: self.version,
        }
    }

    /// What the file keeps of it: each Prompt on its way ahead of what is
    /// written, the latest first.
    fn kept(&self) -> String {
        self.sending
            .iter()
            .fold(self.text.clone(), |written, (_, sent)| {
                ahead(sent, &written)
            })
    }

    /// Empties it without a word to Swift, as a draft dropped is.
    fn dropped(&mut self) {
        self.text.clear();
        self.given.clear();
        self.sending.clear();
    }
}

/// A Prompt taken from a composer and on its way, kept in its draft until its
/// Server answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sent(u64);

/// Every composer's draft, held for the core's life and kept in its
/// directory.
#[derive(Clone, Default)]
pub(crate) struct Drafts {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    held: Mutex<HashMap<String, watch::Sender<Held>>>,
    /// The last Prompt on its way marked.
    sent: AtomicU64,
    /// Where they are kept, where they are: none in a test of what is held
    /// alone.
    kept: Option<Kept>,
}

/// The file the drafts are kept in, and what writes it.
struct Kept {
    path: PathBuf,
    /// Told of each change to be kept.
    changed: Notify,
    /// Held through each write, so a later write never lands under an
    /// earlier one.
    writing: Mutex<()>,
}

impl Drafts {
    /// The drafts kept in `directory`, each change written once `after` has
    /// passed without another, on `runtime`. A file that cannot be read keeps
    /// nothing: no draft is worth refusing to open over.
    pub fn open(directory: &Path, after: Duration, runtime: &tokio::runtime::Handle) -> Self {
        let path = directory.join(DRAFTS_FILE);
        let read: BTreeMap<String, String> =
            shaman_pairing::read_records(&path).unwrap_or_else(|error| {
                tracing::warn!("the drafts kept were not read: {error:#}");
                BTreeMap::new()
            });
        let held = read
            .into_iter()
            .map(|(key, text)| {
                let held = Held {
                    text,
                    ..Held::default()
                };
                (key, watch::Sender::new(held))
            })
            .collect();
        let shared = Arc::new(Shared {
            held: Mutex::new(held),
            sent: AtomicU64::new(0),
            kept: Some(Kept {
                path,
                changed: Notify::new(),
                writing: Mutex::new(()),
            }),
        });
        runtime.spawn(write_behind(shared.clone(), after));
        Self { shared }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, watch::Sender<Held>>> {
        self.shared
            .held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The draft of the composer `key` names, made empty where it has none.
    fn of(&self, key: &str) -> watch::Sender<Held> {
        self.lock()
            .entry(key.to_owned())
            .or_insert_with(|| watch::Sender::new(Held::default()))
            .clone()
    }

    /// Has what changed written a moment from now, once nothing more has.
    fn changed(&self) {
        if let Some(kept) = &self.shared.kept {
            kept.changed.notify_one();
        }
    }

    /// Writes every draft now, as the app leaves the front.
    pub fn write_now(&self) {
        self.shared.write();
    }

    /// The draft of the composer `key` names, empty where it has none.
    pub fn get(&self, key: &str) -> Draft {
        self.of(key).borrow().draft()
    }

    /// The draft of the composer `key` names, told again each time the core
    /// changes it on its own, which Swift is to take into the field.
    pub fn changes(&self, key: &str) -> watch::Receiver<Held> {
        self.of(key).subscribe()
    }

    /// Holds `text`, written over the version `seen` of the draft, as the
    /// draft of the composer `key` names. Where the core has given text back
    /// since `seen`, it stands ahead of `text`, and Swift is told.
    pub fn set(&self, key: &str, text: String, seen: u64) {
        self.of(key).send_if_modified(|held| {
            held.touched = Some(Instant::now());
            let unseen = held
                .given
                .iter()
                .filter(|(version, _)| *version > seen)
                .map(|(_, given)| given.clone())
                .collect::<Vec<_>>();
            if unseen.is_empty() {
                held.given.clear();
                held.text = text;
                // Swift's own change: it holds it already.
                return false;
            }
            // Each is put ahead in turn, so the latest stands first.
            held.text = unseen
                .into_iter()
                .fold(text, |written, given| ahead(&given, &written));
            true
        });
        self.changed();
    }

    /// Gives `text`, which a send took from the composer `key` names, back
    /// to it, ahead of anything written there since, and tells Swift.
    pub fn give_back(&self, key: &str, text: &str) {
        self.of(key).send_modify(|held| give(held, text));
        self.changed();
    }

    /// Marks `text`, taken from the composer `key` names, as on its way: it
    /// is kept in that draft until [`Self::answered`] or
    /// [`Self::give_back_sent`] is told of it.
    pub fn sending(&self, key: &str, text: &str) -> Sent {
        let sent = Sent(self.shared.sent.fetch_add(1, Ordering::Relaxed) + 1);
        self.of(key).send_if_modified(|held| {
            held.sending.push((sent, text.to_owned()));
            false
        });
        self.changed();
        sent
    }

    /// Has `text` take the place of the Prompt `sent` on its way from the
    /// composer `key` names, as a Prompt written instead does.
    pub fn resending(&self, key: &str, sent: Sent, text: &str) {
        self.of(key).send_if_modified(|held| {
            if let Some((_, held)) = held.sending.iter_mut().find(|(on, _)| *on == sent) {
                text.clone_into(held);
            }
            false
        });
        self.changed();
    }

    /// The Prompt `sent` from the composer `key` names was answered: the
    /// draft keeps it no more.
    pub fn answered(&self, key: &str, sent: Sent) {
        let mut was = false;
        self.of(key).send_if_modified(|held| {
            let before = held.sending.len();
            held.sending.retain(|(on, _)| *on != sent);
            was = held.sending.len() != before;
            false
        });
        if was {
            self.changed();
        }
    }

    /// Gives `text` back to the composer `key` names, as
    /// [`Self::give_back`] does, as what the Prompt `sent` on its way from it
    /// became: kept in its draft throughout.
    pub fn give_back_sent(&self, key: &str, sent: Sent, text: &str) {
        self.of(key).send_modify(|held| {
            held.sending.retain(|(on, _)| *on != sent);
            give(held, text);
        });
        self.changed();
    }

    /// Drops every draft of a Session on a Remote other than those whose
    /// keys `held` holds, as a Remote removed drops its drafts.
    pub fn keep_remotes(&self, held: &[String]) {
        self.drop_where(|fingerprint, _, _| !held.iter().any(|kept| kept == fingerprint));
    }

    /// Drops the draft of every Session of the Remote `fingerprint` that its
    /// listing, asked for at `asked`, does not hold, where it is not written
    /// since and holds no Prompt on its way.
    pub fn listed(&self, fingerprint: &str, holds: impl Fn(SessionId) -> bool, asked: Instant) {
        self.drop_where(|remote, session, held| {
            remote == fingerprint
                && !holds(session)
                && held.sending.is_empty()
                && held.touched.is_none_or(|touched| touched < asked)
        });
    }

    /// Drops every draft of a Session `drop` says to, without a word to
    /// Swift.
    fn drop_where(&self, drop: impl Fn(&str, SessionId, &Held) -> bool) {
        let mut any = false;
        for (key, held) in self.lock().iter() {
            let Some((fingerprint, session)) = crate::listing::referenced(key) else {
                // The Landing's, of no Session.
                continue;
            };
            held.send_if_modified(|held| {
                let empty = held.text.is_empty() && held.sending.is_empty();
                if !empty && drop(fingerprint, session, held) {
                    held.dropped();
                    any = true;
                }
                false
            });
        }
        if any {
            self.changed();
        }
    }
}

impl Shared {
    /// What the file is to keep: every draft with anything in it.
    fn kept(&self) -> BTreeMap<String, String> {
        self.held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .map(|(key, held)| (key.clone(), held.borrow().kept()))
            .filter(|(_, text)| !text.trim().is_empty())
            .collect()
    }

    /// Writes every draft to the file, whole or not at all, or removes it
    /// where none has anything in it.
    fn write(&self) {
        let Some(kept) = &self.kept else {
            return;
        };
        let _writing = kept
            .writing
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let drafts = self.kept();
        let written = if drafts.is_empty() {
            match std::fs::remove_file(&kept.path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            }
        } else {
            shaman_pairing::write_private_json(&kept.path, &drafts)
        };
        if let Err(error) = written {
            tracing::warn!("the drafts were not kept: {error:#}");
        }
    }
}

/// Writes the drafts each time `after` passes without a change once one is
/// told of, for as long as the core runs.
async fn write_behind(shared: Arc<Shared>, after: Duration) {
    let Some(kept) = &shared.kept else {
        return;
    };
    loop {
        kept.changed.notified().await;
        // A moment after the last change: each one more waits afresh.
        loop {
            tokio::select! {
                () = kept.changed.notified() => {}
                () = tokio::time::sleep(after) => break,
            }
        }
        let writing = shared.clone();
        if let Err(error) = tokio::task::spawn_blocking(move || writing.write()).await {
            tracing::warn!("the drafts were not kept: {error}");
        }
    }
}

/// Puts `text` back in `held`, ahead of what is written, as a new version
/// Swift is to hear of.
fn give(held: &mut Held, text: &str) {
    held.version += 1;
    held.given.push((held.version, text.to_owned()));
    held.text = ahead(text, &held.text);
    held.touched = Some(Instant::now());
}

/// `given` ahead of `written`, which nothing but blanks counts as nothing.
fn ahead(given: &str, written: &str) -> String {
    if written.trim().is_empty() {
        given.to_owned()
    } else {
        format!("{given}\n{written}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_is_what_was_last_handed_in_and_empty_where_none_was() {
        let drafts = Drafts::default();
        assert_eq!(drafts.get("desk/1"), Draft::default());
        drafts.set("desk/1", "half a thought".to_owned(), 0);
        drafts.set("desk/2", "another".to_owned(), 0);
        assert_eq!(drafts.get("desk/1").text, "half a thought");
        drafts.set("desk/1", String::new(), 0);
        assert_eq!(drafts.get("desk/1").text, "");
        assert_eq!(drafts.get("desk/2").text, "another");
    }

    #[test]
    fn text_given_back_stands_ahead_of_anything_written_since() {
        let drafts = Drafts::default();
        drafts.give_back("desk/1", "Tidy the build");
        assert_eq!(
            drafts.get("desk/1"),
            Draft {
                text: "Tidy the build".to_owned(),
                version: 1
            }
        );

        drafts.set("desk/2", "and then".to_owned(), 0);
        drafts.give_back("desk/2", "Tidy the build");
        assert_eq!(drafts.get("desk/2").text, "Tidy the build\nand then");

        drafts.set("desk/3", "  ".to_owned(), 0);
        drafts.give_back("desk/3", "Tidy the build");
        assert_eq!(
            drafts.get("desk/3").text,
            "Tidy the build",
            "nothing but blanks is nothing written"
        );
    }

    #[test]
    fn keystrokes_written_over_a_version_before_a_give_back_keep_what_was_given_back_ahead() {
        let drafts = Drafts::default();
        drafts.set("desk/1", "and then".to_owned(), 0);
        drafts.give_back("desk/1", "Tidy the build");
        drafts.give_back("desk/1", "Steer it");

        // Swift has heard of neither, and goes on typing.
        drafts.set("desk/1", "and then t".to_owned(), 0);
        assert_eq!(
            drafts.get("desk/1").text,
            "Steer it\nTidy the build\nand then t"
        );
        drafts.set("desk/1", "and then th".to_owned(), 0);
        assert_eq!(
            drafts.get("desk/1").text,
            "Steer it\nTidy the build\nand then th",
            "given back once however often it is written over"
        );
        // It has heard of the first alone.
        drafts.set("desk/1", "Tidy the build\nand then th".to_owned(), 1);
        assert_eq!(
            drafts.get("desk/1").text,
            "Steer it\nTidy the build\nand then th"
        );

        // Caught up, its change is taken as it stands.
        let caught_up = drafts.get("desk/1");
        assert_eq!(caught_up.version, 2);
        drafts.set("desk/1", "Steer it".to_owned(), 2);
        assert_eq!(drafts.get("desk/1").text, "Steer it");
        drafts.set("desk/1", "Steer it now".to_owned(), 0);
        assert_eq!(
            drafts.get("desk/1").text,
            "Steer it now",
            "nothing given back is owed once Swift caught up"
        );
    }

    #[tokio::test]
    async fn swift_is_told_only_of_what_the_core_changes_on_its_own() {
        let drafts = Drafts::default();
        let mut changes = drafts.changes("desk/1");
        assert_eq!(changes.borrow_and_update().draft().text, "");

        drafts.set("desk/1", "typed".to_owned(), 0);
        tokio::task::yield_now().await;
        assert!(
            !changes.has_changed().unwrap(),
            "Swift holds its own change"
        );

        drafts.give_back("desk/1", "Tidy the build");
        changes.changed().await.unwrap();
        assert_eq!(
            changes.borrow_and_update().draft(),
            Draft {
                text: "Tidy the build\ntyped".to_owned(),
                version: 1
            }
        );

        drafts.set("desk/1", "typed!".to_owned(), 0);
        changes.changed().await.unwrap();
        assert_eq!(
            changes.borrow_and_update().draft().text,
            "Tidy the build\ntyped!",
            "a change over an older version is told back with what was given"
        );
    }

    /// A Session's key on the Remote `fingerprint`, as Swift hands it in.
    fn key(fingerprint: &str, session: SessionId) -> String {
        format!("{fingerprint}/{session}")
    }

    /// What the file in `directory` keeps, by key.
    fn on_disk(directory: &Path) -> BTreeMap<String, String> {
        shaman_pairing::read_records(&directory.join(DRAFTS_FILE)).unwrap()
    }

    /// What the file in `directory` keeps once it keeps what `kept` looks
    /// for, as it is written a moment after the last change.
    async fn on_disk_until(
        directory: &Path,
        kept: impl Fn(&BTreeMap<String, String>) -> bool,
    ) -> BTreeMap<String, String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(drafts) = shaman_pairing::read_records(&directory.join(DRAFTS_FILE))
                    && kept(&drafts)
                {
                    return drafts;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("the drafts were kept in time")
    }

    /// A moment far longer than any test waits.
    const NEVER: Duration = Duration::from_secs(3_600);

    #[tokio::test(flavor = "multi_thread")]
    async fn drafts_are_written_a_moment_after_the_last_change_as_text_alone() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(
            directory.path(),
            Duration::from_millis(30),
            &tokio::runtime::Handle::current(),
        );
        let session = key("desk", SessionId::new());
        drafts.set(&session, "half".to_owned(), 0);
        drafts.set(&session, "half a thought".to_owned(), 0);
        drafts.set(beginning_key(), "a new one".to_owned(), 0);
        assert!(!directory.path().join(DRAFTS_FILE).exists(), "not at once");

        let kept = on_disk_until(directory.path(), |kept| !kept.is_empty()).await;
        assert_eq!(
            kept,
            BTreeMap::from([
                (session.clone(), "half a thought".to_owned()),
                (beginning_key().to_owned(), "a new one".to_owned()),
            ])
        );
        let reopened = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        assert_eq!(
            reopened.get(&session),
            Draft {
                text: "half a thought".to_owned(),
                version: 0
            }
        );

        drafts.set(&session, " ".to_owned(), 0);
        drafts.set(beginning_key(), String::new(), 0);
        tokio::time::timeout(Duration::from_secs(5), async {
            while directory.path().join(DRAFTS_FILE).exists() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("a file of nothing goes");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn typing_on_puts_the_write_off_and_writing_now_writes_at_once() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(
            directory.path(),
            Duration::from_millis(80),
            &tokio::runtime::Handle::current(),
        );
        let session = key("desk", SessionId::new());
        for typed in ["T", "Ti", "Tid", "Tidy", "Tidy "] {
            drafts.set(&session, typed.to_owned(), 0);
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert!(
            !directory.path().join(DRAFTS_FILE).exists(),
            "nothing is written while typing goes on"
        );
        let kept = on_disk_until(directory.path(), |kept| !kept.is_empty()).await;
        assert_eq!(kept.get(&session).map(String::as_str), Some("Tidy "));

        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        drafts.set(&session, "Tidy the build".to_owned(), 0);
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path()).get(&session).map(String::as_str),
            Some("Tidy the build")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_prompt_on_its_way_is_kept_in_its_draft_until_it_is_answered() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        let session = key("desk", SessionId::new());
        drafts.set(&session, "Tidy the build".to_owned(), 0);
        // The field empties as it sends, and is written in again.
        drafts.set(&session, String::new(), 0);
        let sent = drafts.sending(&session, "Tidy the build");
        drafts.set(&session, "and the docs".to_owned(), 0);
        assert_eq!(drafts.get(&session).text, "and the docs", "Swift's own");
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path()).get(&session).map(String::as_str),
            Some("Tidy the build\nand the docs"),
            "kept ahead of what is written, should the app end before it is answered"
        );

        drafts.answered(&session, sent);
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path()).get(&session).map(String::as_str),
            Some("and the docs")
        );
        drafts.set(&session, String::new(), 0);
        drafts.write_now();
        assert_eq!(on_disk(directory.path()), BTreeMap::new());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_prompt_on_its_way_given_back_is_kept_once() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        let landing = beginning_key();
        let sent = drafts.sending(landing, "Tidy the build");
        drafts.resending(landing, sent, "Tidy the whole build");
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path()).get(landing).map(String::as_str),
            Some("Tidy the whole build"),
            "a Prompt written instead takes its place"
        );

        drafts.give_back_sent(landing, sent, "Tidy the whole build");
        assert_eq!(
            drafts.get(landing),
            Draft {
                text: "Tidy the whole build".to_owned(),
                version: 1
            }
        );
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path()).get(landing).map(String::as_str),
            Some("Tidy the whole build")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_remote_no_longer_held_drops_its_drafts_and_no_others() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        let (desk, lab) = (key("desk", SessionId::new()), key("lab", SessionId::new()));
        drafts.set(&desk, "on the desk".to_owned(), 0);
        drafts.set(&lab, "in the lab".to_owned(), 0);
        drafts.sending(&lab, "on its way to the lab");
        drafts.set(beginning_key(), "a new one".to_owned(), 0);
        let mut told = drafts.changes(&lab);
        told.borrow_and_update();

        drafts.keep_remotes(&["desk".to_owned()]);
        assert_eq!(drafts.get(&lab).text, "");
        assert!(
            !told.has_changed().unwrap(),
            "nothing is pushed into a field"
        );
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path()),
            BTreeMap::from([
                (desk, "on the desk".to_owned()),
                (beginning_key().to_owned(), "a new one".to_owned()),
            ])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_session_absent_from_a_listing_asked_after_its_draft_was_written_drops_it() {
        let directory = tempfile::tempdir().unwrap();
        let (gone, listed, sending, typed_since, elsewhere) = (
            SessionId::new(),
            SessionId::new(),
            SessionId::new(),
            SessionId::new(),
            SessionId::new(),
        );
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        for session in [gone, listed, sending, typed_since] {
            drafts.set(&key("desk", session), "written".to_owned(), 0);
        }
        drafts.set(&key("lab", elsewhere), "written".to_owned(), 0);
        drafts.write_now();
        // The next run, before any listing is asked.
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        drafts.sending(&key("desk", sending), "on its way");
        let asked = Instant::now();
        drafts.set(&key("desk", typed_since), "written since".to_owned(), 0);
        let mut told = drafts.changes(&key("desk", gone));
        told.borrow_and_update();

        drafts.listed("desk", |session| session == listed, asked);
        assert_eq!(drafts.get(&key("desk", gone)).text, "");
        assert!(
            !told.has_changed().unwrap(),
            "nothing is pushed into a field"
        );
        for (kept, why) in [
            (key("desk", listed), "listed"),
            (
                key("desk", sending),
                "a Prompt on its way is kept until answered",
            ),
            (
                key("desk", typed_since),
                "written since the listing was asked",
            ),
            (key("lab", elsewhere), "of another Remote"),
        ] {
            assert_ne!(drafts.get(&kept).text, "", "{why}");
        }
        drafts.write_now();
        assert!(!on_disk(directory.path()).contains_key(&key("desk", gone)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_file_that_cannot_be_read_keeps_nothing_and_is_written_over() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(DRAFTS_FILE), b"{ not json").unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        assert_eq!(drafts.get(beginning_key()), Draft::default());
        drafts.set(beginning_key(), "a new one".to_owned(), 0);
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path())
                .get(beginning_key())
                .map(String::as_str),
            Some("a new one")
        );
    }

    fn beginning_key() -> &'static str {
        crate::beginning::LANDING_DRAFT
    }
}
