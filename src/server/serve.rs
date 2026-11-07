//! The Serving routes that decide for a Client what `/serve` would do: making
//! the Server ready to issue an Invite, and issuing one by the kinds of way it is
//! to offer, so no surface keeps a rule of its own for what is turned on or
//! ticked. They are server administration, refused to Peers, and no Sidekick
//! Tool offers them (ADR-0043).

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};

use super::{
    AppState, SettingChanged, decode_session_command, is_authenticated,
    operations::ServingNotAdopted, session_error_response, settings_mutation_response,
};
use crate::{
    protocol::{
        InviteRelays, IssueInviteRequest, ListenerState, OfferInviteRequest, OfferedInvite, Relay,
        RelayState, RelayWithheld, ServingPreparation, ServingStanding, SessionErrorCode,
        SettingMutation, SettingsSnapshot, Way,
    },
    relay_said::relay_standing,
    relays::relay_address,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/pairing/serving",
            get(serving_standing).post(prepare_serving),
        )
        .route("/v1/pairing/invites/offered", post(offer_invite))
}

async fn serving_standing(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &state.descriptor.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(ServingStanding {
        enabled: state.settings.borrow().settings.serving.enabled,
        listener: state.serving.listener().borrow().clone(),
    })
    .into_response()
}

/// Whether Serving was turned on, what that left in force, and why its listener
/// could not follow, where it could not.
struct Prepared {
    turned_on: bool,
    settings: Option<SettingsSnapshot>,
    not_adopted: Option<ServingNotAdopted>,
}

/// Turns Serving on where it is off. Serving whose listener failed — Serving
/// itself could not start, say, its identity key's store not answering — is
/// asked for again, as one off is turned on: the Server tries once more, and says
/// why where it still cannot. What is in force decides, and what a Config
/// Document says only once it is in force (ADR-0067). It is a change of the
/// Setting, written to the document as the settings panel writes it.
#[allow(clippy::result_large_err)]
async fn turn_serving_on(state: &AppState) -> Result<Prepared, Response> {
    let enabled = state.settings.borrow().settings.serving.enabled;
    let failed = matches!(
        *state.serving.listener().borrow(),
        ListenerState::Failed { .. }
    );
    if enabled && !failed {
        return Ok(Prepared {
            turned_on: false,
            settings: None,
            not_adopted: None,
        });
    }
    let SettingChanged { snapshot, serving } = state
        .operations
        .change_setting(SettingMutation::ServingEnabled { value: Some(true) })
        .await
        .map_err(|error| settings_mutation_response(&error))?;
    Ok(Prepared {
        turned_on: !enabled,
        settings: Some(snapshot),
        not_adopted: serving,
    })
}

async fn prepare_serving(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &state.descriptor.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let prepared = match turn_serving_on(&state).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    if let Some(not_adopted) = prepared.not_adopted {
        return listener_failed(not_adopted.detail());
    }
    let settings = state.settings.borrow().settings.serving.clone();
    match state.serving.own_addresses(&settings) {
        Ok(addresses) => Json(ServingPreparation {
            turned_on: prepared.turned_on,
            settings: prepared.settings,
            addresses,
        })
        .into_response(),
        Err(error) => listener_failed(format!("could not find this machine's addresses: {error}")),
    }
}

fn listener_failed(detail: String) -> Response {
    session_error_response(
        StatusCode::CONFLICT,
        SessionErrorCode::ServingListenerFailed,
        detail,
    )
}

fn no_way_to_offer(message: String) -> Response {
    session_error_response(
        StatusCode::CONFLICT,
        SessionErrorCode::InvalidInviteWays,
        message,
    )
}

/// The Relays an Invite is to offer as its asker said, and what is worth saying
/// of those it leaves out or that may not answer.
#[derive(Default)]
struct ChosenRelays {
    ways: Vec<Way>,
    /// Why each Relay it could have offered it does not.
    left_out: Vec<String>,
    /// That a Relay it does offer is not answering now.
    unreachable: Vec<String>,
}

/// Why an Invite cannot offer `relay`, withheld as `withheld` says, with what
/// would put that right: what its row says and what to run.
fn why_withheld(relay: &Relay, withheld: RelayWithheld) -> String {
    let address = &relay.address;
    match withheld {
        RelayWithheld::LoginNeeded => {
            let under_way = relay
                .login
                .as_ref()
                .is_some_and(|login| !login.outcome.is_settled());
            if under_way {
                format!(
                    "The Relay at {address} needs a login, and one is under way: `suru relay \
                     login {address}` shows it"
                )
            } else {
                format!(
                    "The Relay at {address} cannot be offered: {}. `suru relay login {address}` \
                     logs in there",
                    relay_standing(relay)
                )
            }
        }
        RelayWithheld::NotServedThrough => format!(
            "This Server does not Serve through the Relay at {address}, so an Invite cannot \
             offer it. `suru relay serve-through {address} on` makes this machine reachable \
             through it"
        ),
    }
}

/// The Relays `asked` names among those the Server holds, by the rule every
/// surface offers one by. A Relay asked for by name that cannot be offered fails
/// the whole, naming why of each, where one asked for by kind is left out and
/// said so. An address no Relay could be at fails as a bad address.
#[allow(clippy::result_large_err)]
fn chosen_relays(asked: &InviteRelays, held: &[Relay]) -> Result<ChosenRelays, Response> {
    let mut chosen = ChosenRelays::default();
    let offer = |relay: &Relay, chosen: &mut ChosenRelays| {
        chosen.ways.push(Way::Relay(relay.address.clone()));
        if relay.state == RelayState::Unreachable {
            chosen.unreachable.push(format!(
                "The Relay at {} is offered but not answering now: {}",
                relay.address,
                relay_standing(relay)
            ));
        }
    };
    match asked {
        InviteRelays::None => {}
        InviteRelays::Every => {
            for relay in held {
                match RelayWithheld::of(relay) {
                    None => offer(relay, &mut chosen),
                    Some(withheld) => chosen.left_out.push(why_withheld(relay, withheld)),
                }
            }
        }
        InviteRelays::Only(addresses) => {
            let mut refusals = Vec::new();
            for asked in addresses {
                let address = relay_address(asked).map_err(|failure| {
                    session_error_response(failure.status(), failure.code, failure.message)
                })?;
                if chosen.ways.contains(&Way::Relay(address.clone())) {
                    continue;
                }
                match held.iter().find(|relay| relay.address == address) {
                    None => refusals.push(no_relay_at(&address, held)),
                    Some(relay) => match RelayWithheld::of(relay) {
                        None => offer(relay, &mut chosen),
                        Some(withheld) => refusals.push(why_withheld(relay, withheld)),
                    },
                }
            }
            if !refusals.is_empty() {
                return Err(no_way_to_offer(refusals.join("\n")));
            }
        }
    }
    Ok(chosen)
}

/// What is said of an address the Server holds no Relay at, naming those it does
/// hold, since the two may differ only in how they are written.
fn no_relay_at(address: &str, held: &[Relay]) -> String {
    let holds = if held.is_empty() {
        "it holds none".to_owned()
    } else {
        format!(
            "it holds {}",
            held.iter()
                .map(|relay| relay.address.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!("This Server holds no Relay at {address}: {holds}. `suru relay add {address}` adds it")
}

/// Issues the Invite `/serve` would have ticked, with what its asker left out
/// left out. What can be told to fail beforehand fails before Serving is turned
/// on, so asking for what cannot be offered opens nothing; what comes to fail
/// after says Serving is on now, as it is.
async fn offer_invite(State(state): State<AppState>, request: Request) -> Response {
    let request = match decode_session_command::<OfferInviteRequest>(
        &state,
        request,
        "Invite issuance",
    )
    .await
    {
        Ok(request) => request,
        Err(response) => return response,
    };
    let held = state.relays.list().relays;
    let chosen = match chosen_relays(&request.relays, &held) {
        Ok(chosen) => chosen,
        Err(response) => return response,
    };
    if !request.addresses && chosen.ways.is_empty() {
        return no_way_to_offer(nothing_to_offer(&[], &chosen.left_out, &held));
    }
    let prepared = match turn_serving_on(&state).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let turned_on = turned_on_said(&state, &prepared);
    let mut notes = Vec::from_iter(turned_on.clone());
    let settings = state.settings.borrow().settings.serving.clone();
    let listener = state.serving.listener().borrow().clone();
    let mut said_of_addresses = Vec::new();
    let mut ways = Vec::new();
    if request.addresses {
        match state.serving.own_addresses(&settings) {
            Ok(addresses) if matches!(listener, ListenerState::Open { .. }) => {
                if addresses.is_empty() {
                    said_of_addresses.push(no_address_listened_at(&listener));
                }
                ways.extend(addresses);
            }
            Ok(_) => {
                said_of_addresses.push(listener_says(&listener, prepared.not_adopted.as_ref()))
            }
            Err(error) => said_of_addresses.push(format!(
                "This machine's addresses could not be found: {error}"
            )),
        }
    }
    ways.extend(chosen.ways);
    let ways_were_found = !ways.is_empty();
    match state
        .serving
        .issue_invite(IssueInviteRequest { ways })
        .await
    {
        Ok(issued) => {
            notes.extend(said_of_addresses);
            notes.extend(chosen.left_out);
            notes.extend(chosen.unreachable);
            let minutes = state.serving.invite_ttl().as_secs() / 60;
            notes.push(format!(
                "It can be redeemed once, within {minutes} minutes, and replaces any Invite \
                 issued before it"
            ));
            Json(OfferedInvite { issued, notes }).into_response()
        }
        // Nothing asked of Serving itself failed, so the reason is that there
        // was nothing to ask for.
        Err(failure) if !ways_were_found && failure.code == SessionErrorCode::InvalidInviteWays => {
            let message = nothing_to_offer(&said_of_addresses, &chosen.left_out, &held);
            no_way_to_offer(said_after(turned_on.as_deref(), message))
        }
        Err(failure) => session_error_response(
            failure.status(),
            failure.code,
            said_after(turned_on.as_deref(), failure.message),
        ),
    }
}

/// What is said of Serving having been turned on, where this did that: its
/// Setting on says nothing of its having started, which an identity key that
/// cannot be read keeps it from (ADR-0050).
fn turned_on_said(state: &AppState, prepared: &Prepared) -> Option<String> {
    prepared.turned_on.then(|| {
        if state.serving.is_serving() {
            "Serving is now on: this machine can be reached by the Servers paired with it"
                .to_owned()
        } else {
            "Serving is asked for now (serving.enabled), and could not start".to_owned()
        }
    })
}

/// `message`, saying after it what `turned_on` says came of Serving along the way.
fn said_after(turned_on: Option<&str>, message: String) -> String {
    match turned_on {
        Some(turned_on) => format!("{message}\n{turned_on}"),
        None => message,
    }
}

/// Why an Invite offers none of this machine's addresses, its listener not
/// listening as `listener` says.
fn listener_says(listener: &ListenerState, not_adopted: Option<&ServingNotAdopted>) -> String {
    match listener {
        ListenerState::Failed { reason } => format!(
            "The Serving listener is not listening, so this offers none of this machine's \
             addresses: {reason}"
        ),
        ListenerState::Open { .. } | ListenerState::Off => match not_adopted {
            Some(not_adopted) => format!(
                "The Serving listener is not listening, so this offers none of this machine's \
                 addresses: {}",
                not_adopted.detail()
            ),
            None => "The Serving listener is off (serving.listener), so this offers none of this \
                     machine's addresses"
                .to_owned(),
        },
    }
}

/// What is said of a listener open where no other machine could reach it, or on
/// a machine with no address another could reach.
fn no_address_listened_at(listener: &ListenerState) -> String {
    match listener {
        ListenerState::Open { address } if address.ip().to_canonical().is_loopback() => format!(
            "The Serving listener listens at {address}, which only this machine can reach \
             (serving.bindAddress), so this offers none of its addresses"
        ),
        _ => "This machine has no address another machine could reach it at, so this offers none"
            .to_owned(),
    }
}

/// Why an Invite has no way at all to offer: what was said of this machine's
/// addresses and of each Relay left out, or that it holds none.
fn nothing_to_offer(addresses: &[String], left_out: &[String], held: &[Relay]) -> String {
    let mut why = vec!["An Invite has no way to offer".to_owned()];
    why.extend_from_slice(addresses);
    why.extend_from_slice(left_out);
    if held.is_empty() {
        why.push("This Server holds no Relay; `suru relay add <address>` adds one".to_owned());
    }
    why.join("\n")
}
