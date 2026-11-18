//! Each composer's unsent text, by the Session it is written in.
//!
//! Swift reads a draft when its screen appears and hands the core each change.
//! What the core puts back is text it took away: a Prompt whose send failed, or
//! one an interrupt withdrew, which stands ahead of anything written meanwhile.
//! That is the one change the core makes on its own, and it always wins: each
//! is a new version, which the core tells Swift of, and a change Swift hands in
//! written over an older version has what was given back since kept ahead of
//! it, however many keystrokes reach the core before Swift hears.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::watch;

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
}

impl Held {
    /// The draft as Swift holds it.
    pub fn draft(&self) -> Draft {
        Draft {
            text: self.text.clone(),
            version: self.version,
        }
    }
}

/// Every composer's draft, held for the core's life.
#[derive(Clone, Default)]
pub(crate) struct Drafts {
    held: Arc<Mutex<HashMap<String, watch::Sender<Held>>>>,
}

impl Drafts {
    /// The draft of the composer `key` names, made empty where it has none.
    fn of(&self, key: &str) -> watch::Sender<Held> {
        self.held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entry(key.to_owned())
            .or_insert_with(|| watch::Sender::new(Held::default()))
            .clone()
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
    }

    /// Gives `text`, which a send took from the composer `key` names, back
    /// to it, ahead of anything written there since, and tells Swift.
    pub fn give_back(&self, key: &str, text: &str) {
        self.of(key).send_modify(|held| {
            held.version += 1;
            held.given.push((held.version, text.to_owned()));
            held.text = ahead(text, &held.text);
        });
    }
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
}
