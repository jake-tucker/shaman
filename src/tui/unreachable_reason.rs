//! What the Client says of why a Remote cannot be reached, where its user can
//! do something about it rather than wait: in brief, on a line that names the
//! Remote, and in full, with what to do, wherever there is room for it. Each
//! names what is in the way first, so a line cut short still says it. A loss
//! for any other reason says no more than that the Remote is Unreachable.

use crate::protocol::UnreachableReason;

/// What the offer to try again says where a login is what is in the way.
pub(super) const LOG_IN_TO_TRY_AGAIN: &str = "Log in to try again";

/// What the offer to try again a Remote out of reach for `reason` says: where
/// a login is what is in the way, that it is needed, since that offer leads
/// to the login rather than trying again at once.
pub(super) fn offer(reason: Option<&UnreachableReason>) -> &'static str {
    match reason {
        Some(UnreachableReason::RelayLoginNeeded { .. }) => LOG_IN_TO_TRY_AGAIN,
        Some(
            UnreachableReason::RelayCapReached { .. }
            | UnreachableReason::RelayDifferentAccounts { .. },
        )
        | None => "Try again",
    }
}

/// `reason` in brief.
pub(super) fn brief(reason: &UnreachableReason) -> String {
    match reason {
        UnreachableReason::RelayCapReached { limit, .. } => {
            let connections = if *limit == 1 {
                "connection"
            } else {
                "connections"
            };
            format!("Relay cap reached: {limit} joined {connections}")
        }
        UnreachableReason::RelayDifferentAccounts { relay, .. } => {
            format!("Accounts differ at {relay}")
        }
        UnreachableReason::RelayLoginNeeded { relay } => format!("login needed at {relay}"),
    }
}

/// `reason` in full, with what its user can do about it.
pub(super) fn in_full(reason: &UnreachableReason) -> String {
    match reason {
        UnreachableReason::RelayCapReached { relay, limit } => {
            let connections = if *limit == 1 {
                "connection"
            } else {
                "connections"
            };
            format!(
                "Your account has reached the relay's cap of {limit} {connections} joined at \
                 once at {relay}, so it joins no more until one ends; ask the relay's operator \
                 to raise the cap if that is too few"
            )
        }
        UnreachableReason::RelayDifferentAccounts { relay, account } => format!(
            "Accounts differ at the relay at {relay}: this server is logged in there as {} ({}) \
             and the remote under another account, and a relay joins only servers logged in \
             under the same one, so log this server in there as the user the remote is logged \
             in as, or pair the two directly",
            account.username, account.provider
        ),
        UnreachableReason::RelayLoginNeeded { relay } => format!(
            "Login needed at the relay at {relay}: it joins this server to nothing until this \
             server logs in there, so log in to try again"
        ),
    }
}
