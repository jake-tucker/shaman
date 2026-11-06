//! The short form of an Invite on a Server's own API: the code `/serve` shows
//! beside the long string, how it stands, and what typing it on another
//! machine asks and answers (ADR-0065). This is server administration, refused
//! to Peers and offered to no Sidekick.
//!
//! A code is secret for as long as it lives and is short enough to search once
//! anything derived from it is seen, so it does not offer itself up: it has no
//! `Display`, its `Debug` says nothing of it, and every copy is wiped as it is
//! dropped.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use zeroize::Zeroizing;

use super::{InvitePreview, InviteText};

/// The event the Server's own event stream pushes how its short code stands
/// under: as a client connects, and again whenever that changes.
pub const SHORT_CODE_EVENT: &str = "short_code";

/// What a code is written in: every digit and capital letter but `0`, `O`, `1`
/// and `I`, which leaves 32, so each character carries exactly five bits and a
/// code's eight carry exactly 40.
const ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// How many characters a code runs to, its separator aside.
const LENGTH: usize = 8;

/// A short Invite's code, written the one way: eight characters of its
/// alphabet, in capitals, with no separator. Two machines agree on it only in
/// this form, however it was typed.
#[derive(Clone, Eq, PartialEq)]
pub struct ShortCode(Zeroizing<String>);

impl ShortCode {
    /// A code from `random`, each byte giving one character: 256 is a multiple
    /// of the alphabet's 32, so taking the low five bits favours no character.
    pub(crate) fn from_random(random: &[u8; LENGTH]) -> Self {
        let mut code = Zeroizing::new(String::with_capacity(LENGTH));
        for byte in random {
            code.push(char::from(ALPHABET[usize::from(byte & 0x1f)]));
        }
        Self(code)
    }

    /// The code `typed` says, in any case, with or without its separator, and
    /// with whatever whitespace a paste brought with it. Anything else is
    /// refused here, before it could spend the one guess a code gets.
    pub fn parse(typed: &str) -> Result<Self, ShortCodeProblem> {
        let characters = || {
            typed
                .chars()
                .filter(|character| !character.is_whitespace() && *character != '-')
                .map(|character| character.to_ascii_uppercase())
        };
        // Its length first: what is far too long is no code with a character
        // wrong, and looking at that character again would not help.
        if characters().count() != LENGTH {
            return Err(ShortCodeProblem::WrongLength);
        }
        let mut code = Zeroizing::new(String::with_capacity(LENGTH));
        for character in characters() {
            if !u8::try_from(character).is_ok_and(|byte| ALPHABET.contains(&byte)) {
                return Err(if matches!(character, '0' | 'O' | '1' | 'I') {
                    ShortCodeProblem::LookAlike
                } else {
                    ShortCodeProblem::NotInAlphabet
                });
            }
            code.push(character);
        }
        Ok(Self(code))
    }

    /// The code as both machines agree on it, which nothing but the exchange
    /// reads.
    pub(crate) fn canonical(&self) -> &str {
        &self.0
    }

    /// The code as a reader is shown it, in two groups.
    pub fn grouped(&self) -> String {
        let (first, second) = self.0.split_at(LENGTH / 2);
        format!("{first}-{second}")
    }
}

impl std::fmt::Debug for ShortCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ShortCode(..)")
    }
}

impl Serialize for ShortCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ShortCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let written = Zeroizing::new(String::deserialize(deserializer)?);
        Self::parse(&written).map_err(D::Error::custom)
    }
}

/// Why what was typed is no code. Each says so without repeating any of it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortCodeProblem {
    WrongLength,
    /// It holds `0`, `O`, `1` or `I`, none of which a code is ever shown with.
    LookAlike,
    NotInAlphabet,
}

impl std::fmt::Display for ShortCodeProblem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::WrongLength => "A code is eight characters, such as K7QF-2MXP",
            Self::LookAlike => "A code holds no 0, O, 1 or I; look at that character again",
            Self::NotInAlphabet => "A code holds only letters and digits",
        })
    }
}

impl std::error::Error for ShortCodeProblem {}

/// An Invite's short code as it is issued, and which code of this Server it is.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IssuedShortCode {
    pub code: ShortCode,
    /// Told apart from every other code this Server has issued since it
    /// started, so a Client can tell whether what it is told is of this one.
    pub serial: u64,
}

/// How this Server's latest short code stands, which is never the code itself.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShortCodeStanding {
    /// The code this says of; `0` before any was issued.
    pub serial: u64,
    pub state: ShortCodeState,
}

/// Where a short code has got to. A code is tried once, so of these only
/// `Live` can be tried.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortCodeState {
    Live,
    /// An exchange took it and has yet to finish.
    UnderWay,
    /// Its Invite was delivered to whoever typed it.
    Delivered,
    /// An exchange proved a wrong code, or none, was typed. The Invite stands.
    WrongCodeTried,
    /// An exchange took it and never finished, which uses it up as a wrong
    /// guess does. The Invite stands.
    LeftUnfinished,
    /// Its Invite was redeemed, by this code's delivery or by the long string.
    Spent,
    /// It ended with its Invite, which expired or was superseded.
    #[default]
    Ended,
}

impl ShortCodeState {
    /// Whether it was used up by an attempt that delivered nothing, which is
    /// worth its reader's seeing.
    pub fn burned(self) -> bool {
        matches!(self, Self::WrongCodeTried | Self::LeftUnfinished)
    }

    /// Whether the Invite it came with can still be redeemed.
    pub fn invite_stands(self) -> bool {
        !matches!(self, Self::Spent | Self::Ended)
    }
}

/// A code typed for a machine chosen from those a Relay listed: ask for that
/// machine's Invite over one of the Relays said to carry it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FetchShortInviteRequest {
    /// The identity key listed for the machine, unpadded URL-safe base64. It
    /// is the Relay's word, which the exchange holds to account.
    pub server_key: String,
    /// The Relays it waits at, in the order to try them. Moving on from one to
    /// the next never makes a second guess.
    pub relays: Vec<String>,
    pub code: ShortCode,
}

/// The ordinary Invite a short code delivered, and its preview. Nothing is
/// trusted yet: it is redeemed as any other is.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveredInvite {
    pub invite: InviteText,
    pub preview: InvitePreview,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_typed_in_any_case_with_or_without_its_separator() {
        for typed in [
            "K7QF-2MXP",
            "K7QF2MXP",
            "k7qf-2mxp",
            "k7qf2mxp",
            "  K7QF-2MXP\n",
            "k7qf 2mxp",
            "K7\r\nQF-2M\tXP",
        ] {
            let code =
                ShortCode::parse(typed).unwrap_or_else(|problem| panic!("{typed:?}: {problem}"));
            assert_eq!(code.canonical(), "K7QF2MXP", "{typed:?}");
            assert_eq!(code.grouped(), "K7QF-2MXP", "{typed:?}");
        }
    }

    #[test]
    fn what_is_no_code_is_refused_saying_why_and_without_repeating_it() {
        for (typed, problem) in [
            ("", ShortCodeProblem::WrongLength),
            ("H4RW-9TN", ShortCodeProblem::WrongLength),
            ("H4RW-9TNPP", ShortCodeProblem::WrongLength),
            ("H4RW-9TN0", ShortCodeProblem::LookAlike),
            ("H4RW-9TNO", ShortCodeProblem::LookAlike),
            ("H4RW-9TN1", ShortCodeProblem::LookAlike),
            ("H4RW-9TNI", ShortCodeProblem::LookAlike),
            ("h4rw-9tni", ShortCodeProblem::LookAlike),
            ("H4RW_9TN", ShortCodeProblem::NotInAlphabet),
            // Far too long to be a code with a character wrong.
            ("k7suru-v1-eyJ3IjpbXX0", ShortCodeProblem::WrongLength),
            ("H4RW-9TNé", ShortCodeProblem::NotInAlphabet),
            ("H4RW-9TNß", ShortCodeProblem::NotInAlphabet),
        ] {
            assert_eq!(ShortCode::parse(typed), Err(problem), "{typed:?}");
            assert!(!problem.to_string().contains("H4RW"), "{problem}");
        }
    }

    /// Every byte's low five bits name each character eight times over, so no
    /// character comes up more often than another.
    #[test]
    fn every_character_is_as_likely_as_every_other() {
        let mut tally = std::collections::HashMap::new();
        for byte in 0..=u8::MAX {
            let code = ShortCode::from_random(&[byte; LENGTH]);
            *tally.entry(code.canonical().as_bytes()[0]).or_insert(0) += 1;
        }
        assert_eq!(tally.len(), 32);
        assert!(tally.values().all(|count| *count == 8), "{tally:?}");
    }

    #[test]
    fn the_alphabet_holds_no_look_alike_and_nothing_twice() {
        let mut seen = std::collections::HashSet::new();
        for character in ALPHABET {
            assert!(!b"01OI-".contains(character));
            assert!(character.is_ascii_uppercase() || character.is_ascii_digit());
            assert!(seen.insert(character));
        }
    }

    #[test]
    fn a_whole_invite_is_told_from_a_code_by_how_it_opens() {
        assert!(crate::protocol::opens_as_an_invite("suru-v1-eyJ3IjpbXX0"));
        assert!(crate::protocol::opens_as_an_invite("\n  suru-v2-anything"));
        assert!(!crate::protocol::opens_as_an_invite("K7QF-2MXP"));
        assert!(!crate::protocol::opens_as_an_invite(
            "k7suru-v1-eyJ3IjpbXX0"
        ));
    }

    /// An Invite crosses a Server's API as the bare string it always was, and
    /// nothing holding one prints its token.
    #[test]
    fn nothing_says_an_invites_token_by_accident() {
        const INVITE: &str = "suru-v1-the-token-in-its-string";
        let text = InviteText::from(INVITE);
        assert_eq!(
            serde_json::to_string(&text).unwrap(),
            format!("\"{INVITE}\"")
        );
        let preview = InvitePreview {
            hostname: "workstation".to_owned(),
            fingerprint: "aaaa".to_owned(),
            ways: Vec::new(),
        };
        for said in [
            format!("{text:?}"),
            format!(
                "{:?}",
                DeliveredInvite {
                    invite: text.clone(),
                    preview,
                }
            ),
            format!(
                "{:?}",
                crate::protocol::IssuedInvite {
                    invite: text.clone(),
                    ways: Vec::new(),
                    short_code: None,
                }
            ),
            format!(
                "{:?}",
                crate::protocol::PreviewInviteRequest {
                    invite: text.clone(),
                }
            ),
            format!(
                "{:?}",
                crate::protocol::RedeemInviteRequest {
                    invite: text.clone(),
                    name: None,
                    ways: Vec::new(),
                }
            ),
        ] {
            assert!(!said.contains("the-token"), "{said}");
        }
    }

    #[test]
    fn nothing_says_a_code_by_accident() {
        let code = ShortCode::parse("K7QF-2MXP").unwrap();
        for said in [
            format!("{code:?}"),
            format!(
                "{:?}",
                IssuedShortCode {
                    code: code.clone(),
                    serial: 1
                }
            ),
            format!(
                "{:?}",
                FetchShortInviteRequest {
                    server_key: "key".to_owned(),
                    relays: Vec::new(),
                    code,
                }
            ),
        ] {
            assert!(!said.contains("K7QF"), "{said}");
            assert!(!said.contains("2MXP"), "{said}");
        }
    }

    #[test]
    fn a_code_crosses_a_servers_api_in_its_one_form_and_is_refused_in_any_other() {
        let code = ShortCode::parse("k7qf-2mxp").unwrap();
        assert_eq!(serde_json::to_string(&code).unwrap(), "\"K7QF2MXP\"");
        let read: ShortCode = serde_json::from_str("\"k7qf-2mxp\"").unwrap();
        assert_eq!(read, code);
        let refused = serde_json::from_str::<ShortCode>("\"K7QF-2MX\"").unwrap_err();
        assert!(!refused.to_string().contains("2MX\""), "{refused}");
    }
}
