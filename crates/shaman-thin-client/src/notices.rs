//! What the core says of its own accord, beside what an act's failure says:
//! that a Remote is newer than the app, once a change it sent was passed
//! over as one this build does not know (ADR-0078). It is said once for each
//! Remote for as long as the core runs, quietly, in the banner an act's
//! failure is shown in, however many more such changes that Remote sends.

use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
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

/// What the core is yet to say, and of which Remotes it has said it.
#[derive(Clone)]
pub(crate) struct Notices {
    inner: Arc<Inner>,
}

struct Inner {
    /// The Remotes this phone holds, which name the Remote a notice is of.
    pairings: watch::Receiver<Pairings>,
    held: Mutex<Held>,
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
                shown: watch::Sender::new(None),
            }),
        }
    }

    /// The notice to show now, and each one after it.
    pub fn shown(&self) -> watch::Receiver<Option<Notice>> {
        self.inner.shown.subscribe()
    }

    /// The Remote `fingerprint` names sent a change this build does not
    /// know, and so is newer than it: said, where it has not been already.
    pub fn newer(&self, fingerprint: &str) {
        let words = {
            let pairings = self.inner.pairings.borrow();
            let named = pairings.0.len() > 1;
            match pairings
                .0
                .iter()
                .find(|paired| paired.fingerprint == fingerprint)
            {
                Some(paired) if named => format!(
                    "{} runs a newer Shaman; some things may not show",
                    paired.name
                ),
                _ => "The server runs a newer Shaman; some things may not show".to_owned(),
            }
        };
        let mut held = self.lock();
        if !held.said.insert(fingerprint.to_owned()) {
            return;
        }
        held.next += 1;
        let notice = Notice {
            id: held.next,
            words,
        };
        held.waiting.push_back(notice);
        self.show(&held);
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
        let shown = notices.shown();
        assert_eq!(*shown.borrow(), None);

        notices.newer("desk-key");
        let said = shown.borrow().clone().expect("said");
        assert_eq!(
            said.words, "The server runs a newer Shaman; some things may not show",
            "naming no Origin beside no other"
        );
        notices.newer("desk-key");
        assert_eq!(shown.borrow().as_ref(), Some(&said), "said no more");

        notices.seen(said.id);
        assert_eq!(*shown.borrow(), None);
        notices.newer("desk-key");
        assert_eq!(*shown.borrow(), None, "and never again once seen");
    }

    #[test]
    fn each_remote_is_said_in_turn_by_its_name_beside_others() {
        let notices = Notices::new(pairings(&["desk", "lab"]));
        let shown = notices.shown();

        notices.newer("desk-key");
        notices.newer("lab-key");
        let first = shown.borrow().clone().expect("said");
        assert_eq!(
            first.words,
            "desk runs a newer Shaman; some things may not show"
        );
        notices.seen(first.id);
        let second = shown.borrow().clone().expect("the next is said");
        assert_eq!(
            second.words,
            "lab runs a newer Shaman; some things may not show"
        );
        let words = second.words.to_lowercase();
        assert!(!words.contains("version") && !words.contains("floor"));
    }
}
