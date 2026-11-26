//! What the core says of its own accord, beside what an act's failure says,
//! quietly, in the banner an act's failure is shown in: that a Remote is
//! newer than the app, once a change it sent was passed over as one this
//! build does not know (ADR-0078), which is said once for each Remote for as
//! long as the core runs, however many more such changes that Remote sends;
//! and that a Pairing has ended, and where the Outlook turned for it.
//!
//! Each window says them for itself: a Remote found newer is said once in
//! each window, a window made later included, and seen in one it stays to be
//! seen in the other. A Pairing ending is said in each window whose Outlook
//! stood there; where none stood there, why it ended is said once, by one
//! window.

use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex, Weak},
};

use tokio::sync::watch;

use crate::tree::Pairings;

/// Something the core says of its own accord, until Swift says it was seen.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Notice {
    /// What Swift hands back once the notice has been seen.
    pub id: u64,
    pub words: String,
}

/// What the device knows to say to every window: which Remotes are newer
/// than this build, each said in every window, a window made later included.
#[derive(Clone)]
pub(crate) struct Notices {
    inner: Arc<Inner>,
}

struct Inner {
    /// The Remotes this phone holds, which name the Remote a notice is of.
    pairings: watch::Receiver<Pairings>,
    held: Mutex<Device>,
}

#[derive(Default)]
struct Device {
    /// The Remotes found newer, by key, in the order they were found.
    newer: Vec<String>,
    /// Every window's notices, told each Remote found newer.
    windows: Vec<Weak<Window>>,
    /// What was said while no window stood, which the first window to stand
    /// says, and no other.
    unclaimed: Vec<String>,
}

/// What one window is yet to say, and of which Remotes it has said it.
#[derive(Clone)]
pub(crate) struct WindowNotices {
    inner: Arc<Window>,
}

struct Window {
    held: Mutex<Held>,
    /// Whether the window stands: its notices have been followed, as a
    /// window on screen follows them.
    stands: std::sync::atomic::AtomicBool,
    /// The notice to show now: the oldest not yet seen.
    shown: watch::Sender<Option<Notice>>,
}

#[derive(Default)]
struct Held {
    next: u64,
    /// The Remotes said to be newer, by key.
    said: HashSet<String>,
    waiting: VecDeque<Notice>,
}

impl Default for Notices {
    /// Notices of a phone holding no Remote, as a test's are.
    fn default() -> Self {
        Self::new(watch::channel(Pairings::default()).1)
    }
}

impl Notices {
    /// Notices naming each Remote as `pairings` names it.
    pub fn new(pairings: watch::Receiver<Pairings>) -> Self {
        Self {
            inner: Arc::new(Inner {
                pairings,
                held: Mutex::default(),
            }),
        }
    }

    /// The notices of a window made now, which says every Remote found newer
    /// so far, and each found after.
    pub fn window(&self) -> WindowNotices {
        let window = WindowNotices {
            inner: Arc::new(Window {
                held: Mutex::default(),
                stands: std::sync::atomic::AtomicBool::new(false),
                shown: watch::Sender::new(None),
            }),
        };
        let mut device = self.lock();
        // Of those still held alone, under the names they go by now.
        for fingerprint in &device.newer {
            if let Some(words) = self.newer_words(fingerprint) {
                window.newer(fingerprint, words);
            }
        }
        device.windows.retain(|window| window.strong_count() > 0);
        device.windows.push(Arc::downgrade(&window.inner));
        window
    }

    /// The Remote `fingerprint` names sent a change this build does not
    /// know, and so is newer than it: said in every window, where it has not
    /// been already.
    pub fn newer(&self, fingerprint: &str) {
        let words = self.newer_words(fingerprint).unwrap_or_else(|| {
            "The server runs a newer Shaman; some things may not show".to_owned()
        });
        let mut device = self.lock();
        if device.newer.iter().any(|found| found == fingerprint) {
            return;
        }
        device.newer.push(fingerprint.to_owned());
        device.windows.retain(|window| window.strong_count() > 0);
        for window in device.windows.iter().filter_map(Weak::upgrade) {
            WindowNotices { inner: window }.newer(fingerprint, words.clone());
        }
    }

    /// What says the Remote `fingerprint` names is newer, by the name it goes
    /// by now beside others; none where it is held no more.
    fn newer_words(&self, fingerprint: &str) -> Option<String> {
        let pairings = self.inner.pairings.borrow();
        let paired = pairings
            .0
            .iter()
            .find(|paired| paired.fingerprint == fingerprint)?;
        Some(if pairings.0.len() > 1 {
            format!(
                "{} runs a newer Shaman; some things may not show",
                paired.name
            )
        } else {
            "The server runs a newer Shaman; some things may not show".to_owned()
        })
    }

    /// Says `words` once, in one window: in the oldest window that stands,
    /// or, where none stands yet, in the first to stand.
    pub fn say_unclaimed(&self, words: String) {
        let mut device = self.lock();
        device.windows.retain(|window| window.strong_count() > 0);
        let standing = device
            .windows
            .iter()
            .filter_map(Weak::upgrade)
            .find(|window| window.stands.load(std::sync::atomic::Ordering::Acquire));
        match standing {
            Some(window) => {
                WindowNotices { inner: window }.say(words);
            }
            None => device.unclaimed.push(words),
        }
    }

    /// Hears that `window` stands, as it first follows its notices: the
    /// first to stand says what was said as none stood.
    pub fn stands(&self, window: &WindowNotices) {
        let mut device = self.lock();
        window
            .inner
            .stands
            .store(true, std::sync::atomic::Ordering::Release);
        for words in std::mem::take(&mut device.unclaimed) {
            window.say(words);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Device> {
        self.inner
            .held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl WindowNotices {
    /// The notice to show now, and each one after it.
    pub fn shown(&self) -> watch::Receiver<Option<Notice>> {
        self.inner.shown.subscribe()
    }

    /// Says `words` of the Remote `fingerprint` being newer, where this
    /// window has not said it already.
    fn newer(&self, fingerprint: &str, words: String) {
        let mut held = self.lock();
        if !held.said.insert(fingerprint.to_owned()) {
            return;
        }
        self.wait(&mut held, words);
    }

    /// Says `words`, after whatever is waiting to be said, answering the
    /// notice's id.
    pub fn say(&self, words: String) -> u64 {
        let mut held = self.lock();
        self.wait(&mut held, words)
    }

    fn wait(&self, held: &mut Held, words: String) -> u64 {
        held.next += 1;
        let id = held.next;
        held.waiting.push_back(Notice { id, words });
        self.show(held);
        id
    }

    /// Swift has seen the notice `id` names: the next waiting is shown.
    pub fn seen(&self, id: u64) {
        let mut held = self.lock();
        held.waiting.retain(|notice| notice.id != id);
        self.show(&held);
    }

    fn show(&self, held: &Held) {
        let next = held.waiting.front().cloned();
        self.inner.shown.send_if_modified(|shown| {
            let changed = *shown != next;
            *shown = next;
            changed
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Held> {
        self.inner
            .held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Paired;

    fn pairings(names: &[&str]) -> watch::Receiver<Pairings> {
        watch::channel(Pairings(
            names
                .iter()
                .map(|name| Paired {
                    fingerprint: format!("{name}-key"),
                    name: (*name).to_owned(),
                    paths: None,
                })
                .collect(),
        ))
        .1
    }

    #[test]
    fn a_newer_remote_is_said_once_and_never_again_however_often_it_sends_what_is_passed_over() {
        let notices = Notices::new(pairings(&["desk"]));
        let window = notices.window();
        let shown = window.shown();
        assert_eq!(*shown.borrow(), None);

        notices.newer("desk-key");
        let said = shown.borrow().clone().expect("said");
        assert_eq!(
            said.words, "The server runs a newer Shaman; some things may not show",
            "naming no Origin beside no other"
        );
        notices.newer("desk-key");
        assert_eq!(shown.borrow().as_ref(), Some(&said), "said no more");

        window.seen(said.id);
        assert_eq!(*shown.borrow(), None);
        notices.newer("desk-key");
        assert_eq!(*shown.borrow(), None, "and never again once seen");
    }

    #[test]
    fn each_remote_is_said_in_turn_by_its_name_beside_others() {
        let notices = Notices::new(pairings(&["desk", "lab"]));
        let window = notices.window();
        let shown = window.shown();

        notices.newer("desk-key");
        notices.newer("lab-key");
        let first = shown.borrow().clone().expect("said");
        assert_eq!(
            first.words,
            "desk runs a newer Shaman; some things may not show"
        );
        window.seen(first.id);
        let second = shown.borrow().clone().expect("the next is said");
        assert_eq!(
            second.words,
            "lab runs a newer Shaman; some things may not show"
        );
        let words = second.words.to_lowercase();
        assert!(!words.contains("version") && !words.contains("floor"));
    }

    #[test]
    fn each_window_says_a_newer_remote_for_itself_and_a_window_made_later_says_it_again() {
        let notices = Notices::new(pairings(&["desk"]));
        let first = notices.window();
        let second = notices.window();
        notices.newer("desk-key");

        let said = first.shown().borrow().clone().expect("said in the first");
        assert_eq!(
            second.shown().borrow().as_ref().map(|notice| &notice.words),
            Some(&said.words),
            "and in the second"
        );
        first.seen(said.id);
        assert_eq!(*first.shown().borrow(), None);
        assert!(
            second.shown().borrow().is_some(),
            "seen in one window, it stands in the other"
        );

        let later = notices.window();
        assert_eq!(
            later.shown().borrow().as_ref().map(|notice| &notice.words),
            Some(&said.words),
            "a window made later says it again"
        );
    }

    #[test]
    fn what_is_said_while_no_window_stands_is_said_by_the_first_to_stand_alone() {
        let notices = Notices::default();
        notices.say_unclaimed("desk removed this phone".to_owned());
        let discarded = notices.window();
        let first = notices.window();
        notices.stands(&first);
        drop(discarded);
        assert_eq!(
            first
                .shown()
                .borrow()
                .as_ref()
                .map(|notice| notice.words.as_str()),
            Some("desk removed this phone")
        );
        let later = notices.window();
        notices.stands(&later);
        assert_eq!(*later.shown().borrow(), None);

        // One standing by now says it, and no other: not one made and never
        // stood.
        let notices = Notices::default();
        let (never, first, second) = (notices.window(), notices.window(), notices.window());
        notices.stands(&first);
        notices.stands(&second);
        notices.say_unclaimed("lab removed this phone".to_owned());
        assert_eq!(*never.shown().borrow(), None);
        assert!(first.shown().borrow().is_some());
        assert_eq!(*second.shown().borrow(), None);
    }

    #[test]
    fn a_window_made_later_says_only_remotes_still_held_under_the_names_they_go_by_now() {
        let named = |names: &[&str]| {
            Pairings(
                names
                    .iter()
                    .map(|name| Paired {
                        fingerprint: format!("{}-key", name.split('=').next().unwrap()),
                        name: name.split('=').next_back().unwrap().to_owned(),
                        paths: None,
                    })
                    .collect(),
            )
        };
        let (pairings, held) = watch::channel(named(&["desk", "lab", "attic"]));
        let notices = Notices::new(held);
        notices.newer("desk-key");
        notices.newer("lab-key");

        // The desk is renamed the study, and the lab removed.
        pairings.send_replace(named(&["desk=study", "attic"]));
        let later = notices.window();
        let said = later.shown().borrow().clone().expect("said");
        assert_eq!(
            said.words,
            "study runs a newer Shaman; some things may not show"
        );
        later.seen(said.id);
        assert_eq!(*later.shown().borrow(), None, "and nothing of the lab");
    }

    #[test]
    fn what_one_window_says_the_other_does_not() {
        let notices = Notices::default();
        let (first, second) = (notices.window(), notices.window());
        first.say("desk was removed".to_owned());
        assert!(first.shown().borrow().is_some());
        assert_eq!(*second.shown().borrow(), None);
    }
}
