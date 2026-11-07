//! `suru relay` and `suru serve`: a thin Client over the routes `/relay` and
//! `/serve` use, for a machine with no one at its screen. Whatever would make
//! them differ from those surfaces is the Server's to decide: what an Invite
//! offers, what is turned on for it, and whether a login begins.
//!
//! What a command was asked for goes to standard output, and only that, so a
//! script can capture it. Everything else goes to standard error, and neither
//! carries colour. An Invite and its short code reach no Log (ADR-0008) and are
//! never taken as arguments. How each command exits is [`EXIT_STATUS`], which
//! `docs/relay/README.md` repeats.

use std::io::{IsTerminal as _, Write};

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use crate::{
    managed_client::{ManagedClient, ManagedClientConfig},
    protocol::{
        InviteOfferAnswer, InviteRelays, OfferInviteRequest, OfferedInvite, Relay, RelayLogin,
        RelayLoginOutcome, RelayLoginRefusal, RelayLoginStanding, RelayState, ServingChange,
        SessionError, SessionErrorCode, Way, WayLeftOut,
    },
    relay_said,
};

/// How the commands exit, as their help says.
pub const EXIT_STATUS: &str = "\
Exit status:
  0    it did as asked, or that already held
  1    it could not, saying why on standard error
  2    its command line cannot be read, an address no Relay could be at among that
  3    `relay remove` removed the Relay here, but the Relay did not answer to forget the Login
  4    `relay login` was refused: by you at the identity provider, or by the Relay
  5    `relay login` ended with no one refusing it: it expired, or the Relay stopped answering
  130  `relay login` was interrupted, which leaves the login under way at the Server";

const SUCCESS: u8 = 0;
const FAILED: u8 = 1;
const BAD_COMMAND_LINE: u8 = 2;
const REMOVAL_UNCONFIRMED: u8 = 3;
const LOGIN_REFUSED: u8 = 4;
const LOGIN_LAPSED: u8 = 5;
const INTERRUPTED: u8 = 130;

/// What turns Serving on, for a reader at a command line.
const TURNS_SERVING_ON: &str = "`suru serve invite`";

/// The Server's Relays, as `/relay` manages them.
#[derive(Debug, Subcommand)]
pub enum RelayCommand {
    /// Lists the Server's Relays and how each stands.
    List {
        /// Prints them as JSON, as `docs/relay/README.md` documents it.
        #[arg(long)]
        json: bool,
    },
    /// Adds a Relay by its address. Adding one held already is success.
    Add { address: String },
    /// Logs in at a Relay, following a login already under way, and stays until
    /// the login ends. Interrupting this leaves the login under way at the Server.
    Login { address: String },
    /// Chooses whether this machine is reachable through a Relay.
    ServeThrough {
        address: String,
        #[arg(value_enum)]
        choice: OnOff,
    },
    /// Removes a Relay, and forgets this Server's Login there. It does not ask.
    Remove { address: String },
}

/// Whether this machine is to Serve through a Relay.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

/// Serving, as `/serve` manages it.
#[derive(Debug, Subcommand)]
pub enum ServeCommand {
    /// Issues an Invite, printing it on standard output and everything else on
    /// standard error. It turns Serving on where it is off, and offers every way
    /// `/serve` would tick unless told otherwise. The Invite replaces any issued
    /// before it, so running this twice kills the first.
    #[command(after_help = EXIT_STATUS)]
    Invite(InviteArguments),
}

#[derive(Args, Debug)]
pub struct InviteArguments {
    /// Offers none of this machine's own addresses, for an Invite reached only
    /// through a Relay.
    #[arg(long)]
    no_addresses: bool,
    /// Offers no Relay.
    #[arg(long, conflicts_with = "relay")]
    no_relays: bool,
    /// Offers this Relay, and among Relays only those named; repeatable. It fails
    /// where the Server cannot offer one.
    #[arg(long, value_name = "ADDRESS")]
    relay: Vec<String>,
    /// Prints the Invite, its ways and its short code as JSON on standard output.
    #[arg(long)]
    json: bool,
}

impl InviteArguments {
    /// What the Server is asked for, or why nothing could come of asking.
    fn request(&self) -> std::result::Result<OfferInviteRequest, &'static str> {
        if self.no_addresses && self.no_relays {
            return Err("--no-addresses and --no-relays together leave an Invite no way to offer");
        }
        Ok(OfferInviteRequest {
            addresses: !self.no_addresses,
            relays: if self.no_relays {
                InviteRelays::None
            } else if self.relay.is_empty() {
                InviteRelays::Every
            } else {
                InviteRelays::Only(self.relay.clone())
            },
        })
    }
}

/// Runs `command` against the Channel's Server, starting one as a Client would,
/// answering the status to exit with.
pub async fn relay(config: ManagedClientConfig, command: RelayCommand) -> u8 {
    let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
    let Some(client) = reach_server(config, &mut err).await else {
        return FAILED;
    };
    let finished = match command {
        RelayCommand::List { json } => list(&client, json, &mut out, &mut err).await,
        RelayCommand::Add { address } => add(&client, &address, &mut err).await,
        RelayCommand::Login { address } => log_in(&client, &address, &mut out, &mut err).await,
        RelayCommand::ServeThrough { address, choice } => {
            serve_through(&client, &address, choice, &mut err).await
        }
        RelayCommand::Remove { address } => remove(&client, &address, &mut err).await,
    };
    finished.unwrap_or_else(|error| failed(&error, &mut err))
}

/// Runs `command` against the Channel's Server, as [`relay`] does.
pub async fn serve(config: ManagedClientConfig, command: ServeCommand) -> u8 {
    let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
    let ServeCommand::Invite(arguments) = command;
    // Nothing needs a Server to tell this, so none is started for it.
    let request = match arguments.request() {
        Ok(request) => request,
        Err(why) => {
            say(&mut err, &format!("error: {why}"));
            return BAD_COMMAND_LINE;
        }
    };
    let Some(client) = reach_server(config, &mut err).await else {
        return FAILED;
    };
    match client.offer_invite(request).await {
        Ok(InviteOfferAnswer::Issued(offered)) => print_invite(
            &offered,
            arguments.json,
            std::io::stderr().is_terminal(),
            &mut out,
            &mut err,
        ),
        Ok(refused) => invite_refused(&refused, &mut err),
        Err(error) => failed(&error, &mut err),
    }
}

/// What is said of what asking for an Invite did to Serving.
fn serving_change_said(change: ServingChange) -> &'static str {
    match change {
        ServingChange::TurnedOn => {
            "Serving is now on: this machine can be reached by the Servers paired with it"
        }
        ServingChange::Recovered => "The Serving listener, which had failed, is listening again",
        ServingChange::CouldNotStart => {
            "Serving is asked for now (serving.enabled), and could not start"
        }
    }
}

/// Why a way is not offered, and what to run to put it right: what `/relay` and
/// `/serve` say of it, with this command line's commands for their keys.
fn way_left_out_said(way: &WayLeftOut) -> String {
    match way {
        WayLeftOut::NoRelayAt { address, held } => {
            let holds = if held.is_empty() {
                "it holds none".to_owned()
            } else {
                format!("it holds {}", held.join(", "))
            };
            format!(
                "This Server holds no Relay at {address}: {holds}. `suru relay add {address}` \
                 adds it"
            )
        }
        WayLeftOut::NoRelayHeld => {
            "This Server holds no Relay; `suru relay add <address>` adds one".to_owned()
        }
        WayLeftOut::RelayLoginNeeded { relay } if pending(relay) => format!(
            "The Relay at {address} needs a login, and one is under way: `suru relay login \
             {address}` shows it",
            address = relay.address
        ),
        WayLeftOut::RelayLoginNeeded { relay } => format!(
            "The Relay at {address} cannot be offered: {standing}. `suru relay login {address}` \
             logs in there",
            address = relay.address,
            standing = relay_said::relay_standing(relay)
        ),
        WayLeftOut::RelayNotServedThrough { address } => format!(
            "This Server does not Serve through the Relay at {address}, so an Invite cannot \
             offer it. `suru relay serve-through {address} on` makes this machine reachable \
             through it"
        ),
        WayLeftOut::ListenerOff => "The Serving listener is off (serving.listener), so this \
                                    offers none of this machine's addresses"
            .to_owned(),
        WayLeftOut::ListenerNotListening { reason } => format!(
            "The Serving listener is not listening, so this offers none of this machine's \
             addresses: {reason}"
        ),
        WayLeftOut::ListenerLoopback { at } => format!(
            "The Serving listener listens at {at}, which only this machine can reach \
             (serving.bindAddress), so this offers none of its addresses"
        ),
        WayLeftOut::NoAddress => {
            "This machine has no address another machine could reach it at, so this offers none"
                .to_owned()
        }
        WayLeftOut::AddressesUnknown { reason } => {
            format!("This machine's addresses could not be found: {reason}")
        }
    }
}

/// Says why no Invite was issued, and what asking for one did to Serving.
fn invite_refused(refused: &InviteOfferAnswer, err: &mut dyn Write) -> u8 {
    let serving = match refused {
        InviteOfferAnswer::Issued(_) => return SUCCESS,
        InviteOfferAnswer::RelayWithheld { why } => {
            for way in why {
                say(err, &format!("Error: {}", way_left_out_said(way)));
            }
            None
        }
        InviteOfferAnswer::NothingToOffer { serving, why } => {
            say(err, "Error: An Invite has no way to offer");
            for way in why {
                say(err, &way_left_out_said(way));
            }
            *serving
        }
        InviteOfferAnswer::Failed { serving, message } => {
            say(err, &format!("Error: {message}"));
            *serving
        }
    };
    if let Some(serving) = serving {
        say(err, serving_change_said(serving));
    }
    FAILED
}

/// Says `line` on `err`. Nobody reading it is no reason to stop.
fn say(err: &mut dyn Write, line: &str) {
    let _ = writeln!(err, "{line}");
    let _ = err.flush();
}

/// Says why a command could not do as it was asked, in the Server's words where
/// they are the Server's, answering the status to exit with.
fn failed(error: &anyhow::Error, err: &mut dyn Write) -> u8 {
    match error.downcast_ref::<SessionError>() {
        Some(refusal) => {
            say(err, &format!("Error: {}", refusal.message));
            if refusal.code == SessionErrorCode::InvalidRelayAddress {
                BAD_COMMAND_LINE
            } else {
                FAILED
            }
        }
        None => {
            say(err, &format!("Error: {error:#}"));
            FAILED
        }
    }
}

/// The Channel's Server, started as a Client starts one where none runs, which
/// is said, on standard error alone. Nothing is said where it failed but why.
async fn reach_server(config: ManagedClientConfig, err: &mut dyn Write) -> Option<ManagedClient> {
    let channel = config.channel().to_owned();
    match ManagedClient::for_one_command(config).await {
        Ok((client, started)) => {
            if let Some(health) = started {
                say(
                    err,
                    &format!(
                        "Started a Suru server on the `{channel}` Channel (pid {})",
                        health.identity.pid
                    ),
                );
            }
            Some(client)
        }
        Err(error) => {
            failed(&error, err);
            None
        }
    }
}

/// A Relay as `suru relay list --json` says it: the command line's own shape,
/// which `docs/relay/README.md` documents, so that nothing renamed on the Server's
/// own API changes what a script reads. It holds nothing secret: no login's code.
#[derive(Debug, Serialize)]
struct ListedRelay<'a> {
    address: &'a str,
    state: &'static str,
    account: Option<ListedAccount<'a>>,
    unreachable_reason: Option<&'a str>,
    serve_through: bool,
    login_under_way: bool,
    pinned_in: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ListedAccount<'a> {
    provider: &'a str,
    username: &'a str,
}

#[derive(Debug, Serialize)]
struct Listing<'a> {
    serving: bool,
    relays: Vec<ListedRelay<'a>>,
}

fn pending(relay: &Relay) -> bool {
    relay
        .login
        .as_ref()
        .is_some_and(|login| !login.outcome.is_settled())
}

fn listed(relay: &Relay) -> ListedRelay<'_> {
    ListedRelay {
        address: &relay.address,
        state: match relay.state {
            RelayState::LoggedIn => "logged_in",
            RelayState::LoginNeeded => "login_needed",
            RelayState::Unreachable => "unreachable",
        },
        account: relay.account.as_ref().map(|account| ListedAccount {
            provider: &account.provider,
            username: &account.username,
        }),
        unreachable_reason: relay
            .unreachable
            .as_ref()
            .map(|unreachable| unreachable.message.as_str()),
        serve_through: relay.serve_through,
        login_under_way: pending(relay),
        pinned_in: relay.pinned_in.as_deref(),
    }
}

/// What the plain listing says of `relay`: its address, how it stands and is
/// Served through, and what pins it. A login under way is said to be, and its
/// code left to `suru relay login`, since a listing is what lands in a log.
fn listing_lines(relay: &Relay, serving: bool) -> String {
    let standing = if pending(relay) {
        format!(
            "Login under way; `suru relay login {}` shows its code",
            relay.address
        )
    } else {
        relay_said::relay_standing(relay)
    };
    let mut lines = format!("{}\n  {standing}", relay.address);
    if let Some(serving) = relay_said::serve_through_status(relay, serving) {
        lines.push_str(&format!(" · {serving}"));
    }
    if let Some(document) = &relay.pinned_in {
        lines.push_str(&format!("\n  Pinned by serving.relays in {document}"));
    }
    lines
}

async fn list(
    client: &ManagedClient,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8> {
    let serving = client.serving_standing().await?.enabled;
    let listing = client.list_relays().await?;
    if listing.relays.is_empty() && !json {
        say(
            err,
            "This Server holds no Relay; `suru relay add <address>` adds one",
        );
        return Ok(SUCCESS);
    }
    let mut printed = if json {
        serde_json::to_string(&Listing {
            serving,
            relays: listing.relays.iter().map(listed).collect(),
        })?
    } else {
        listing
            .relays
            .iter()
            .map(|relay| listing_lines(relay, serving))
            .collect::<Vec<_>>()
            .join("\n")
    };
    printed.push('\n');
    out.write_all(printed.as_bytes())?;
    out.flush()?;
    Ok(SUCCESS)
}

/// Adds a Relay. Adding one held already is success: what a script asked for
/// holds, and running one twice is normal. The Server's word on it is said.
async fn add(client: &ManagedClient, address: &str, err: &mut dyn Write) -> Result<u8> {
    match client.add_relay(address).await {
        Ok(relay) => {
            say(err, &format!("Added the Relay at {}", relay.address));
            say(
                err,
                &format!(
                    "Nothing connects there until `suru relay login {}`",
                    relay.address
                ),
            );
            Ok(SUCCESS)
        }
        Err(error) => match error.downcast_ref::<SessionError>() {
            Some(refusal) if refusal.code == SessionErrorCode::RelayAlreadyAdded => {
                say(err, &refusal.message);
                Ok(SUCCESS)
            }
            _ => Err(error),
        },
    }
}

/// What a login's reader does to finish it: one line on standard output, so a
/// reader taking the first line, as `| head -1` does, has the address and the
/// code both. Should standard output take nothing, the one running this still
/// needs it, so standard error has it.
fn show_login(login: &RelayLogin, out: &mut dyn Write, err: &mut dyn Write) {
    let line = format!(
        "Visit {} and enter the code {}",
        login.verification_uri, login.user_code
    );
    if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
        say(err, &line);
    }
}

/// The status a login that ended in `outcome` exits with.
fn login_status(outcome: &RelayLoginOutcome) -> u8 {
    match outcome {
        RelayLoginOutcome::Pending | RelayLoginOutcome::Done { .. } => SUCCESS,
        RelayLoginOutcome::Refused { reason, .. } => match reason {
            RelayLoginRefusal::Denied
            | RelayLoginRefusal::NotAdmitted
            | RelayLoginRefusal::LoginsCapReached { .. }
            | RelayLoginRefusal::Unavailable => LOGIN_REFUSED,
            RelayLoginRefusal::Expired
            | RelayLoginRefusal::Interrupted
            | RelayLoginRefusal::Unrecorded => LOGIN_LAPSED,
        },
    }
}

async fn log_in(
    client: &ManagedClient,
    address: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8> {
    let mut interrupt = Interrupt::listen();
    let lead = match client.log_in_at_relay(address, false).await {
        Ok(lead) => lead,
        Err(error) => return Ok(login_not_begun(&error, address, err)),
    };
    let address = &lead.address;
    let login = match lead.standing {
        RelayLoginStanding::Standing { relay } => {
            say(err, &relay_said::login_stands(&relay));
            return Ok(SUCCESS);
        }
        RelayLoginStanding::UnderWay { login } => {
            say(err, &format!("Following the login under way at {address}"));
            login
        }
        RelayLoginStanding::Begun { login } => login,
    };
    show_login(&login, out, err);
    say(
        err,
        "Waiting for the login to end. Interrupting this leaves it under way at the Server",
    );
    tokio::select! {
        ended = client.follow_relay_login(address) => Ok(match ended {
            Ok(login) => login_ended(address, &login, err),
            Err(error) => {
                say(err, &format!(
                    "Error: Stopped following the login at {address}: {error:#}. A login ends \
                     with its Server; `suru relay login {address}` says where it stands"
                ));
                FAILED
            }
        }),
        () = interrupt.arrives() => {
            say(err, &format!(
                "Left the login at {address} under way: it goes on at the Server, where `suru \
                 relay login {address}` and /relay find it"
            ));
            Ok(INTERRUPTED)
        }
    }
}

/// Ctrl+C, listened for from the moment this is made. It must be made before a
/// login's line is printed: whoever read the line may interrupt at once, and
/// nothing listening then would end the process by the signal instead.
struct Interrupt {
    #[cfg(unix)]
    signal: Option<tokio::signal::unix::Signal>,
    #[cfg(windows)]
    signal: Option<tokio::signal::windows::CtrlC>,
}

impl Interrupt {
    fn listen() -> Self {
        Self {
            #[cfg(unix)]
            signal: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).ok(),
            #[cfg(windows)]
            signal: tokio::signal::windows::ctrl_c().ok(),
        }
    }

    /// Resolves on Ctrl+C; never, should that not be listened for.
    async fn arrives(&mut self) {
        match &mut self.signal {
            Some(signal) => {
                signal.recv().await;
            }
            None => std::future::pending::<()>().await,
        }
    }
}

/// Says why no login began, as `/relay` says it, answering the status.
fn login_not_begun(error: &anyhow::Error, address: &str, err: &mut dyn Write) -> u8 {
    let Some(refusal) = error.downcast_ref::<SessionError>() else {
        return failed(error, err);
    };
    match refusal.code {
        SessionErrorCode::InvalidRelayAddress => failed(error, err),
        SessionErrorCode::RelayNotFound => {
            say(err, &not_held(address));
            FAILED
        }
        _ => {
            say(
                err,
                &format!(
                    "Error: Could not begin a login at {address}: {}",
                    refusal.message
                ),
            );
            FAILED
        }
    }
}

/// What is said of an address the Server holds no Relay at.
fn not_held(address: &str) -> String {
    format!("Error: This Server holds no Relay at {address}; `suru relay add {address}` adds it")
}

/// Says how a followed login ended, answering the status.
fn login_ended(address: &str, login: &RelayLogin, err: &mut dyn Write) -> u8 {
    if let Some(ended) = relay_said::login_ended(address, &login.outcome) {
        if ended.failed {
            say(err, &format!("Error: {}", ended.text));
        } else {
            say(err, &ended.text);
            say(
                err,
                &format!(
                    "Logging in opens nothing: `suru relay serve-through {address} on` makes \
                     this machine reachable through this Relay"
                ),
            );
        }
    }
    login_status(&login.outcome)
}

async fn serve_through(
    client: &ManagedClient,
    address: &str,
    choice: OnOff,
    err: &mut dyn Write,
) -> Result<u8> {
    let wanted = matches!(choice, OnOff::On);
    let relay = match client.set_relay_serve_through(address, wanted).await {
        Ok(relay) => relay,
        Err(error) => {
            return Ok(match error.downcast_ref::<SessionError>() {
                Some(refusal) if refusal.code == SessionErrorCode::RelayNotFound => {
                    say(err, &not_held(address));
                    FAILED
                }
                _ => failed(&error, err),
            });
        }
    };
    let serving = client.serving_standing().await?.enabled;
    say(
        err,
        &relay_said::serve_through_said(&relay, serving, TURNS_SERVING_ON),
    );
    if wanted {
        say(err, relay_said::REACHABLE);
    } else {
        say(
            err,
            &format!(
                "It stops waiting there, and its Login stands: `suru relay remove {}` forgets it",
                relay.address
            ),
        );
    }
    Ok(SUCCESS)
}

/// Removes a Relay. An address the Server does not hold is a failure, unlike
/// adding one it does: a mistyped one would leave the machine logged in where
/// the script took it to have left.
async fn remove(client: &ManagedClient, address: &str, err: &mut dyn Write) -> Result<u8> {
    let removal = match client.remove_relay(address).await {
        Ok(removal) => removal,
        Err(error) => {
            return Ok(match error.downcast_ref::<SessionError>() {
                Some(refusal) if refusal.code == SessionErrorCode::RelayNotFound => {
                    say(err, &not_held(address));
                    FAILED
                }
                _ => failed(&error, err),
            });
        }
    };
    if removal.acknowledged {
        say(
            err,
            &format!(
                "Removed {}, and the Relay forgot this Server's Login there",
                removal.address
            ),
        );
        Ok(SUCCESS)
    } else {
        say(
            err,
            &format!(
                "Removed {} here; the Relay did not answer, so it may still hold a Login for this \
                 Server, until its operator removes it",
                removal.address
            ),
        );
        Ok(REMOVAL_UNCONFIRMED)
    }
}

/// An Invite as `suru serve invite --json` says it: the command line's own
/// shape, as [`ListedRelay`] is.
#[derive(Debug, Serialize)]
struct PrintedInvite<'a> {
    invite: &'a str,
    ways: Vec<PrintedWay>,
    short_code: Option<String>,
}

#[derive(Debug, Serialize)]
struct PrintedWay {
    kind: &'static str,
    address: String,
}

fn printed_way(way: &Way) -> PrintedWay {
    match way {
        Way::Direct(address) => PrintedWay {
            kind: "direct",
            address: address.to_string(),
        },
        Way::Relay(address) => PrintedWay {
            kind: "relay",
            address: address.clone(),
        },
    }
}

/// Prints what the Server issued. The Invite alone goes to standard output, and
/// its short code goes to standard error only where that is a terminal: it is as
/// secret as the Invite, and what a provisioning script keeps is its standard
/// error. `--json` carries both on standard output for whoever wants them.
fn print_invite(
    offered: &OfferedInvite,
    json: bool,
    terminal: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    if let Some(serving) = offered.serving {
        say(err, serving_change_said(serving));
    }
    for way in &offered.left_out {
        say(err, &way_left_out_said(way));
    }
    for relay in &offered.not_answering {
        say(
            err,
            &format!(
                "The Relay at {} is offered but not answering now: {}",
                relay.address,
                relay_said::relay_standing(relay)
            ),
        );
    }
    let issued = &offered.issued;
    for way in &issued.ways {
        say(err, &format!("It offers {way}"));
    }
    say(
        err,
        &format!(
            "It can be redeemed once, within {} minutes, and replaces any Invite issued before it",
            offered.expires_in_seconds / 60
        ),
    );
    let short_code = issued.short_code.as_ref().map(|code| code.code.grouped());
    let mut printed = if json {
        let printed = PrintedInvite {
            invite: &issued.invite,
            ways: issued.ways.iter().map(printed_way).collect(),
            short_code: short_code.clone(),
        };
        serde_json::to_string(&printed).expect("what is printed always serializes")
    } else {
        issued.invite.0.clone()
    };
    printed.push('\n');
    if let Err(error) = out.write_all(printed.as_bytes()).and_then(|()| out.flush()) {
        // It is never said anywhere else, so it goes unread, and the next one
        // issued replaces it.
        say(
            err,
            &format!("Error: could not print the Invite on standard output: {error}"),
        );
        return FAILED;
    }
    if !json && let Some(code) = short_code {
        if terminal {
            say(
                err,
                &format!(
                    "Its short code is {code}: type it in /pair on the other machine. It gets \
                     one try"
                ),
            );
        } else {
            say(
                err,
                "It has a short code too, left out as standard error is not a terminal and may \
                 be kept in a log; --json carries it on standard output",
            );
        }
    }
    SUCCESS
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::protocol::{
        IssuedInvite, IssuedShortCode, RelayAccount, RelayUnreachable, ShortCode,
    };

    fn relay(address: &str) -> Relay {
        Relay {
            address: address.to_owned(),
            state: RelayState::LoginNeeded,
            unreachable: None,
            account: None,
            login: None,
            serve_through: false,
            login_needed_notice: None,
            pinned_in: None,
        }
    }

    fn login(outcome: RelayLoginOutcome) -> RelayLogin {
        RelayLogin {
            verification_uri: "https://example.com/device".to_owned(),
            user_code: "WDJB-MJHT".to_owned(),
            outcome,
        }
    }

    fn said(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[test]
    fn the_listing_is_the_command_lines_own_and_keeps_a_login_code_to_itself() {
        let under_way = Relay {
            login: Some(login(RelayLoginOutcome::Pending)),
            login_needed_notice: Some(uuid::Uuid::nil()),
            ..relay("https://a.example")
        };
        let standing = Relay {
            state: RelayState::Unreachable,
            unreachable: Some(RelayUnreachable {
                behind: None,
                message: "the Relay stopped answering".to_owned(),
            }),
            account: Some(RelayAccount {
                provider: "github".to_owned(),
                username: "octocat".to_owned(),
            }),
            serve_through: true,
            pinned_in: Some("~/.config/suru/suru.jsonc".to_owned()),
            ..relay("https://b.example")
        };
        let printed = serde_json::to_value(Listing {
            serving: true,
            relays: vec![listed(&under_way), listed(&standing)],
        })
        .unwrap();
        assert_eq!(
            printed,
            json!({
                "serving": true,
                "relays": [
                    {
                        "address": "https://a.example",
                        "state": "login_needed",
                        "account": null,
                        "unreachable_reason": null,
                        "serve_through": false,
                        "login_under_way": true,
                        "pinned_in": null,
                    },
                    {
                        "address": "https://b.example",
                        "state": "unreachable",
                        "account": { "provider": "github", "username": "octocat" },
                        "unreachable_reason": "the Relay stopped answering",
                        "serve_through": true,
                        "login_under_way": false,
                        "pinned_in": "~/.config/suru/suru.jsonc",
                    },
                ],
            })
        );
        assert!(!printed.to_string().contains("WDJB-MJHT"));

        let lines = listing_lines(&under_way, true);
        assert!(!lines.contains("WDJB-MJHT"), "{lines}");
        assert!(lines.contains("Login under way"), "{lines}");
        let lines = listing_lines(&standing, false);
        assert!(lines.contains("Unreachable · the Relay stopped"), "{lines}");
        assert!(
            lines.contains("Serves through once Serving is on"),
            "{lines}"
        );
        assert!(lines.contains("~/.config/suru/suru.jsonc"), "{lines}");
    }

    #[test]
    fn a_login_exits_by_how_it_ended() {
        for (reason, status) in [
            (RelayLoginRefusal::Denied, LOGIN_REFUSED),
            (RelayLoginRefusal::NotAdmitted, LOGIN_REFUSED),
            (
                RelayLoginRefusal::LoginsCapReached { limit: 3 },
                LOGIN_REFUSED,
            ),
            (RelayLoginRefusal::Unavailable, LOGIN_REFUSED),
            (RelayLoginRefusal::Expired, LOGIN_LAPSED),
            (RelayLoginRefusal::Interrupted, LOGIN_LAPSED),
            (RelayLoginRefusal::Unrecorded, LOGIN_LAPSED),
        ] {
            let outcome = RelayLoginOutcome::Refused {
                reason,
                message: "why".to_owned(),
            };
            assert_eq!(login_status(&outcome), status, "{reason:?}");
        }
        let done = RelayLoginOutcome::Done {
            account: RelayAccount {
                provider: "github".to_owned(),
                username: "octocat".to_owned(),
            },
        };
        assert_eq!(login_status(&done), SUCCESS);
    }

    fn offered(short_code: Option<ShortCode>) -> OfferedInvite {
        OfferedInvite {
            issued: IssuedInvite {
                invite: "suru-v1-secret".into(),
                ways: vec![Way::Relay("https://a.example".to_owned())],
                short_code: short_code.map(|code| IssuedShortCode { code, serial: 1 }),
            },
            serving: Some(ServingChange::TurnedOn),
            left_out: Vec::new(),
            not_answering: Vec::new(),
            expires_in_seconds: 600,
        }
    }

    fn a_short_code() -> ShortCode {
        ShortCode::parse("K7QF2MXP").expect("a code from the alphabet")
    }

    #[test]
    fn a_short_code_is_shown_to_a_terminal_and_kept_from_anything_else() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let status = print_invite(
            &offered(Some(a_short_code())),
            false,
            true,
            &mut out,
            &mut err,
        );
        assert_eq!(status, SUCCESS);
        assert_eq!(said(&out), "suru-v1-secret\n", "the Invite alone");
        assert!(said(&err).contains("K7QF-2MXP"), "{}", said(&err));
        assert!(!said(&err).contains("suru-v1-secret"));

        let (mut out, mut err) = (Vec::new(), Vec::new());
        print_invite(
            &offered(Some(a_short_code())),
            false,
            false,
            &mut out,
            &mut err,
        );
        assert_eq!(said(&out), "suru-v1-secret\n");
        assert!(!said(&err).contains("K7QF-2MXP"), "{}", said(&err));
        assert!(said(&err).contains("--json"), "{}", said(&err));
    }

    #[test]
    fn json_carries_the_invite_and_its_short_code_on_standard_output() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        print_invite(
            &offered(Some(a_short_code())),
            true,
            true,
            &mut out,
            &mut err,
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out).unwrap(),
            json!({
                "invite": "suru-v1-secret",
                "ways": [{ "kind": "relay", "address": "https://a.example" }],
                "short_code": "K7QF-2MXP",
            })
        );
        assert!(said(&out).ends_with('\n'));
        assert!(!said(&err).contains("K7QF-2MXP"));
    }

    /// A writer whose reader has gone, as a closed pipe's.
    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }

    #[test]
    fn an_invite_nobody_could_read_is_a_failure_and_goes_nowhere_else() {
        let mut err = Vec::new();
        let status = print_invite(
            &offered(Some(a_short_code())),
            false,
            false,
            &mut Closed,
            &mut err,
        );
        assert_eq!(status, FAILED);
        assert!(said(&err).contains("could not print the Invite"));
        assert!(!said(&err).contains("suru-v1-secret"));
    }

    #[test]
    fn saying_something_with_nobody_reading_is_no_failure() {
        say(&mut Closed, "nobody hears this");
    }

    #[test]
    fn a_login_whose_line_cannot_be_printed_shows_it_on_standard_error() {
        let mut err = Vec::new();
        show_login(&login(RelayLoginOutcome::Pending), &mut Closed, &mut err);
        assert_eq!(
            said(&err),
            "Visit https://example.com/device and enter the code WDJB-MJHT\n"
        );
    }

    #[test]
    fn asking_for_no_way_at_all_needs_no_server_to_tell() {
        let arguments = InviteArguments {
            no_addresses: true,
            no_relays: true,
            relay: Vec::new(),
            json: false,
        };
        assert!(arguments.request().unwrap_err().contains("no way to offer"));
    }
}
