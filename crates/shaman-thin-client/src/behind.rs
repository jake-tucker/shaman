//! A Remote the phone and it can no longer work together across until one of
//! the two is updated (ADR-0078): which side that is, and how the phone says
//! so. The Pairing stands either way, so updating the side behind heals it
//! with no second pairing.
//!
//! The words are the phone's own, since a Server's say "update Shaman here"
//! of a side that, on a phone, is the app; none says "version" or "floor".

use shaman_protocol::{RemoteStatus, SessionErrorCode};

/// Which side of a Pairing is too old for the other to work with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Behind {
    /// The app: the Server no longer works with one as old as it.
    App,
    /// The Server: its Shaman is too old for the app to work with.
    Server,
}

impl Behind {
    /// The side a refusal under `code` says is behind, where it says one is.
    pub fn of_code(code: SessionErrorCode) -> Option<Self> {
        match code {
            SessionErrorCode::PairingPeerBehind => Some(Self::App),
            SessionErrorCode::PairingServerBehind => Some(Self::Server),
            _ => None,
        }
    }

    /// The side a Remote standing as `status` has behind, where it has one:
    /// one standing ahead of the phone has the app behind.
    pub fn of_status(status: RemoteStatus) -> Option<Self> {
        match status {
            RemoteStatus::Ahead => Some(Self::App),
            RemoteStatus::Behind => Some(Self::Server),
            RemoteStatus::Available | RemoteStatus::Revoked | RemoteStatus::Unavailable => None,
        }
    }

    /// What an act refused for it says, in the banner over the screen that
    /// asked.
    pub fn refusal(self) -> String {
        match self {
            Self::App => "That server no longer works with an app as old as this one: update the \
                          app, and the pairing works again"
                .to_owned(),
            Self::Server => "That server's Shaman is too old to work with this app: update Shaman \
                             there, and the pairing works again"
                .to_owned(),
        }
    }

    /// What the confirmation screen says where pairing is refused for it.
    pub fn pairing_refused(self) -> String {
        match self {
            Self::App => "That server no longer pairs with an app as old as this one: update the \
                          app, then pair again"
                .to_owned(),
            Self::Server => "That server's Shaman is too old to pair with this app: update Shaman \
                             there, then pair again"
                .to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_side_is_told_apart_by_the_refusal_or_the_standing_it_comes_by() {
        assert_eq!(
            Behind::of_code(SessionErrorCode::PairingPeerBehind),
            Some(Behind::App)
        );
        assert_eq!(
            Behind::of_code(SessionErrorCode::PairingServerBehind),
            Some(Behind::Server)
        );
        assert_eq!(Behind::of_code(SessionErrorCode::ServerTooOld), None);
        assert_eq!(Behind::of_status(RemoteStatus::Ahead), Some(Behind::App));
        assert_eq!(
            Behind::of_status(RemoteStatus::Behind),
            Some(Behind::Server)
        );
        assert_eq!(Behind::of_status(RemoteStatus::Revoked), None);
    }

    #[test]
    fn the_words_name_the_side_to_update_and_say_no_version_or_floor() {
        for (behind, side) in [
            (Behind::App, "update the app"),
            (Behind::Server, "update Shaman there"),
        ] {
            for words in [behind.refusal(), behind.pairing_refused()] {
                assert!(words.contains(side), "{words}");
                let lowered = words.to_lowercase();
                assert!(
                    !lowered.contains("version") && !lowered.contains("floor"),
                    "{words}"
                );
            }
        }
    }
}
