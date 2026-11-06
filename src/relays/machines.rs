//! What is made of a Relay's list of the machines logged in there before a
//! Client hears of it (ADR-0064).
//!
//! The Relay may lie, and a Login's hostname is whatever its Server reported,
//! so this is where what a Relay says is held to what can safely be passed on:
//! an entry whose key no Server could hold, this Server's own, or a second of
//! one key goes; a hostname keeps only what a reader can be shown; and a key
//! is matched against the Pairings this Server holds to name a Remote, which
//! opens nothing, so the most a lying Relay can do is mislabel a row.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use suru_relay_protocol::{self as relay_protocol, ListedLogin, MAX_LISTED_HOSTNAME_CHARS};

use crate::{
    ansi::{AnsiScanner, FragmentRole},
    protocol::{RelayMachine, RelayMachinesAnswer, Remote, RemoteStatus},
    serving::{composed, short_fingerprint, unseen},
};

/// The most characters of why a Relay is out of reach that are passed on.
const MAX_REASON_CHARS: usize = 512;

/// A Relay out of reach, for `reason`, which may be the Relay's own words.
pub(super) fn unreachable(reason: &str) -> RelayMachinesAnswer {
    RelayMachinesAnswer::Unreachable {
        message: shown(reason, MAX_REASON_CHARS),
    }
}

/// What can be passed on of `logins`, as a Relay listed them to the Server
/// whose identity key is `own_key` and which is paired with `remotes`, in the
/// order listed.
pub(super) fn listed(
    logins: Vec<ListedLogin>,
    own_key: &[u8],
    remotes: &[Remote],
) -> Vec<RelayMachine> {
    let mut seen = std::collections::HashSet::new();
    logins
        .into_iter()
        .filter(|login| {
            let key = login.key.0.as_slice();
            key != own_key && relay_protocol::supports_key(key) && seen.insert(key.to_vec())
        })
        .map(|login| {
            let fingerprint = relay_protocol::fingerprint(&login.key.0);
            let hostname = shown(&login.hostname, MAX_LISTED_HOSTNAME_CHARS);
            RelayMachine {
                key: URL_SAFE_NO_PAD.encode(&login.key.0),
                hostname: if hostname.is_empty() {
                    format!("machine {}", short_fingerprint(&fingerprint))
                } else {
                    hostname
                },
                waiting: login.waiting,
                remote: remotes
                    .iter()
                    .find(|remote| {
                        remote.fingerprint == fingerprint && remote.status != RemoteStatus::Revoked
                    })
                    .map(|remote| remote.name.clone()),
                fingerprint,
            }
        })
        .collect()
}

/// What can safely reach a terminal of `text`, which came from another
/// machine, within `at_most` characters: escape sequences and control strings
/// go whole, as do control, formatting and otherwise default-ignorable
/// characters, as a Peer's name is held free of them — nothing a reader could
/// not see, and nothing that moves what they do see; any other break of a line
/// is a space; and it is composed canonically, outer whitespace aside.
pub(super) fn shown(text: &str, at_most: usize) -> String {
    let visible: String = AnsiScanner::default()
        .feed(text)
        .into_iter()
        .filter_map(|fragment| match fragment.into_role() {
            FragmentRole::Text(text) => Some(text),
            _ => None,
        })
        .collect::<String>()
        .chars()
        .filter(|character| !unseen(*character))
        .map(|character| {
            if character.is_whitespace() {
                ' '
            } else {
                character
            }
        })
        .collect();
    composed(visible.trim())
        .chars()
        .take(at_most)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use suru_relay_protocol::Bytes;

    use super::*;

    /// An ECDSA P-256 SubjectPublicKeyInfo, made afresh.
    fn key() -> Vec<u8> {
        rcgen::PublicKeyData::subject_public_key_info(
            &rcgen::KeyPair::generate().expect("make a key"),
        )
    }

    fn login(key: &[u8], hostname: &str) -> ListedLogin {
        ListedLogin {
            key: Bytes(key.to_vec()),
            hostname: hostname.to_owned(),
            waiting: true,
        }
    }

    fn remote(name: &str, fingerprint: &str, status: RemoteStatus) -> Remote {
        Remote {
            name: name.to_owned(),
            fingerprint: fingerprint.to_owned(),
            ways: Vec::new(),
            status,
        }
    }

    #[test]
    fn what_could_move_or_hide_text_on_a_terminal_goes_whole() {
        for (given, kept) in [
            ("workstation", "workstation"),
            ("  workstation \t", "workstation"),
            // C0, C1, DEL.
            ("work\u{0}\u{7}\u{8}\r\nstation", "workstation"),
            ("work\u{80}\u{85}\u{9b}\u{9f}station", "workstation"),
            ("work\u{7f}station", "workstation"),
            // Escape sequences, whole and not their leavings: CSI, SGR, cursor
            // moves, a title set by OSC either way it ends, a hyperlink, DCS,
            // APC, and an escape of two characters.
            ("\u{1b}[31mwork\u{1b}[0mstation", "workstation"),
            ("work\u{1b}[2J\u{1b}[1;1Hstation", "workstation"),
            ("\u{1b}]0;owned\u{7}workstation", "workstation"),
            ("\u{1b}]0;owned\u{1b}\\workstation", "workstation"),
            (
                "\u{1b}]8;;https://evil.example\u{1b}\\workstation\u{1b}]8;;\u{1b}\\",
                "workstation",
            ),
            ("\u{1b}Powned\u{1b}\\workstation", "workstation"),
            ("\u{1b}_owned\u{1b}\\workstation", "workstation"),
            ("\u{1b}cworkstation", "workstation"),
            // Bidi overrides and isolates, which reorder what is drawn.
            ("\u{202e}noitatskrow", "noitatskrow"),
            ("work\u{2066}station\u{2069}\u{200f}", "workstation"),
            // What takes no width at all.
            (
                "work\u{200b}\u{200c}\u{200d}\u{2060}\u{feff}station",
                "workstation",
            ),
            ("work\u{ad}\u{e0001}\u{fe0f}station", "workstation"),
            // Another way to break a line is a space, not a break.
            ("work\u{2028}station\u{2029}", "work station"),
            // Composed one way, however it was typed.
            ("Cafe\u{301}", "Café"),
            // Wide characters stay, and are bound like any other.
            ("東京のマシン", "東京のマシン"),
        ] {
            assert_eq!(shown(given, 64), kept, "{given:?}");
        }
        assert_eq!(shown(&"語".repeat(100), 64), "語".repeat(64));
        assert_eq!(shown("ab cd", 3), "ab", "no whitespace is left at its end");
    }

    #[test]
    fn a_hostname_with_nothing_left_of_it_is_named_by_its_fingerprint() {
        let key = key();
        for hostname in [
            "",
            "   ",
            "\u{1b}[31m",
            "\u{200b}\u{202e}",
            "\u{1b}]0;never ends",
        ] {
            let listed = listed(vec![login(&key, hostname)], b"own", &[]);
            let fingerprint = relay_protocol::fingerprint(&key);
            assert_eq!(
                listed[0].hostname,
                format!("machine {}", &fingerprint[..8]),
                "{hostname:?}"
            );
            assert_eq!(listed[0].fingerprint, fingerprint);
        }
    }

    #[test]
    fn an_entry_no_server_could_hold_this_servers_own_or_a_second_of_a_key_goes() {
        let (own, first, second) = (key(), key(), key());
        let listed = listed(
            vec![
                login(&own, "myself"),
                login(&first, "first"),
                login(b"not a key", "malformed"),
                login(&[], "empty"),
                login(&first, "first again"),
                login(&second, "second"),
            ],
            &own,
            &[],
        );
        assert_eq!(
            listed
                .iter()
                .map(|machine| machine.hostname.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(listed[0].key, URL_SAFE_NO_PAD.encode(&first));
    }

    #[test]
    fn a_key_matching_a_pairing_names_the_remote_and_a_pairing_ended_names_nothing() {
        let (paired, ended, stranger) = (key(), key(), key());
        let remotes = [
            remote(
                "build-box",
                &relay_protocol::fingerprint(&paired),
                RemoteStatus::Available,
            ),
            remote(
                "gone",
                &relay_protocol::fingerprint(&ended),
                RemoteStatus::Revoked,
            ),
        ];
        let listed = listed(
            vec![
                login(&paired, "says-it-is-something-else"),
                login(&ended, "ended"),
                login(&stranger, "stranger"),
            ],
            b"own",
            &remotes,
        );
        assert_eq!(listed[0].remote.as_deref(), Some("build-box"));
        assert_eq!(listed[1].remote, None);
        assert_eq!(listed[2].remote, None);
    }
}
