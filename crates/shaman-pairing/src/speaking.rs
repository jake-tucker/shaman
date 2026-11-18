//! What each side of a Pairing speaks, and how it judges what the other
//! states (ADR-0078).
//!
//! Each side states its protocol version on everything it says — a Peer on
//! every request, a Serving Server on every answer — under
//! [`PAIRING_PROTOCOL_HEADER`], and accepts the other where that is at or
//! above its own floor. Where it is not, the side that judged refuses, saying
//! which side is behind, and the Pairing stands, so updating the side behind
//! heals it with no second ceremony.
//!
//! A Peer states its floor on every request as well, under
//! [`PAIRING_FLOOR_HEADER`], so a Serving Server older than the Peer speaks
//! with refuses what it asks before doing any of it, as it refuses a Peer
//! older than it speaks with: nothing is done that the Peer would then
//! refuse to read the answer to. The Peer judges each answer's stated version
//! too, so one from a Serving Server that does neither is never read.

use http::{HeaderMap, header::HeaderValue};

use crate::{
    PAIRING_FLOOR_HEADER, PAIRING_PROTOCOL_HEADER, PairingFailure,
    protocol::{PROTOCOL_FLOOR, PROTOCOL_VERSION, SessionErrorCode},
};

/// The protocol one side of a Pairing speaks: the version it states, and the
/// oldest it still speaks with. Injectable, so a test can stand a side at any
/// release.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairingProtocol {
    pub version: u32,
    pub floor: u32,
}

impl PairingProtocol {
    /// What this build speaks.
    pub const CURRENT: Self = Self {
        version: PROTOCOL_VERSION,
        floor: PROTOCOL_FLOOR,
    };

    /// Whether this side speaks with one stating `version`: one at or above
    /// its floor, however much newer.
    pub fn speaks_with(self, version: u32) -> bool {
        version >= self.floor
    }

    /// Whether this side speaks with whoever said what carried `headers`:
    /// one stating no version, or none this side reads, is older than any
    /// floor, since every side at or above one states it.
    pub fn speaks_with_stated(self, headers: &HeaderMap) -> bool {
        stated_version(headers).is_some_and(|version| self.speaks_with(version))
    }

    /// Whether this side, as a Serving Server, speaks with the Peer that
    /// asked carrying `headers`, and if not which of the two is behind: the
    /// Peer, stating a version below this side's floor or none at all, or
    /// this side, below the floor the Peer states.
    pub fn judge_peer(self, headers: &HeaderMap) -> Result<(), Behind> {
        if !self.speaks_with_stated(headers) {
            return Err(Behind::Peer);
        }
        if stated_number(headers, PAIRING_FLOOR_HEADER).is_some_and(|floor| self.version < floor) {
            return Err(Behind::Server);
        }
        Ok(())
    }

    /// The headers a Peer states this side by on every request: its version
    /// and its floor.
    pub fn stated(self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(PAIRING_PROTOCOL_HEADER, self.header_value());
        headers.insert(PAIRING_FLOOR_HEADER, HeaderValue::from(self.floor));
        headers
    }

    /// This side's version, as it is stated.
    pub fn header_value(self) -> HeaderValue {
        HeaderValue::from(self.version)
    }
}

impl Default for PairingProtocol {
    fn default() -> Self {
        Self::CURRENT
    }
}

/// The version stated in `headers`, where one is.
pub fn stated_version(headers: &HeaderMap) -> Option<u32> {
    stated_number(headers, PAIRING_PROTOCOL_HEADER)
}

/// The number stated in `headers` under `name`, where one is.
fn stated_number(headers: &HeaderMap, name: &str) -> Option<u32> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// Which side of a Pairing is older than the other still speaks with, and so
/// which to update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Behind {
    /// The redeeming side: the Serving Server refuses it.
    Peer,
    /// The Serving Server: the redeeming side refuses it.
    Server,
}

impl Behind {
    /// What was asked over a standing Pairing is refused for: worded for the
    /// redeeming side's user, who reads it, and naming the side to update.
    pub fn refusal(self) -> PairingFailure {
        match self {
            Self::Peer => PairingFailure::new(
                SessionErrorCode::PairingPeerBehind,
                "That server no longer works with a Shaman as old as this one: update Shaman \
                 here, and the pairing works again",
            ),
            Self::Server => PairingFailure::new(
                SessionErrorCode::PairingServerBehind,
                "That server's Shaman is too old to work with this one: update Shaman there, and \
                 the pairing works again",
            ),
        }
    }

    /// What the ceremony that would form a Pairing is refused for, worded as
    /// [`Self::refusal`] is.
    pub fn pairing_refused(self) -> PairingFailure {
        match self {
            Self::Peer => PairingFailure::new(
                SessionErrorCode::PairingPeerBehind,
                "That server no longer pairs with a Shaman as old as this one: update Shaman \
                 here, then pair again",
            ),
            Self::Server => PairingFailure::new(
                SessionErrorCode::PairingServerBehind,
                "That server's Shaman is too old to pair with this one: update Shaman there, then \
                 pair again",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_side_speaks_with_any_at_or_above_its_floor_however_much_newer() {
        let speaking = PairingProtocol {
            version: 130,
            floor: 120,
        };

        assert!(!speaking.speaks_with(119));
        assert!(speaking.speaks_with(120));
        assert!(speaking.speaks_with(125));
        assert!(speaking.speaks_with(400));
    }

    #[test]
    fn a_side_states_its_version_and_one_stating_none_is_older_than_any_floor() {
        let older = PairingProtocol {
            version: 120,
            floor: 118,
        };
        let newer = PairingProtocol {
            version: 125,
            floor: 121,
        };

        assert_eq!(stated_version(&older.stated()), Some(120));
        assert!(newer.speaks_with_stated(&newer.stated()));
        assert!(older.speaks_with_stated(&newer.stated()));
        assert!(!newer.speaks_with_stated(&older.stated()));
        assert!(!older.speaks_with_stated(&HeaderMap::new()));
        let mut garbled = HeaderMap::new();
        garbled.insert(PAIRING_PROTOCOL_HEADER, HeaderValue::from_static("v120"));
        assert!(!older.speaks_with_stated(&garbled));
    }

    #[test]
    fn a_serving_side_judges_a_peer_by_its_floor_and_its_own_by_the_peers() {
        let serving = PairingProtocol {
            version: 125,
            floor: 120,
        };
        let peer = |version, floor| PairingProtocol { version, floor }.stated();

        assert_eq!(serving.judge_peer(&peer(120, 110)), Ok(()));
        assert_eq!(serving.judge_peer(&peer(140, 125)), Ok(()));
        assert_eq!(serving.judge_peer(&peer(119, 110)), Err(Behind::Peer));
        assert_eq!(serving.judge_peer(&peer(140, 126)), Err(Behind::Server));
        assert_eq!(
            serving.judge_peer(&HeaderMap::new()),
            Err(Behind::Peer),
            "a Peer stating nothing is older than any floor"
        );
        let mut no_floor = HeaderMap::new();
        no_floor.insert(PAIRING_PROTOCOL_HEADER, HeaderValue::from(130_u32));
        assert_eq!(serving.judge_peer(&no_floor), Ok(()));
    }

    #[test]
    fn a_refusal_names_the_side_to_update_in_words_that_say_no_version_or_floor() {
        for (behind, code, side) in [
            (Behind::Peer, SessionErrorCode::PairingPeerBehind, "here"),
            (
                Behind::Server,
                SessionErrorCode::PairingServerBehind,
                "there",
            ),
        ] {
            for refused in [behind.refusal(), behind.pairing_refused()] {
                assert_eq!(refused.code, code);
                assert!(
                    refused.message.contains(&format!("update Shaman {side}")),
                    "{}",
                    refused.message
                );
                let words = refused.message.to_lowercase();
                assert!(
                    !words.contains("version") && !words.contains("floor"),
                    "{}",
                    refused.message
                );
            }
        }
    }
}
