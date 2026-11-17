//! The Session projection, which `shaman-protocol` holds so every Client
//! shares it, and beside it [`agent_reading`], which projects the same state
//! as text for an Agent to read.

pub(crate) mod agent_reading;

pub(crate) use shaman_protocol::session_projection::*;
