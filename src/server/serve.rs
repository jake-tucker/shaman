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
        InviteOfferAnswer, InviteRelays, IssueInviteRequest, ListenerState, OfferInviteRequest,
        OfferedInvite, Relay, RelayState, ServingChange, ServingPreparation, ServingSettings,
        ServingStanding, SessionErrorCode, SettingMutation, SettingsSnapshot, Way, WayLeftOut,
    },
    relay_said::RelayWithheld,
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

/// What turning Serving on did, what that left in force, and why its listener
/// could not follow, where it could not.
struct Prepared {
    change: Option<ServingChange>,
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
            change: None,
            settings: None,
            not_adopted: None,
        });
    }
    let SettingChanged { snapshot, serving } = state
        .operations
        .change_setting(SettingMutation::ServingEnabled { value: Some(true) })
        .await
        .map_err(|error| settings_mutation_response(&error))?;
    let still_failed = matches!(
        *state.serving.listener().borrow(),
        ListenerState::Failed { .. }
    );
    let change = if !state.serving.is_serving() {
        Some(ServingChange::CouldNotStart)
    } else if !enabled {
        Some(ServingChange::TurnedOn)
    } else if failed && !still_failed {
        Some(ServingChange::Recovered)
    } else {
        None
    };
    Ok(Prepared {
        change,
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
            change: prepared.change,
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

/// The Relays an Invite is to offer as its asker said, and what is worth saying
/// of those it leaves out or that may not answer.
#[derive(Default)]
struct ChosenRelays {
    ways: Vec<Way>,
    /// Why each Relay it could have offered it does not.
    left_out: Vec<WayLeftOut>,
    /// Those it does offer that are not answering now.
    not_answering: Vec<Relay>,
}

/// Why an Invite cannot offer `relay`, withheld as `withheld` says.
fn way_withheld(relay: &Relay, withheld: RelayWithheld) -> WayLeftOut {
    match withheld {
        RelayWithheld::LoginNeeded => WayLeftOut::RelayLoginNeeded {
            relay: Box::new(relay.clone()),
        },
        RelayWithheld::NotServedThrough => WayLeftOut::RelayNotServedThrough {
            address: relay.address.clone(),
        },
    }
}

/// The Relays `asked` names among those the Server holds, by the rule every
/// surface offers one by. A Relay asked for by name that cannot be offered fails
/// the whole, saying why of each, where one asked for by kind is left out and
/// said so. An address no Relay could be at fails as a bad address.
#[allow(clippy::result_large_err)]
fn chosen_relays(asked: &InviteRelays, held: &[Relay]) -> Result<ChosenRelays, Response> {
    let mut chosen = ChosenRelays::default();
    let offer = |relay: &Relay, chosen: &mut ChosenRelays| {
        chosen.ways.push(Way::Relay(relay.address.clone()));
        if relay.state == RelayState::Unreachable {
            chosen.not_answering.push(relay.clone());
        }
    };
    match asked {
        InviteRelays::None => {}
        InviteRelays::Every => {
            for relay in held {
                match RelayWithheld::of(relay) {
                    None => offer(relay, &mut chosen),
                    Some(withheld) => chosen.left_out.push(way_withheld(relay, withheld)),
                }
            }
            if held.is_empty() {
                chosen.left_out.push(WayLeftOut::NoRelayHeld);
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
                    None => refusals.push(WayLeftOut::NoRelayAt {
                        address,
                        held: held.iter().map(|relay| relay.address.clone()).collect(),
                    }),
                    Some(relay) => match RelayWithheld::of(relay) {
                        None => offer(relay, &mut chosen),
                        Some(withheld) => refusals.push(way_withheld(relay, withheld)),
                    },
                }
            }
            if !refusals.is_empty() {
                return Err(answer(InviteOfferAnswer::RelayWithheld { why: refusals }));
            }
        }
    }
    Ok(chosen)
}

fn answer(answer: InviteOfferAnswer) -> Response {
    Json(answer).into_response()
}

/// Why this machine's addresses cannot be offered, whatever becomes of Serving,
/// as its Settings say: the listener off, or bound where no other machine could
/// reach it, or nothing to offer it at. Where only trying can tell, nothing.
fn addresses_cannot_be_offered(state: &AppState, settings: &ServingSettings) -> Option<WayLeftOut> {
    if !settings.listener {
        return Some(WayLeftOut::ListenerOff);
    }
    match state.serving.own_addresses(settings) {
        Ok(addresses) if addresses.is_empty() => Some(no_address_at(settings)),
        Ok(_) | Err(_) => None,
    }
}

/// Why no address is offered of a listener `settings` bind, there being none.
fn no_address_at(settings: &ServingSettings) -> WayLeftOut {
    let at = settings.bind_address.to_canonical();
    if at.is_loopback() {
        WayLeftOut::ListenerLoopback { at }
    } else {
        WayLeftOut::NoAddress
    }
}

/// This machine's addresses an Invite offers now, or why it offers none, the
/// listener standing as `listener` says.
fn addresses_offered(
    state: &AppState,
    settings: &ServingSettings,
    listener: &ListenerState,
    not_adopted: Option<&ServingNotAdopted>,
) -> Result<Vec<Way>, WayLeftOut> {
    match listener {
        ListenerState::Open { .. } => match state.serving.own_addresses(settings) {
            Ok(addresses) if addresses.is_empty() => Err(no_address_at(settings)),
            Ok(addresses) => Ok(addresses),
            Err(error) => Err(WayLeftOut::AddressesUnknown {
                reason: error.to_string(),
            }),
        },
        ListenerState::Failed { reason } => Err(WayLeftOut::ListenerNotListening {
            reason: reason.clone(),
        }),
        ListenerState::Off => Err(match not_adopted {
            Some(not_adopted) => WayLeftOut::ListenerNotListening {
                reason: not_adopted.detail(),
            },
            None => WayLeftOut::ListenerOff,
        }),
    }
}

/// Issues the Invite `/serve` would have ticked, with what its asker left out
/// left out. What can be told to fail beforehand fails before Serving is turned
/// on, so asking for what cannot be offered opens nothing; what comes to fail
/// after says what it did to Serving, as it stands.
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
    if chosen.ways.is_empty() {
        let settings = state.settings.borrow().settings.serving.clone();
        let addresses = request
            .addresses
            .then(|| addresses_cannot_be_offered(&state, &settings));
        if let None | Some(Some(_)) = addresses {
            return answer(InviteOfferAnswer::NothingToOffer {
                serving: None,
                why: addresses
                    .flatten()
                    .into_iter()
                    .chain(chosen.left_out)
                    .collect(),
            });
        }
    }
    let prepared = match turn_serving_on(&state).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let settings = state.settings.borrow().settings.serving.clone();
    let listener = state.serving.listener().borrow().clone();
    let mut left_out = Vec::new();
    let mut ways = Vec::new();
    if request.addresses {
        match addresses_offered(&state, &settings, &listener, prepared.not_adopted.as_ref()) {
            Ok(addresses) => ways = addresses,
            Err(why) => left_out.push(why),
        }
    }
    left_out.extend(chosen.left_out);
    ways.extend(chosen.ways);
    let nothing_to_ask = ways.is_empty();
    answer(
        match state
            .serving
            .issue_invite(IssueInviteRequest { ways })
            .await
        {
            Ok(issued) => InviteOfferAnswer::Issued(Box::new(OfferedInvite {
                issued,
                serving: prepared.change,
                left_out,
                not_answering: chosen.not_answering,
                expires_in_seconds: state.serving.invite_ttl().as_secs(),
            })),
            // Nothing asked of Serving itself failed, so the reason is that there
            // was nothing to ask for.
            Err(failure)
                if nothing_to_ask && failure.code == SessionErrorCode::InvalidInviteWays =>
            {
                InviteOfferAnswer::NothingToOffer {
                    serving: prepared.change,
                    why: left_out,
                }
            }
            Err(failure) => InviteOfferAnswer::Failed {
                serving: prepared.change,
                message: failure.message,
            },
        },
    )
}
