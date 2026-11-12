//! What is made of a Relay's list of the machines logged in there before a
//! Client hears of it (ADR-0064).
//!
//! The Relay may lie, and a Login's hostname is whatever its Server reported,
//! so this is where what a Relay lists is held to what can be passed on, each
//! hostname having been held to what a reader can be shown as it was taken in
//! ([`super::intake`]): an entry whose key no Server could hold, this Server's
//! own, or a second of one key goes; a hostname with nothing left of it goes by
//! the key's fingerprint; and a key is matched against the Pairings this Server holds to name a Remote, which
//! opens nothing, so the most a lying Relay can do is mislabel a row.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use shaman_relay_protocol::{self as relay_protocol, ListedLogin};

use crate::{
    protocol::{RelayMachine, RelayMachinesAnswer, Remote, RemoteStatus},
    shown_text::short_fingerprint,
};

/// A Relay out of reach, for `reason`.
pub(super) fn unreachable(reason: &str) -> RelayMachinesAnswer {
    RelayMachinesAnswer::Unreachable {
        message: reason.to_owned(),
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
            RelayMachine {
                key: URL_SAFE_NO_PAD.encode(&login.key.0),
                hostname: if login.hostname.is_empty() {
                    format!("machine {}", short_fingerprint(&fingerprint))
                } else {
                    login.hostname
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

#[cfg(test)]
mod tests {
    use shaman_relay_protocol::Bytes;

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
            icon: None,
        }
    }

    #[test]
    fn a_hostname_with_nothing_left_of_it_is_named_by_its_fingerprint() {
        let key = key();
        let listed = listed(vec![login(&key, "")], b"own", &[]);
        let fingerprint = relay_protocol::fingerprint(&key);
        assert_eq!(listed[0].hostname, format!("machine {}", &fingerprint[..8]));
        assert_eq!(listed[0].fingerprint, fingerprint);
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
