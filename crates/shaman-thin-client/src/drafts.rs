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
//! Every draft is kept as its text in one file of the core's directory,
//! written a moment after the last change, at once as an answer changes it,
//! and at once as the app leaves the front, so it is there after iOS ends the
//! process. A Prompt on its way is kept in its composer's draft, ahead of what
//! is written there, until its Server answers: so the text of one whose answer
//! never came is in the composer the next run, saying it may have been sent,
//! as one given back unanswered says it.
//!
//! A draft is dropped as its Prompt is answered, as the Remote it was written
//! for is no longer held — the Landing's, for the Remote the Outlook stood at
//! as it was written — and as its Session is absent from a listing its Remote
//! answered with, asked after the draft was last written, once a listing has
//! held a Session being begun; a Remote that does not answer drops nothing.
//! Nothing is kept for a Remote no longer held, and nothing dropped is told to
//! Swift, so the core never puts text into a field being edited.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
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
    sending: Vec<OnItsWay>,
    /// When it was last changed in this run; none where it is as the file
    /// kept it.
    touched: Option<Instant>,
    /// The Remote it was written for, by its key, where its key names none:
    /// the Landing's, written where the Outlook stood.
    remote: Option<String>,
    /// Whether what it holds may have been sent already: given back from a
    /// send whose answer never came, or on its way as the app was ended.
    maybe_sent: bool,
}

/// A Prompt on its way from a composer.
#[derive(Clone, Debug)]
struct OnItsWay {
    sent: Sent,
    text: String,
    /// The Remote it went to, where the composer's key names none.
    remote: Option<String>,
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
    /// written, the latest first, which may have been sent should the app
    /// end before it is answered.
    fn kept(&self) -> KeptDraft {
        KeptDraft {
            text: self
                .sending
                .iter()
                .fold(self.text.clone(), |written, on| ahead(&on.text, &written)),
            remote: self
                .remote
                .clone()
                .or_else(|| self.sending.iter().rev().find_map(|on| on.remote.clone())),
            maybe_sent: self.maybe_sent || !self.sending.is_empty(),
        }
    }

    /// Empties it without a word to Swift, as a draft dropped is.
    fn dropped(&mut self) {
        self.text.clear();
        self.given.clear();
        self.sending.clear();
        self.remote = None;
        self.maybe_sent = false;
    }
}

/// A draft as the file keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
struct KeptDraft {
    text: String,
    /// The Remote it was written for, by its key, where its key names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<String>,
    /// Whether it may have been sent already.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    maybe_sent: bool,
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
    /// The Remotes held, by key, once the core has said which: nothing is
    /// kept for another.
    remotes: Mutex<Option<HashSet<String>>>,
    /// The Sessions being begun, by key, which no listing drops a draft of
    /// until one has held them.
    beginning: Mutex<HashSet<String>>,
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
        let read: BTreeMap<String, KeptDraft> =
            shaman_pairing::read_records(&path).unwrap_or_else(|error| {
                tracing::warn!("the drafts kept were not read: {error:#}");
                BTreeMap::new()
            });
        let held = read
            .into_iter()
            .map(|(key, kept)| {
                let held = Held {
                    text: kept.text,
                    remote: kept.remote,
                    maybe_sent: kept.maybe_sent,
                    ..Held::default()
                };
                (key, watch::Sender::new(held))
            })
            .collect();
        let shared = Arc::new(Shared {
            held: Mutex::new(held),
            kept: Some(Kept {
                path,
                changed: Notify::new(),
                writing: Mutex::new(()),
            }),
            ..Shared::default()
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

    /// Writes every draft now: as the app leaves the front, as an answer
    /// changes what is kept, and before Swift is told no act is under way.
    pub fn write_now(&self) {
        self.shared.write();
    }

    /// The draft of the composer `key` names, empty where it has none.
    pub fn get(&self, key: &str) -> Draft {
        self.of(key).borrow().draft()
    }

    /// Whether what the composer `key` names holds may have been sent
    /// already, which its screen says beside it.
    pub fn maybe_sent(&self, key: &str) -> bool {
        self.of(key).borrow().maybe_sent
    }

    /// The draft of the composer `key` names, told again each time the core
    /// changes it on its own, which Swift is to take into the field.
    pub fn changes(&self, key: &str) -> watch::Receiver<Held> {
        self.of(key).subscribe()
    }

    /// [`Self::set_for`] for a composer whose key names its Remote.
    #[cfg(test)]
    pub fn set(&self, key: &str, text: String, seen: u64) {
        self.set_for(key, text, seen, None);
    }

    /// Holds `text`, written over the version `seen` of the draft, as the
    /// draft of the composer `key` names. Where the core has given text back
    /// since `seen`, it stands ahead of `text`, and Swift is told. Where the
    /// key names no Remote, it is written for the one `remote` names by its
    /// key, as the Landing's is written for where the Outlook stands; emptied,
    /// it is written for none, and what it held is no longer said to have
    /// maybe been sent.
    pub fn set_for(&self, key: &str, text: String, seen: u64, remote: Option<String>) {
        self.of(key).send_if_modified(|held| {
            held.touched = Some(Instant::now());
            let unseen = held
                .given
                .iter()
                .filter(|(version, _)| *version > seen)
                .map(|(_, given)| given.clone())
                .collect::<Vec<_>>();
            let told = if unseen.is_empty() {
                held.given.clear();
                held.text = text;
                // Swift's own change: it holds it already.
                false
            } else {
                // Each is put ahead in turn, so the latest stands first.
                held.text = unseen
                    .into_iter()
                    .fold(text, |written, given| ahead(&given, &written));
                true
            };
            if held.text.trim().is_empty() {
                held.remote = None;
                held.maybe_sent = false;
            } else if remote.is_some() {
                held.remote = remote;
            }
            told
        });
        self.changed();
    }

    /// Gives `text`, which a send took from the composer `key` names, back
    /// to it, ahead of anything written there since, and tells Swift.
    pub fn give_back(&self, key: &str, text: &str) {
        self.of(key).send_modify(|held| give(held, text));
        self.changed();
    }

    /// Marks `text`, taken from the composer `key` names, as on its way to
    /// the Remote `remote` names by its key, where the composer's key names
    /// none: it is kept in that draft until [`Self::answered`] or
    /// [`Self::give_back_sent`] is told of it. What the composer holds is no
    /// longer said to have maybe been sent.
    pub fn sending(&self, key: &str, text: &str, remote: Option<String>) -> Sent {
        let sent = Sent(self.shared.sent.fetch_add(1, Ordering::Relaxed) + 1);
        self.of(key).send_if_modified(|held| {
            held.sending.push(OnItsWay {
                sent,
                text: text.to_owned(),
                remote,
            });
            held.maybe_sent = false;
            false
        });
        self.changed();
        sent
    }

    /// Has `text` take the place of the Prompt `sent` on its way from the
    /// composer `key` names, as a Prompt written instead does.
    pub fn resending(&self, key: &str, sent: Sent, text: &str) {
        self.of(key).send_if_modified(|held| {
            if let Some(on) = held.sending.iter_mut().find(|on| on.sent == sent) {
                text.clone_into(&mut on.text);
            }
            false
        });
        self.changed();
    }

    /// The Prompt `sent` from the composer `key` names was answered: the
    /// draft keeps it no more, which is written at once.
    pub fn answered(&self, key: &str, sent: Sent) {
        let mut was = false;
        self.of(key).send_if_modified(|held| {
            let before = held.sending.len();
            held.sending.retain(|on| on.sent != sent);
            was = held.sending.len() != before;
            false
        });
        if was {
            self.write_now();
        }
    }

    /// Gives `text` back to the composer `key` names, as
    /// [`Self::give_back`] does, as what the Prompt `sent` on its way from it
    /// became: kept in its draft throughout, and said to have maybe been sent
    /// where it `maybe_sent`. Written at once.
    pub fn give_back_sent(&self, key: &str, sent: Sent, text: &str, maybe_sent: bool) {
        self.of(key).send_modify(|held| {
            if let Some(at) = held.sending.iter().position(|on| on.sent == sent) {
                let on = held.sending.remove(at);
                held.remote = held.remote.take().or(on.remote);
            }
            give(held, text);
            held.maybe_sent |= maybe_sent;
        });
        self.write_now();
    }

    /// Marks the Session `key` names as being begun: no listing drops its
    /// draft until one has held it.
    pub fn beginning(&self, key: &str) {
        self.shared
            .beginning
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(key.to_owned());
    }

    /// The Session `key` names, being begun, was discarded unmade: its draft
    /// goes with it, as nothing opens it again.
    pub fn abandoned(&self, key: &str) {
        self.shared
            .beginning
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(key);
        self.drop_where(|dropped, _, _| dropped == key);
    }

    /// Drops every draft written for a Remote other than those whose keys
    /// `held` holds, as a Remote removed drops its drafts, the Landing's
    /// among them where it was written for it; and keeps nothing for another
    /// from now on.
    pub fn keep_remotes(&self, held: &[String]) {
        *self
            .shared
            .remotes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(held.iter().cloned().collect());
        self.drop_where(|_, remote, _| remote.is_some_and(|remote| !held.contains(&remote)));
    }

    /// Drops the draft of every Session of the Remote `fingerprint` that its
    /// listing, asked for at `asked`, does not hold, where it is not written
    /// since and holds no Prompt on its way; a Session being begun only once
    /// a listing has held it.
    pub fn listed(&self, fingerprint: &str, holds: impl Fn(SessionId) -> bool, asked: Instant) {
        let of_this = |key: &str| {
            crate::listing::referenced(key)
                .filter(|(remote, _)| *remote == fingerprint)
                .map(|(_, session)| session)
        };
        let beginning = {
            let mut beginning = self
                .shared
                .beginning
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            beginning.retain(|key| !of_this(key).is_some_and(&holds));
            beginning.clone()
        };
        self.drop_where(|key, _, held| {
            of_this(key).is_some_and(|session| !holds(session))
                && !beginning.contains(key)
                && held.sending.is_empty()
                && held.touched.is_none_or(|touched| touched < asked)
        });
    }

    /// Drops every draft with anything in it that `drop` says to, by its key
    /// and the Remote it was written for, without a word to Swift.
    fn drop_where(&self, drop: impl Fn(&str, Option<String>, &Held) -> bool) {
        let mut any = false;
        for (key, held) in self.lock().iter() {
            held.send_if_modified(|held| {
                let empty = held.text.is_empty() && held.sending.is_empty();
                if !empty && drop(key, held.remote_for(key), held) {
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

impl Held {
    /// The Remote the draft the composer `key` names is written for, by its
    /// key: the one its key names, or where it names none the one recorded.
    fn remote_for(&self, key: &str) -> Option<String> {
        crate::listing::referenced(key)
            .map(|(remote, _)| remote.to_owned())
            .or_else(|| self.kept().remote)
    }
}

impl Shared {
    /// What the file is to keep: every draft with anything in it, written
    /// for a Remote still held or for none.
    fn kept(&self) -> BTreeMap<String, KeptDraft> {
        let remotes = self
            .remotes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        self.held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .filter_map(|(key, held)| {
                let held = held.borrow();
                let held_for = held.remote_for(key).is_none_or(|remote| {
                    remotes
                        .as_ref()
                        .is_none_or(|remotes| remotes.contains(&remote))
                });
                let kept = held.kept();
                (held_for && !kept.text.trim().is_empty()).then(|| (key.clone(), kept))
            })
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
        kept_in(directory)
            .into_iter()
            .map(|(key, kept)| (key, kept.text))
            .collect()
    }

    /// Every draft the file in `directory` keeps, whole.
    fn kept_in(directory: &Path) -> BTreeMap<String, KeptDraft> {
        shaman_pairing::read_records(&directory.join(DRAFTS_FILE)).unwrap()
    }

    /// The text the file in `directory` keeps under `key`.
    fn text(directory: &Path, key: &str) -> Option<String> {
        on_disk(directory).remove(key)
    }

    /// What the file in `directory` keeps once it keeps what `kept` looks
    /// for, as it is written a moment after the last change.
    async fn on_disk_until(
        directory: &Path,
        kept: impl Fn(&BTreeMap<String, String>) -> bool,
    ) -> BTreeMap<String, String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(drafts) = shaman_pairing::read_records::<BTreeMap<String, KeptDraft>>(
                    &directory.join(DRAFTS_FILE),
                ) {
                    let drafts = drafts
                        .into_iter()
                        .map(|(key, kept)| (key, kept.text))
                        .collect();
                    if kept(&drafts) {
                        return drafts;
                    }
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

    /// Lets the write a moment after the last change run, on a runtime whose
    /// clock stands still but as a test moves it, and waits for its file.
    async fn written_by_now(directory: &Path) -> BTreeMap<String, KeptDraft> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(kept) = shaman_pairing::read_records::<BTreeMap<String, KeptDraft>>(
                &directory.join(DRAFTS_FILE),
            ) && !kept.is_empty()
            {
                return kept;
            }
            assert!(std::time::Instant::now() < deadline, "never written");
            // The write runs on the blocking pool; the task that waits on it,
            // here.
            std::thread::sleep(Duration::from_millis(1));
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn typing_on_puts_the_write_off_and_writing_now_writes_at_once() {
        let directory = tempfile::tempdir().unwrap();
        let after = Duration::from_millis(500);
        let drafts = Drafts::open(directory.path(), after, &tokio::runtime::Handle::current());
        let session = key("desk", SessionId::new());
        for typed in ["T", "Ti", "Tid", "Tidy", "Tidy "] {
            drafts.set(&session, typed.to_owned(), 0);
            tokio::time::sleep(after - Duration::from_millis(1)).await;
        }
        assert!(
            !directory.path().join(DRAFTS_FILE).exists(),
            "nothing is written while typing goes on"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
        let kept = written_by_now(directory.path()).await;
        assert_eq!(
            kept.get(&session).map(|kept| kept.text.as_str()),
            Some("Tidy ")
        );

        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        drafts.set(&session, "Tidy the build".to_owned(), 0);
        drafts.write_now();
        assert_eq!(
            text(directory.path(), &session).as_deref(),
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
        let sent = drafts.sending(&session, "Tidy the build", None);
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
        let sent = drafts.sending(landing, "Tidy the build", None);
        drafts.resending(landing, sent, "Tidy the whole build");
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path()).get(landing).map(String::as_str),
            Some("Tidy the whole build"),
            "a Prompt written instead takes its place"
        );

        drafts.give_back_sent(landing, sent, "Tidy the whole build", false);
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
        drafts.sending(&lab, "on its way to the lab", None);
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
        drafts.sending(&key("desk", sending), "on its way", None);
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

    #[tokio::test(flavor = "multi_thread")]
    async fn an_answer_is_written_at_once() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        let session = key("desk", SessionId::new());
        let sent = drafts.sending(&session, "Tidy the build", None);
        drafts.write_now();
        assert_eq!(
            text(directory.path(), &session).as_deref(),
            Some("Tidy the build")
        );

        drafts.answered(&session, sent);
        assert_eq!(
            text(directory.path(), &session),
            None,
            "with no moment waited and nothing else asked"
        );

        let sent = drafts.sending(&session, "Steer it", None);
        drafts.write_now();
        drafts.give_back_sent(&session, sent, "Steer it", false);
        drafts.set(&session, "Steer it now".to_owned(), 1);
        assert_eq!(
            kept_in(directory.path()).get(&session),
            Some(&KeptDraft {
                text: "Steer it".to_owned(),
                ..KeptDraft::default()
            }),
            "given back, at once, and no longer on its way"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn what_may_have_been_sent_says_so_the_next_run_until_it_is_sent_or_emptied() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        let (lost, ended) = (key("desk", SessionId::new()), key("desk", SessionId::new()));
        let sent = drafts.sending(&lost, "Tidy the build", None);
        drafts.give_back_sent(&lost, sent, "Tidy the build", true);
        assert!(drafts.maybe_sent(&lost));
        // The app is ended with this one on its way.
        drafts.sending(&ended, "Steer it", None);
        drafts.write_now();

        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        for (key, text) in [(&lost, "Tidy the build"), (&ended, "Steer it")] {
            assert_eq!(drafts.get(key).text, text);
            assert!(drafts.maybe_sent(key), "{text} may have been sent");
        }

        drafts.set(&lost, "Tidy the build again".to_owned(), 0);
        assert!(drafts.maybe_sent(&lost), "still the text it belongs to");
        drafts.sending(&lost, "Tidy the build again", None);
        assert!(!drafts.maybe_sent(&lost), "sent again");
        drafts.set(&ended, String::new(), 0);
        assert!(!drafts.maybe_sent(&ended), "emptied");
        drafts.write_now();
        assert!(
            kept_in(directory.path())
                .values()
                .all(|kept| !kept.maybe_sent || !kept.text.is_empty()),
        );
        assert_eq!(text(directory.path(), &ended), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_session_being_begun_keeps_its_draft_until_a_listing_has_held_it() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        let (begun, discarded) = (SessionId::new(), SessionId::new());
        for session in [begun, discarded] {
            drafts.beginning(&key("desk", session));
            drafts.set(&key("desk", session), "written as it begins".to_owned(), 0);
        }

        drafts.listed("desk", |_| false, Instant::now());
        assert_eq!(
            drafts.get(&key("desk", begun)).text,
            "written as it begins",
            "no listing held it yet"
        );
        drafts.listed("desk", |session| session == begun, Instant::now());
        drafts.listed("desk", |_| false, Instant::now());
        assert_eq!(
            drafts.get(&key("desk", begun)).text,
            "",
            "once a listing has held it, one that does not drops it"
        );

        assert_eq!(
            drafts.get(&key("desk", discarded)).text,
            "written as it begins"
        );
        drafts.abandoned(&key("desk", discarded));
        assert_eq!(drafts.get(&key("desk", discarded)).text, "");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_landings_draft_goes_with_the_remote_it_was_written_for_and_not_as_the_outlook_turns()
     {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        let landing = beginning_key();
        drafts.keep_remotes(&["desk".to_owned(), "lab".to_owned()]);
        drafts.set_for(landing, "for the lab".to_owned(), 0, Some("lab".to_owned()));
        // The Outlook turns toward the desk, and nothing more is written.
        drafts.keep_remotes(&["desk".to_owned(), "lab".to_owned()]);
        drafts.write_now();
        assert_eq!(
            kept_in(directory.path()).get(landing),
            Some(&KeptDraft {
                text: "for the lab".to_owned(),
                remote: Some("lab".to_owned()),
                maybe_sent: false
            })
        );

        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        drafts.keep_remotes(&["desk".to_owned()]);
        assert_eq!(drafts.get(landing).text, "", "the lab is no longer held");

        drafts.set_for(
            landing,
            "for the desk".to_owned(),
            0,
            Some("desk".to_owned()),
        );
        drafts.keep_remotes(&["desk".to_owned()]);
        assert_eq!(drafts.get(landing).text, "for the desk");
        drafts.set(landing, String::new(), 0);
        drafts.set(landing, "for no remote yet".to_owned(), 0);
        drafts.keep_remotes(&[]);
        assert_eq!(
            drafts.get(landing).text,
            "for no remote yet",
            "emptied, it was written for none"
        );

        // A Prompt sent from it, given back, is written for where it went.
        let sent = drafts.sending(landing, "Begin it", Some("desk".to_owned()));
        drafts.set(landing, String::new(), 0);
        drafts.give_back_sent(landing, sent, "Begin it", false);
        drafts.keep_remotes(&["lab".to_owned()]);
        assert_eq!(drafts.get(landing).text, "");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nothing_is_kept_for_a_remote_no_longer_held_however_late_it_comes() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = Drafts::open(directory.path(), NEVER, &tokio::runtime::Handle::current());
        let (desk, lab) = (key("desk", SessionId::new()), key("lab", SessionId::new()));
        let sent = drafts.sending(&lab, "Tidy the lab", None);
        drafts.keep_remotes(&["desk".to_owned()]);
        drafts.set(&desk, "on the desk".to_owned(), 0);

        // The send to the lab ends after the lab went.
        drafts.give_back_sent(&lab, sent, "Tidy the lab", true);
        drafts.set(&lab, "and more".to_owned(), 1);
        drafts.write_now();
        assert_eq!(
            on_disk(directory.path()),
            BTreeMap::from([(desk, "on the desk".to_owned())])
        );
    }

    fn beginning_key() -> &'static str {
        crate::beginning::LANDING_DRAFT
    }
}
