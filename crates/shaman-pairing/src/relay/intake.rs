//! What a Relay says, as the Server takes it in.
//!
//! This is the intake rule. A Relay holds no trust (ADR-0045), and what it says
//! is drawn on the terminal of whoever reads it, which acts on whatever it is
//! sent. So every string a Relay fills, in every message, is held here to what
//! can safely be shown ([`shown`]) before anything else on this Server sees it,
//! each within what that string could need; a message gaining a field has to
//! say here what becomes of it. A reader's being sent somewhere is the one
//! thing not cleaned: an address that needed cleaning is not where they should
//! go, so the login offering it is refused.

use shaman_relay_protocol::{
    Account, ListedLogin, MAX_LISTED_HOSTNAME_CHARS, Refusal, RelayMessage,
};

use crate::shown_text::{shown, unseen};

/// The most characters of a Relay's own account of itself that are kept, past
/// anything a Relay says for a reader to read.
pub(super) const MAX_REASON_CHARS: usize = 512;
/// The most characters of an identity's name at its provider: an address is
/// the longest name one goes by.
const MAX_USERNAME_CHARS: usize = 254;
const MAX_PROVIDER_CHARS: usize = 64;
/// The most characters of a code a reader is to type on another device.
const MAX_USER_CODE_CHARS: usize = 64;
/// The most bytes of an address a reader is to visit.
const MAX_LOGIN_ADDRESS_LEN: usize = 2048;

/// What a frame of `text` from a Relay says, held to the intake rule, where it
/// is a message this Server recognizes.
pub(super) fn told(text: &str) -> Option<RelayMessage> {
    match serde_json::from_str(text) {
        Ok(RelayMessage::Unrecognized) | Err(_) => None,
        Ok(message) => Some(taken_in(message)),
    }
}

fn taken_in(message: RelayMessage) -> RelayMessage {
    match message {
        RelayMessage::Proven { login } => RelayMessage::Proven {
            login: login.map(account),
        },
        RelayMessage::LoginDone { account: named } => RelayMessage::LoginDone {
            account: account(named),
        },
        RelayMessage::LoginStarted {
            verification_uri,
            user_code,
            expires_in_seconds,
        } => {
            let user_code = shown(&user_code, MAX_USER_CODE_CHARS);
            match login_address(&verification_uri) {
                Some(verification_uri) if !user_code.is_empty() => RelayMessage::LoginStarted {
                    verification_uri,
                    user_code,
                    expires_in_seconds,
                },
                _ => RelayMessage::Refused {
                    refusal: Refusal::LoginUnavailable,
                    message: "the Relay offered a login this Server will not show: it must name \
                              an https address to visit and a code to enter there"
                        .to_owned(),
                },
            }
        }
        RelayMessage::Logins { logins, cut_short } => RelayMessage::Logins {
            logins: logins
                .into_iter()
                .map(|login| ListedLogin {
                    hostname: shown(&login.hostname, MAX_LISTED_HOSTNAME_CHARS),
                    ..login
                })
                .collect(),
            cut_short,
        },
        RelayMessage::Refused { refusal, message } => RelayMessage::Refused {
            refusal,
            message: named_or(
                shown(&message, MAX_REASON_CHARS),
                "the Relay did not say why",
            ),
        },
        // The address a Relay names itself by is compared as it was said, since
        // a proof is made only for one naming exactly the address this Server
        // knows it at, and is held to the rule where it is quoted.
        message @ RelayMessage::Challenge { .. } => message,
        // Nothing here fills a string.
        message @ (RelayMessage::Forgotten
        | RelayMessage::Waiting
        | RelayMessage::Reach { .. }
        | RelayMessage::Joined
        | RelayMessage::Unrecognized) => message,
    }
}

fn account(Account { provider, username }: Account) -> Account {
    Account {
        provider: named_or(shown(&provider, MAX_PROVIDER_CHARS), "unknown"),
        username: named_or(shown(&username, MAX_USERNAME_CHARS), "unknown"),
    }
}

fn named_or(text: String, otherwise: &str) -> String {
    if text.is_empty() {
        otherwise.to_owned()
    } else {
        text
    }
}

/// `address` as a reader is told to visit it, where it is an `https` address
/// of a host naming no credentials and holding nothing a reader could not see.
/// A host in another script is shown in the form that cannot pass for another
/// host; any other is left exactly as the Relay gave it.
fn login_address(address: &str) -> Option<String> {
    if address.len() > MAX_LOGIN_ADDRESS_LEN
        || address
            .chars()
            .any(|character| unseen(character) || character.is_whitespace())
    {
        return None;
    }
    let url = reqwest::Url::parse(address).ok()?;
    let usable = url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none();
    usable.then(|| {
        if address.is_ascii() {
            address.to_owned()
        } else {
            url.into()
        }
    })
}

#[cfg(test)]
mod tests {
    use shaman_relay_protocol::Bytes;

    use super::*;

    const HOSTILE: &str = "\u{1b}]52;c;cHduZWQ=\u{7}\u{1b}[2J";

    /// A join refused on the way to carrying a Pairing is heard here like any
    /// other refusal, whichever way the Server came to ask.
    #[test]
    fn every_refusal_is_held_to_the_rule_keeping_why_it_was_refused() {
        let wire = serde_json::to_string(&RelayMessage::Refused {
            refusal: Refusal::CapReached {
                cap: shaman_relay_protocol::Cap::JoinedConnections,
                limit: 3,
            },
            message: format!("{HOSTILE}at its cap\n\tof three{HOSTILE}"),
        })
        .unwrap();
        assert_eq!(
            told(&wire),
            Some(RelayMessage::Refused {
                refusal: Refusal::CapReached {
                    cap: shaman_relay_protocol::Cap::JoinedConnections,
                    limit: 3,
                },
                message: "at its cap of three".to_owned(),
            })
        );
    }

    #[test]
    fn a_refusal_with_nothing_left_of_why_says_it_was_not_told() {
        let wire = serde_json::to_string(&RelayMessage::Refused {
            refusal: Refusal::Unavailable,
            message: HOSTILE.to_owned(),
        })
        .unwrap();
        assert!(matches!(
            told(&wire),
            Some(RelayMessage::Refused { message, .. }) if message == "the Relay did not say why"
        ));
    }

    #[test]
    fn a_list_is_held_to_the_rule_and_its_keys_are_left_to_be_judged() {
        let wire = serde_json::to_string(&RelayMessage::Logins {
            logins: vec![ListedLogin {
                key: Bytes(vec![1, 2, 3]),
                hostname: format!("{HOSTILE}laptop  Remote already{HOSTILE}"),
                waiting: true,
            }],
            cut_short: true,
        })
        .unwrap();
        assert_eq!(
            told(&wire),
            Some(RelayMessage::Logins {
                logins: vec![ListedLogin {
                    key: Bytes(vec![1, 2, 3]),
                    hostname: "laptop Remote already".to_owned(),
                    waiting: true,
                }],
                cut_short: true,
            })
        );
    }

    #[test]
    fn what_is_not_a_message_recognized_is_not_told() {
        assert_eq!(told("not json"), None);
        assert_eq!(told(r#"{"type":"said_in_a_later_release"}"#), None);
    }

    #[test]
    fn each_string_keeps_what_it_could_need_past_a_hostnames_bound() {
        let long = "x".repeat(200);
        let wire = serde_json::to_string(&RelayMessage::Proven {
            login: Some(Account {
                provider: "github".to_owned(),
                username: long.clone(),
            }),
        })
        .unwrap();
        assert!(matches!(
            told(&wire),
            Some(RelayMessage::Proven { login: Some(account) }) if account.username == long
        ));
    }
}
