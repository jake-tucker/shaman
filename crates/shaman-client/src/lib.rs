//! How a Client reads one Server's work and asks things of it, whatever
//! carries its requests there.
//!
//! A [`Transport`] carries a request to one Server's work and its answer
//! back: the root crate's loopback transport reaches the machine's own Server
//! turned toward an Outlook, and the [`PairingTransport`] a Remote over its
//! Pairing alone, with no Server of the reader's own. Over one, the readers follow a Session
//! ([`SessionSubscription`]), the Session catalog
//! ([`SessionCatalogSubscription`]) and a Subagent tree
//! ([`SubagentTreeSubscription`]), each reconnecting on the
//! [`RecoveryBackoff`] it is given and trying again at once when its
//! transport says to ([`TryNow`]); and [`SessionCommands`] asks exactly what
//! a Peer may ask of a Serving Server.
//!
//! Nothing here learns which Server it reaches or how: finding and keeping
//! the local Server, its own event stream and its administration stay in the
//! root crate's `managed_client`, over the same transport and the same
//! decoding of what a Server answers.

mod backoff;
mod pairing_transport;
mod remote_connection;
mod session_catalog_stream;
mod session_commands;
mod session_stream;
mod subagent_tree_stream;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
mod transport;

use shaman_protocol as protocol;

pub use backoff::{RecoveryBackoff, RecoveryStatus};
pub use pairing_transport::PairingTransport;
pub use session_catalog_stream::{
    SessionCatalogEvent, SessionCatalogOutcome, SessionCatalogSubscription, consume_session_catalog,
};
pub use session_commands::{
    NotReached, SessionCommandTimings, SessionCommands, decode_api_error, decode_api_response,
    decode_empty_api_response,
};
pub use session_stream::{
    SESSION_EVENT_CAPACITY, SessionEvent, SessionStreamError, SessionSubscription,
};
pub use subagent_tree_stream::{SubagentTreeEvent, SubagentTreeSubscription};
pub use transport::{BodyError, ResponseBody, Transport, TransportResponse, TryNow, full_body};
