//! What every surface says of a Relay, so `/relay`, `/serve` and the command
//! line say one thing of it, whoever is reading.

use crate::protocol::{Relay, RelayLoginOutcome, RelayLoginRefusal, RelayState};

/// What a Relay not Served through says before it is: the one thing that opens
/// the machine to.
pub const REACHABLE: &str = "Serving through it makes this machine reachable through this relay";

/// Why an Invite cannot offer a Relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayWithheld {
    /// The Server holds no Login there that stands.
    LoginNeeded,
    /// The Server does not Serve through it.
    NotServedThrough,
}

impl RelayWithheld {
    /// Why an Invite cannot offer `relay`, where it cannot: what the Server asks
    /// of a Relay an Invite offers, so every surface that lists one says so
    /// before the Server is asked.
    pub fn of(relay: &Relay) -> Option<Self> {
        if relay.state == RelayState::LoginNeeded {
            Some(Self::LoginNeeded)
        } else if !relay.serve_through {
            Some(Self::NotServedThrough)
        } else {
            None
        }
    }

    /// What a row says of it.
    pub fn brief(self) -> &'static str {
        match self {
            Self::LoginNeeded => "login needed",
            Self::NotServedThrough => "not served through",
        }
    }
}

/// What is said of a login that ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginEnded {
    pub text: String,
    /// Whether it says the login formed no Login.
    pub failed: bool,
}

/// What is said of a login at `address` that ended in `outcome`; nothing of one
/// still under way.
pub fn login_ended(address: &str, outcome: &RelayLoginOutcome) -> Option<LoginEnded> {
    let refused = |text: String| LoginEnded { text, failed: true };
    Some(match outcome {
        RelayLoginOutcome::Pending => return None,
        RelayLoginOutcome::Done { account } => LoginEnded {
            text: format!("Logged in at {address} as {}", account.username),
            failed: false,
        },
        RelayLoginOutcome::Refused {
            reason: RelayLoginRefusal::NotAdmitted,
            ..
        } => refused(format!(
            "You are not admitted to {address}; ask the relay's operator to admit you"
        )),
        RelayLoginOutcome::Refused {
            reason: RelayLoginRefusal::LoginsCapReached { limit },
            ..
        } => refused(format!(
            "Your account already has {} logged in at {address}, as many as the relay's \
             operator allows; remove the relay from a server that no longer needs it, or ask \
             the operator to raise the cap",
            servers(*limit)
        )),
        RelayLoginOutcome::Refused { message, .. } => {
            refused(format!("The login at {address} ended: {message}"))
        }
    })
}

/// How a Relay stands, as its row says it, leaving out a login under way there,
/// which each surface says in its own way.
pub fn relay_standing(relay: &Relay) -> String {
    match relay.state {
        RelayState::LoggedIn => relay.account.as_ref().map_or_else(
            || "Logged in".to_owned(),
            |account| format!("Logged in as {} ({})", account.username, account.provider),
        ),
        RelayState::LoginNeeded => match relay.login.as_ref().map(|login| &login.outcome) {
            Some(RelayLoginOutcome::Refused {
                reason: RelayLoginRefusal::NotAdmitted,
                ..
            }) => "Login needed · not admitted; ask the relay's operator".to_owned(),
            Some(RelayLoginOutcome::Refused {
                reason: RelayLoginRefusal::LoginsCapReached { limit },
                ..
            }) => format!(
                "Login needed · at the cap of {}; ask the relay's operator",
                servers(*limit)
            ),
            Some(RelayLoginOutcome::Refused { message, .. }) => {
                format!("Login needed · {message}")
            }
            _ => "Login needed".to_owned(),
        },
        RelayState::Unreachable => relay.unreachable.as_ref().map_or_else(
            || "Unreachable".to_owned(),
            |unreachable| format!("Unreachable · {}", unreachable.message),
        ),
    }
}

/// What is said where a login was asked for at `relay` and its Login stands, so
/// the Server began none.
pub fn login_stands(relay: &Relay) -> String {
    format!(
        "{} needs no login: {}",
        relay.address,
        relay_standing(relay)
    )
}

/// What a row says of the Server Serving through `relay`, where it does — and
/// of what it waits on, where that is anything: a login there, or Serving
/// itself, which `serving` says is on.
pub fn serve_through_status(relay: &Relay, serving: bool) -> Option<String> {
    if !relay.serve_through {
        return None;
    }
    Some(match waits_on(relay, serving) {
        Some(waits) => format!("Serves through once {waits}"),
        None => "Serving through".to_owned(),
    })
}

/// What the Server Serving through `relay` waits on, where it waits.
fn waits_on(relay: &Relay, serving: bool) -> Option<&'static str> {
    match (relay.state == RelayState::LoginNeeded, serving) {
        (true, true) => Some("logged in"),
        (true, false) => Some("logged in and serving is on"),
        (false, false) => Some("serving is on"),
        (false, true) => None,
    }
}

/// What is said of the Server choosing whether it Serves through `relay`, as the
/// Server answered it, `turning_on` naming what turns Serving on for this reader.
pub fn serve_through_said(relay: &Relay, serving: bool, turning_on: &str) -> String {
    let address = &relay.address;
    if !relay.serve_through {
        return format!("This server no longer serves through {address}");
    }
    let once = match (relay.state == RelayState::LoginNeeded, serving) {
        (true, true) => " once it is logged in there".to_owned(),
        (true, false) => {
            format!(" once it is logged in there and serving is on; {turning_on} turns serving on")
        }
        (false, false) => format!(" once serving is on; {turning_on} turns serving on"),
        (false, true) => String::new(),
    };
    format!("This server serves through {address}{once}")
}

/// `count` Servers, said as a reader says it.
fn servers(count: u32) -> String {
    if count == 1 {
        "1 server".to_owned()
    } else {
        format!("{count} servers")
    }
}
