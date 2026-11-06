//! What a Relay answers a Server asking which other Logins stand under its
//! Account (ADR-0064): every one of them, whether or not its Server Serves,
//! within what one message of the protocol holds.
//!
//! It holds at most [`MAX_MESSAGE_LEN`] bytes however high an operator raises
//! the cap on Logins, and a hostname is at most
//! [`MAX_LISTED_HOSTNAME_CHARS`] characters, whatever was kept of it as its
//! Server logged in. Those are measured as they are encoded rather than
//! counted, since what a character comes to on the wire depends on the
//! character. Where not all of them fit, those that wait to be reached are
//! kept first, being the ones that can be reached now, each kind the latest
//! formed first, so a Relay asked twice of unchanged Logins cuts one answer
//! short in one place.

use suru_relay_protocol::{
    Bytes, ListedLogin, MAX_LISTED_HOSTNAME_CHARS, MAX_MESSAGE_LEN, RelayMessage,
};

/// How many Logins one asking reads from the Relay's records, well past what
/// one message holds so those that wait are not lost among those that do not,
/// and bound so a cap raised without end costs each asking no more than this.
pub(crate) const LOGINS_READ_AT_ONCE: usize = 4096;

/// What `hostname` shows a reader within `at_most` characters: what is left
/// of it beside control characters and outer whitespace.
pub(crate) fn shown(hostname: &str, at_most: usize) -> String {
    hostname
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(at_most)
        .collect()
}

/// The answer listing `logins`, by the identity key each is tied to and the
/// hostname it reported, each said to wait as `waits` has it: cut short where
/// they do not all fit, or where `more` says they were cut short already.
pub(crate) fn answer(
    logins: Vec<(Vec<u8>, String)>,
    more: bool,
    waits: impl Fn(&[u8]) -> bool,
) -> RelayMessage {
    let mut logins: Vec<ListedLogin> = logins
        .into_iter()
        .map(|(key, hostname)| ListedLogin {
            waiting: waits(&key),
            key: Bytes(key),
            hostname: shown(&hostname, MAX_LISTED_HOSTNAME_CHARS),
        })
        .collect();
    logins.sort_by_key(|login| !login.waiting);
    let total = logins.len();
    let kept = within_one_message(&logins);
    logins.truncate(kept);
    RelayMessage::Logins {
        logins,
        cut_short: more || kept < total,
    }
}

/// How many of `logins`, from the first, fit one message saying it was cut
/// short.
fn within_one_message(logins: &[ListedLogin]) -> usize {
    let mut length = encoded_length(&RelayMessage::Logins {
        logins: Vec::new(),
        cut_short: true,
    });
    logins
        .iter()
        .take_while(|login| {
            // A comma between each and the one before; one too many is
            // counted, which errs on the side of the bound.
            length += encoded_length(login) + 1;
            length <= MAX_MESSAGE_LEN
        })
        .count()
}

fn encoded_length(value: &impl serde::Serialize) -> usize {
    serde_json::to_vec(value)
        .expect("a Relay message always encodes")
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key of an ECDSA P-384 identity, the longest kind a Relay checks.
    const LONGEST_KEY: usize = 120;

    fn listed(answer: &RelayMessage) -> (&[ListedLogin], bool) {
        match answer {
            RelayMessage::Logins { logins, cut_short } => (logins, *cut_short),
            other => panic!("a list, not {other:?}"),
        }
    }

    /// What the bound is measured against: what the Relay would put on the
    /// wire, for the characters that come to the most there.
    #[test]
    fn an_answer_stays_within_one_message_whatever_its_hostnames_are_made_of() {
        for character in [
            'a', '"', '\\', 'é', '語', '🦀', '\u{202e}', '\u{1}', '\u{7f}',
        ] {
            let logins = (0..1000u32)
                .map(|number| {
                    let mut key = number.to_be_bytes().to_vec();
                    key.resize(LONGEST_KEY, 7);
                    (key, character.to_string().repeat(255))
                })
                .collect();
            let answer = answer(logins, false, |_| true);
            let wire = serde_json::to_vec(&answer).unwrap();
            assert!(
                wire.len() <= MAX_MESSAGE_LEN,
                "{character:?}: {} bytes",
                wire.len()
            );
            let (kept, cut_short) = listed(&answer);
            assert!(cut_short, "{character:?}");
            assert!(!kept.is_empty(), "{character:?}");
        }
    }

    /// A Relay at its default cap lists every Login, each with the longest
    /// hostname it lists, however that hostname is made up.
    #[test]
    fn the_default_cap_on_logins_is_listed_whole() {
        let logins = (0..crate::LOGINS_PER_ACCOUNT.get())
            .map(|number| {
                let mut key = number.to_be_bytes().to_vec();
                key.resize(LONGEST_KEY, 7);
                (key, "🦀".repeat(255))
            })
            .collect();
        let answer = answer(logins, false, |_| false);
        let (kept, cut_short) = listed(&answer);
        assert!(!cut_short);
        assert_eq!(kept.len(), crate::LOGINS_PER_ACCOUNT.get() as usize);
    }

    #[test]
    fn a_hostname_is_listed_without_controls_within_its_bound() {
        let answer = answer(
            vec![(
                b"key".to_vec(),
                format!("  \u{1b}[31m{}\u{7}  ", "x".repeat(300)),
            )],
            false,
            |_| false,
        );
        let (kept, _) = listed(&answer);
        assert_eq!(
            kept[0].hostname.chars().count(),
            MAX_LISTED_HOSTNAME_CHARS,
            "{:?}",
            kept[0].hostname
        );
        assert!(!kept[0].hostname.contains(char::is_control));
    }

    #[test]
    fn those_that_wait_are_kept_first_in_one_order_and_a_cut_is_said() {
        let logins: Vec<(Vec<u8>, String)> = (0..2000u32)
            .map(|number| {
                let mut key = number.to_be_bytes().to_vec();
                key.resize(LONGEST_KEY, 7);
                (key, format!("box-{number}"))
            })
            .collect();
        let waits = |key: &[u8]| key[3] % 2 == 1;
        let first = answer(logins.clone(), false, waits);
        assert_eq!(first, answer(logins.clone(), false, waits));
        let (kept, cut_short) = listed(&first);
        assert!(cut_short);
        let waiting = kept.iter().take_while(|login| login.waiting).count();
        assert!(
            kept[waiting..].iter().all(|login| !login.waiting),
            "those that wait come first"
        );
        assert_eq!(kept[0].hostname, "box-1", "in the order they were read");
        assert_eq!(kept[1].hostname, "box-3");

        let (_, cut_short) = listed(&answer(logins[..2].to_vec(), true, waits));
        assert!(cut_short, "what the records held back is cut short too");
        let whole = answer(logins[..2].to_vec(), false, waits);
        let (kept, cut_short) = listed(&whole);
        assert!(!cut_short);
        assert_eq!(kept.len(), 2);
    }
}
