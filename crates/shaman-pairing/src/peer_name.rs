//! The rule a Peer's name is held to: what a Serving Server keeps of the name
//! a Peer gives itself redeeming its Invite, and what a thin Client refuses
//! before it gives one.

use std::fmt;

use shaman_text::shown_text::{composed, unseen};

use crate::protocol::{EVERYWHERE, names_everywhere};

/// The most characters a Peer's name runs to.
pub const MAX_PEER_NAME_CHARS: usize = 64;

/// `name` as a Peer goes by it, where a reader can be shown it as a name:
/// kept as given, outer whitespace aside and composed canonically (NFC),
/// something but whitespace, no longer than [`MAX_PEER_NAME_CHARS`], holding
/// no control, formatting or otherwise default-ignorable character — nothing
/// a reader could not see — and not [`EVERYWHERE`] in any case.
pub fn shown_peer_name(name: &str) -> Result<String, PeerNameRefusal> {
    let name = composed(name.trim());
    if name.is_empty() {
        return Err(PeerNameRefusal::Empty);
    }
    let characters = name.chars().count();
    if characters > MAX_PEER_NAME_CHARS {
        return Err(PeerNameRefusal::TooLong { characters });
    }
    if name.chars().any(unseen) {
        return Err(PeerNameRefusal::Unseen);
    }
    if names_everywhere(&name) {
        return Err(PeerNameRefusal::Everywhere);
    }
    Ok(name)
}

/// Why a name is not one a Peer may go by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerNameRefusal {
    /// It holds nothing but whitespace.
    Empty,
    /// It runs past [`MAX_PEER_NAME_CHARS`], to `characters`.
    TooLong { characters: usize },
    /// It holds a character a reader could not see.
    Unseen,
    /// It is [`EVERYWHERE`], which names every Server at once.
    Everywhere,
}

impl fmt::Display for PeerNameRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("A name needs something other than spaces"),
            Self::TooLong { characters } => write!(
                formatter,
                "A name runs to {MAX_PEER_NAME_CHARS} characters at most; this one has \
                 {characters}"
            ),
            Self::Unseen => formatter.write_str(
                "A name cannot hold characters that do not show, such as tabs or formatting marks",
            ),
            Self::Everywhere => write!(
                formatter,
                "\"{EVERYWHERE}\" names every server at once, so nothing may be named so"
            ),
        }
    }
}

impl std::error::Error for PeerNameRefusal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_kept_as_given_without_its_outer_whitespace_and_composed() {
        assert_eq!(shown_peer_name("  laptop ").as_deref(), Ok("laptop"));
        assert_eq!(shown_peer_name("cafe\u{301}").as_deref(), Ok("caf\u{e9}"));
        assert_eq!(
            shown_peer_name(&"n".repeat(MAX_PEER_NAME_CHARS)),
            Ok("n".repeat(MAX_PEER_NAME_CHARS))
        );
    }

    #[test]
    fn a_name_a_reader_could_not_be_shown_is_refused_with_why() {
        assert_eq!(shown_peer_name(""), Err(PeerNameRefusal::Empty));
        assert_eq!(shown_peer_name("   "), Err(PeerNameRefusal::Empty));
        assert_eq!(
            shown_peer_name(&"n".repeat(MAX_PEER_NAME_CHARS + 1)),
            Err(PeerNameRefusal::TooLong {
                characters: MAX_PEER_NAME_CHARS + 1
            })
        );
        for unseen in [
            "lap\ttop",
            "lap\u{202e}pot",
            "lap\u{200b}top",
            "lap\u{ad}top",
            "lap\u{2064}top",
            "lap\u{e0001}top",
            "lap\u{180e}top",
        ] {
            assert_eq!(
                shown_peer_name(unseen),
                Err(PeerNameRefusal::Unseen),
                "{unseen:?}"
            );
        }
        for everywhere in ["everywhere", "Everywhere", " EVERYWHERE "] {
            assert_eq!(
                shown_peer_name(everywhere),
                Err(PeerNameRefusal::Everywhere)
            );
        }
    }

    #[test]
    fn a_refusal_says_what_a_name_must_be() {
        assert_eq!(
            PeerNameRefusal::TooLong { characters: 70 }.to_string(),
            "A name runs to 64 characters at most; this one has 70"
        );
        assert_eq!(
            PeerNameRefusal::Everywhere.to_string(),
            "\"everywhere\" names every server at once, so nothing may be named so"
        );
    }
}
