//! Pairing with a Serving Server by its Invite (ADR-0077): reading the Invite
//! for the confirmation screen, dialling nothing, and confirming it, which
//! redeems it as an ordinary Peer.
//!
//! A phone holds one Remote for each Server, so an Invite from a Server it
//! already holds replaces that Pairing. It offers nothing to pair back and
//! dials no IPv6 link-local way. Confirming keeps dialling for a few seconds
//! while no way answers, so that allowing the local network prompt is
//! followed by success without a tap; nothing is kept from a pairing that
//! fails.

use std::{net::SocketAddr, sync::Arc};

use shaman_pairing::{PairingFailure, ParsedInvite, Redemption};
use shaman_protocol::{RedeemInviteRequest, SessionErrorCode, UnreachableReason, Way};

use crate::{
    Inner,
    relays::{self, LoginOffer, RelayHint},
};

/// What the confirmation screen shows of an Invite, read without dialling
/// anything.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum InviteReading {
    Readable {
        invite: ReadInvite,
    },
    /// The text is not an Invite this phone can read, and why.
    Unreadable {
        message: String,
    },
}

/// An Invite, as the confirmation screen shows it before anything is
/// dialled.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ReadInvite {
    /// The Serving Server's hostname, as its Invite gives it.
    pub hostname: String,
    /// The fingerprint of its identity key, whole.
    pub fingerprint: String,
    /// The ways it offers, in the order they are dialled in.
    pub ways: Vec<OfferedWay>,
    /// The name the Remote is first offered under: the one this phone holds
    /// it by already, or its hostname.
    pub remote_name: String,
    /// Said where this phone holds the Server already: under which name, and
    /// that pairing again replaces that Pairing.
    pub held: Option<String>,
    /// The name this phone goes by, as it will give itself to the Server.
    pub device_name: String,
}

/// A way an Invite offers.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct OfferedWay {
    /// The way as it reads: a direct way's address, or a Relay's.
    pub address: String,
    /// Said of a way the phone will not dial, or not until it logs in.
    pub note: Option<String>,
    /// The offer to log in at a Relay way's Relay, where this phone's Login
    /// there needs one: pairing needs it only where no other way answers.
    pub login: Option<LoginOffer>,
}

/// What came of confirming a pairing.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PairingOutcome {
    /// The Remote is paired under `name`, and the Outlook turned toward it.
    Paired { name: String },
    /// Nothing was paired, in the Server's words where it said them, with
    /// what the phone offers next.
    Failed { message: String, again: TryInstead },
    /// The Remote's name is not one it can be held by, said beside it.
    RemoteNameRefused { message: String },
    /// The phone's name is not one the Server would keep, said beside it.
    DeviceNameRefused { message: String },
    /// No way answered, and a Relay way would once this phone logs in at
    /// its Relay, as `message` says: pairing goes on by itself once the
    /// login `login` offers is done, while the Invite lives; or it may be
    /// tried again as it is, as where a direct way failed for a moment.
    LoginNeeded {
        message: String,
        login: LoginOffer,
        try_again: String,
    },
}

/// What a failed pairing offers next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TryInstead {
    /// The Invite is dead, spent, expired or superseded, or is not one this
    /// phone can redeem: only a new one will pair.
    ScanAgain,
    /// No way answered, or the phone could not keep what it was given: the
    /// same Invite may pair, until it dies.
    TryAgain,
}

/// What the screen where pairing begins says to do.
pub(crate) const PROMPT: &str = "On the server, run /serve in Shaman or `shaman serve invite`, copy the invite, and paste it here.";

/// Reads `text` as an Invite for the confirmation screen, dialling nothing.
pub(crate) fn read(inner: &Inner, text: &str) -> InviteReading {
    let invite = match shaman_pairing::parse_invite(text.trim()) {
        Ok(invite) => invite,
        Err(failure) => {
            return InviteReading::Unreadable {
                message: failure.message,
            };
        }
    };
    let held = held_as(inner, &invite);
    // A login may be asked for at a Relay the Invite offers before its
    // Server is a Remote.
    *inner
        .invited_relays
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = invite
        .ways
        .iter()
        .filter_map(|way| match way {
            Way::Relay(relay) => Some(relay.clone()),
            _ => None,
        })
        .collect();
    InviteReading::Readable {
        invite: ReadInvite {
            fingerprint: shaman_pairing::fingerprint(&invite.server_key),
            ways: invite
                .ways
                .iter()
                .map(|way| {
                    let hint = inner.relays.hint(std::slice::from_ref(way));
                    OfferedWay {
                        address: way.to_string(),
                        note: match &hint {
                            Some(RelayHint::LoginNeeded { .. }) => Some("login needed".to_owned()),
                            Some(RelayHint::OtherAccount { .. }) => {
                                Some("logged in under another account".to_owned())
                            }
                            None => (!dialled(way)).then(|| "not dialled from a phone".to_owned()),
                        },
                        login: hint.as_ref().map(RelayHint::offer),
                    }
                })
                .collect(),
            remote_name: held.clone().unwrap_or_else(|| invite.hostname.clone()),
            held: held.map(|name| {
                format!(
                    "This phone is already paired with this server as {name}. Pairing again \
                     replaces that pairing."
                )
            }),
            hostname: invite.hostname,
            device_name: inner.device.borrow().name.clone(),
        },
    }
}

/// Confirms the pairing `text` invites to, naming the Remote `remote_name`
/// and this phone `device_name`: redeems it, replacing any Remote of the
/// same Server, keeps the phone's name and turns the Outlook toward the
/// Remote. Nothing is kept where it fails.
pub(crate) async fn confirm(
    inner: &Arc<Inner>,
    text: &str,
    remote_name: &str,
    device_name: &str,
) -> PairingOutcome {
    let mut invite = match shaman_pairing::parse_invite(text.trim()) {
        Ok(invite) => invite,
        Err(failure) => {
            return PairingOutcome::Failed {
                message: failure.message,
                again: TryInstead::ScanAgain,
            };
        }
    };
    let device_name = match shaman_pairing::shown_peer_name(device_name) {
        Ok(name) => name,
        Err(refusal) => {
            return PairingOutcome::DeviceNameRefused {
                message: refusal.to_string(),
            };
        }
    };
    let remote_name = remote_name.trim().to_owned();
    if let Err(failure) = shaman_pairing::validate_remote_name(&remote_name) {
        return PairingOutcome::RemoteNameRefused {
            message: failure.message,
        };
    }
    if let Some(other) =
        inner.redeemer.records().into_iter().find(|stored| {
            stored.remote.name == remote_name && stored.public_key != invite.server_key
        })
    {
        return PairingOutcome::RemoteNameRefused {
            message: format!(
                "This phone holds another server as {}; choose another name",
                other.remote.name
            ),
        };
    }
    invite.ways.retain(dialled);
    if invite.ways.is_empty() {
        return PairingOutcome::Failed {
            message: "This invite offers no way a phone can dial.".to_owned(),
            again: TryInstead::ScanAgain,
        };
    }

    // One pairing at a time, so two never replace one another's Remote.
    let _pairing = inner.pairing.lock().await;
    let replaced = held_as(inner, &invite);
    let ways = invite.ways.clone();
    let remote = match keep_dialling(inner, invite, &remote_name, &device_name).await {
        Ok(remote) => remote,
        Err(failure) => {
            // Whatever was shown of it while it was under way goes.
            inner.remotes.refresh();
            // What this attempt's refused join said comes first; then how
            // this phone stands at each Relay now, which a refused join has
            // just changed where the Login there lapsed or stands under
            // another Account than the Server's.
            let relay_hint = refused_at(&failure).or_else(|| inner.relays.hint(&ways));
            return match relay_hint {
                // No way answered, or the one a Relay way's join was asked
                // at refused it for want of the login.
                Some(hint)
                    if matches!(
                        failure.code,
                        SessionErrorCode::PairingConnectionFailed
                            | SessionErrorCode::RelayLoginNeeded
                            | SessionErrorCode::RelayDifferentAccounts
                    ) =>
                {
                    login_needed(&hint)
                }
                _ => failed(failure),
            };
        }
    };
    let name = remote.name.clone();
    let turned = name.clone();
    // The Pairing is made, so it is shown as made, kept or not: a record
    // that could not be written now is written whole by the next change.
    let shown = inner
        .change_record_shown(move |record| {
            record.name = device_name;
            // The Server is the one it was, so where the Workspace last
            // turned toward on it is remembered under its old name, it is
            // remembered under its new.
            let remembered = replaced
                .as_ref()
                .and_then(|replaced| record.workspaces.remove(replaced));
            record.turned.retain(|held| Some(held) != replaced.as_ref());
            record.turn(&turned, remembered);
        })
        .await;
    if let Err(error) = shown {
        tracing::warn!("a pairing was made but this phone's records were not kept: {error}");
    }
    inner.listing.follow(&inner.remotes.held());
    let asked = inner.clone();
    let asked_name = name.clone();
    tokio::spawn(async move { asked.remotes.ask_ways(&asked_name).await });
    PairingOutcome::Paired { name }
}

/// Redeems `invite`, dialling its ways again while none answers, until
/// confirming has kept dialling as long as it does.
async fn keep_dialling(
    inner: &Inner,
    invite: ParsedInvite,
    remote_name: &str,
    device_name: &str,
) -> Result<shaman_protocol::Remote, PairingFailure> {
    let until = tokio::time::Instant::now() + inner.timings.pairing_keeps_dialling;
    loop {
        let redeemed = inner
            .redeemer
            .redeem_as(
                invite.clone(),
                RedeemInviteRequest {
                    invite: String::new().into(),
                    name: Some(remote_name.to_owned()),
                    ways: Vec::new(),
                    pair_back: false,
                },
                Redemption {
                    peer_name: Some(device_name.to_owned()),
                    replacing: true,
                },
            )
            .await;
        match redeemed {
            Err(failure)
                if failure.code == SessionErrorCode::PairingConnectionFailed
                    && tokio::time::Instant::now() + inner.timings.pairing_redial < until =>
            {
                tokio::time::sleep(inner.timings.pairing_redial).await;
            }
            redeemed => return redeemed,
        }
    }
}

/// A pairing no way answered but a Relay way past `hint`, which a login
/// leads past.
fn login_needed(hint: &RelayHint) -> PairingOutcome {
    let message = match hint {
        RelayHint::LoginNeeded { relay } => format!(
            "No way this invite offers answered. It offers the relay at {}, where this phone \
             needs to log in: log in there and pairing goes on by itself, or try again once this \
             phone can reach the server.",
            relays::shown(relay)
        ),
        RelayHint::OtherAccount { relay, account } => {
            relays::different_accounts(relay, account, "the server")
        }
    };
    PairingOutcome::LoginNeeded {
        message,
        login: hint.offer(),
        try_again: "Try again".to_owned(),
    }
}

/// What a Relay way's refused join says is in the way of `failure`, where
/// it says.
fn refused_at(failure: &PairingFailure) -> Option<RelayHint> {
    match failure.unreachable.clone()? {
        UnreachableReason::RelayLoginNeeded { relay } => Some(RelayHint::LoginNeeded { relay }),
        UnreachableReason::RelayDifferentAccounts { relay, account } => {
            Some(RelayHint::OtherAccount { relay, account })
        }
        _ => None,
    }
}

/// A pairing that failed with `failure`, in the words the confirmation
/// screen shows.
fn failed(failure: PairingFailure) -> PairingOutcome {
    match failure.code {
        SessionErrorCode::PairingConnectionFailed => PairingOutcome::Failed {
            message: "No way this invite offers answered. Check that the server is serving and \
                      that this phone can reach it."
                .to_owned(),
            again: TryInstead::TryAgain,
        },
        SessionErrorCode::RemoteNameConflict | SessionErrorCode::InvalidRemoteName => {
            PairingOutcome::RemoteNameRefused {
                message: failure.message,
            }
        }
        // Said in the phone's words: the Relay's own speak of a Server.
        SessionErrorCode::RelayCapReached => PairingOutcome::Failed {
            message: match failure.unreachable {
                Some(UnreachableReason::RelayCapReached { relay, limit }) => format!(
                    "The relay at {} joins at most {limit} connections at once for one account, \
                     and yours has as many. Try again once one ends.",
                    relays::shown(&relay)
                ),
                _ => "The relay joins no more connections for this account just now. Try again \
                      once one ends."
                    .to_owned(),
            },
            again: TryInstead::TryAgain,
        },
        // Where nothing said which Relay refused, still in the phone's words.
        SessionErrorCode::RelayLoginNeeded | SessionErrorCode::RelayDifferentAccounts => {
            PairingOutcome::Failed {
                message: "A relay this invite offers would not join this phone to the server. \
                          Check this phone's login there in Servers & device, then try again."
                    .to_owned(),
                again: TryInstead::TryAgain,
            }
        }
        SessionErrorCode::RemoteBusy | SessionErrorCode::PairingOutcomeUnknown => {
            PairingOutcome::Failed {
                message: failure.message,
                again: TryInstead::TryAgain,
            }
        }
        _ => PairingOutcome::Failed {
            message: failure.message,
            again: TryInstead::ScanAgain,
        },
    }
}

/// The name this phone holds the Server `invite` is from by, where it holds
/// it.
fn held_as(inner: &Inner, invite: &ParsedInvite) -> Option<String> {
    inner
        .redeemer
        .records()
        .into_iter()
        .find(|stored| stored.public_key == invite.server_key)
        .map(|stored| stored.remote.name)
}

/// Whether a phone dials `way`: every way but an IPv6 link-local one, which
/// names no interface a phone can choose.
fn dialled(way: &Way) -> bool {
    !matches!(way, Way::Direct(SocketAddr::V6(address)) if address.ip().is_unicast_link_local())
}
