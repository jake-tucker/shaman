//! What a reader has half given each Questionnaire waiting: the device's, as a
//! composer's draft is, so a sheet of it opening later, in either window,
//! starts from whichever sheet last changed it (ADR-0084).
//!
//! Each sheet holds its own while it stands and hands the core each change; it
//! is never told of another's. What is kept lasts as long as the core does and
//! is never written to its directory, a secret's answer among it: it goes as
//! its Questionnaire is answered, declined or no longer waits, and with the
//! Remote its Session is on.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// What a reader has half given a Questionnaire, as a sheet of it last wrote
/// it: the page on show, and what was given each Question, in their order.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct HalfAnswered {
    pub page: u32,
    pub pages: Vec<HalfAnsweredPage>,
}

/// What was given one Question of a Questionnaire.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct HalfAnsweredPage {
    /// Whether it is a secret Question's.
    pub secret: bool,
    pub choices: Vec<String>,
    pub text: String,
    /// Whether a secret's answer was left behind, so it shows nothing of
    /// what it was.
    pub sealed: bool,
}

/// Every Questionnaire's half-given answers, by its Session and its own
/// reference, for the core's life.
#[derive(Clone, Default)]
pub(crate) struct HalfAnswers {
    held: Arc<Mutex<HashMap<(String, String), HalfAnswered>>>,
}

impl HalfAnswers {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), HalfAnswered>> {
        self.held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// What was last half given the Questionnaire `questionnaire` of the
    /// Session `session` names, where anything was.
    pub fn get(&self, session: &str, questionnaire: &str) -> Option<HalfAnswered> {
        self.lock()
            .get(&(session.to_owned(), questionnaire.to_owned()))
            .cloned()
    }

    /// Keeps `answers` as what was last half given that Questionnaire.
    pub fn keep(&self, session: &str, questionnaire: &str, answers: HalfAnswered) {
        self.lock()
            .insert((session.to_owned(), questionnaire.to_owned()), answers);
    }

    /// Forgets what was half given that Questionnaire.
    pub fn forget(&self, session: &str, questionnaire: &str) {
        self.lock()
            .remove(&(session.to_owned(), questionnaire.to_owned()));
    }

    /// Forgets what was half given any Questionnaire of the Session `session`
    /// names but those `waiting` names, as the others no longer wait.
    pub fn keep_waiting(&self, session: &str, waiting: &[String]) {
        self.lock().retain(|(of, questionnaire), _| {
            of != session || waiting.iter().any(|live| live == questionnaire)
        });
    }

    /// Forgets what was half given any Questionnaire of a Session on the
    /// Remote whose key's fingerprint is `fingerprint`, as it is held no more.
    pub fn forget_remote(&self, fingerprint: &str) {
        let prefix = format!("{fingerprint}/");
        self.lock()
            .retain(|(session, _), _| !session.starts_with(&prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half(text: &str) -> HalfAnswered {
        HalfAnswered {
            page: 1,
            pages: vec![
                HalfAnsweredPage::default(),
                HalfAnsweredPage {
                    text: text.to_owned(),
                    ..HalfAnsweredPage::default()
                },
            ],
        }
    }

    #[test]
    fn what_was_half_given_last_is_what_a_sheet_opening_later_starts_from() {
        let answers = HalfAnswers::default();
        assert_eq!(answers.get("desk-key/1", "q"), None);
        answers.keep("desk-key/1", "q", half("written in one window"));
        answers.keep("desk-key/1", "q", half("written in the other"));
        assert_eq!(
            answers.get("desk-key/1", "q"),
            Some(half("written in the other"))
        );
        assert_eq!(answers.get("desk-key/1", "another"), None);
        assert_eq!(answers.get("desk-key/2", "q"), None);

        answers.forget("desk-key/1", "q");
        assert_eq!(answers.get("desk-key/1", "q"), None);
    }

    #[test]
    fn what_was_half_given_a_questionnaire_that_waits_no_more_goes() {
        let answers = HalfAnswers::default();
        answers.keep("desk-key/1", "q1", half("still waiting"));
        answers.keep("desk-key/1", "q2", half("answered elsewhere"));
        answers.keep("desk-key/2", "q2", half("another session's"));
        answers.keep_waiting("desk-key/1", &["q1".to_owned()]);
        assert_eq!(answers.get("desk-key/1", "q1"), Some(half("still waiting")));
        assert_eq!(answers.get("desk-key/1", "q2"), None);
        assert_eq!(
            answers.get("desk-key/2", "q2"),
            Some(half("another session's"))
        );
    }

    #[test]
    fn a_remote_no_longer_held_takes_what_was_half_given_on_it() {
        let answers = HalfAnswers::default();
        answers.keep("desk-key/1", "q", half("on the desk"));
        answers.keep("lab-key/1", "q", half("on the lab"));
        answers.forget_remote("desk-key");
        assert_eq!(answers.get("desk-key/1", "q"), None);
        assert_eq!(answers.get("lab-key/1", "q"), Some(half("on the lab")));
    }
}
