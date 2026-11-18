//! Each composer's unsent text, by the Session it is written in.
//!
//! Drafts are asked for, not told: Swift reads one when its screen appears
//! and after a failed send, and hands the core each change, so the core never
//! pushes text into a field being edited. What the core puts back is text it
//! took away: a Prompt whose send failed, or one an interrupt withdrew, which
//! stands ahead of anything written meanwhile.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Every composer's draft, held for the core's life.
#[derive(Clone, Default)]
pub(crate) struct Drafts {
    held: Arc<Mutex<HashMap<String, String>>>,
}

impl Drafts {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The draft of the composer `key` names, empty where it has none.
    pub fn get(&self, key: &str) -> String {
        self.lock().get(key).cloned().unwrap_or_default()
    }

    /// Holds `text` as the draft of the composer `key` names, holding none
    /// where it is empty.
    pub fn set(&self, key: &str, text: String) {
        let mut held = self.lock();
        if text.is_empty() {
            held.remove(key);
        } else {
            held.insert(key.to_owned(), text);
        }
    }

    /// Gives `text`, which a send took from the composer `key` names, back
    /// to it, ahead of anything written there since.
    pub fn give_back(&self, key: &str, text: &str) {
        let mut held = self.lock();
        let draft = held.entry(key.to_owned()).or_default();
        *draft = if draft.trim().is_empty() {
            text.to_owned()
        } else {
            format!("{text}\n{draft}")
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_is_what_was_last_handed_in_and_empty_where_none_was() {
        let drafts = Drafts::default();
        assert_eq!(drafts.get("desk/1"), "");
        drafts.set("desk/1", "half a thought".to_owned());
        drafts.set("desk/2", "another".to_owned());
        assert_eq!(drafts.get("desk/1"), "half a thought");
        drafts.set("desk/1", String::new());
        assert_eq!(drafts.get("desk/1"), "");
        assert_eq!(drafts.get("desk/2"), "another");
    }

    #[test]
    fn text_given_back_stands_ahead_of_anything_written_since() {
        let drafts = Drafts::default();
        drafts.give_back("desk/1", "Tidy the build");
        assert_eq!(drafts.get("desk/1"), "Tidy the build");

        drafts.set("desk/2", "and then".to_owned());
        drafts.give_back("desk/2", "Tidy the build");
        assert_eq!(drafts.get("desk/2"), "Tidy the build\nand then");

        drafts.set("desk/3", "  ".to_owned());
        drafts.give_back("desk/3", "Tidy the build");
        assert_eq!(
            drafts.get("desk/3"),
            "Tidy the build",
            "nothing but blanks is nothing written"
        );
    }
}
