//! The schema of the Pairing wire, and the check that holds it to
//! compatibility (ADR-0078).
//!
//! Everything a Peer may ask of a Serving Server, the pairing ceremony
//! included, is described from its types in [`pairing_wire`] and written down
//! in `pairing-wire.json` beside this crate: each route by what it takes and
//! answers, each header by how its value is spelled, and each type by both
//! what its reader requires and what its writer always writes. The Serving
//! side's tests hold the routes it names to exactly those the Server's route
//! table serves a Peer, and its listener carries a Peer's request on only to
//! a route that table serves, so no route escapes the schema.
//!
//! CI writes the schema again and fails where the file is stale, and judges
//! it against the schema the change started from: a removal, a rename, a
//! retype, a newly required field, a field a writer may now leave out, or a
//! dropped or changed default fails unless `PROTOCOL_FLOOR` rises to the
//! change's version with it, and any change to the wire fails unless
//! `PROTOCOL_VERSION` moves. Every enum an answer carries, of however many
//! kinds, must take a fallback for a kind its reader does not know and say
//! so by `x-unknown-kinds = "tolerated"`, or stand in a list marked
//! `x-unknown-kinds = "passed-over"` whose reader skips such an item, so a
//! new kind never breaks an older reader. Each mark is proved: a test reads a
//! kind no build knows through every marked type (see `fallbacks.rs`).
//! Headers say which side sends them and whether a reader requires them, so
//! a newly required header fails as a newly required field does.
//!
//! What it cannot see:
//!
//! - A field whose meaning changed while its shape did not; review and the
//!   cross-version test catch those.
//! - The parts of the ceremony that are not JSON: the SPAKE2 exchange a short
//!   code delivers an Invite by (ADR-0065), the Invite's token as the common
//!   name of the certificate a redeemer presents, and how an Invite is spelled
//!   as text around the payload this schema describes. Each has tests of its
//!   own.
//!
//! The wire types derive their schemas behind the `schema` feature of
//! `shaman-protocol` and `shaman-pairing`, which only this crate turns on,
//! so neither the Server nor a thin Client builds them.

mod compare;
mod schema;

pub use compare::{
    Change, ChangeKind, INVENTED_KIND, Numbers, PASSED_OVER, TOLERATED, TOLERATES_UNKNOWN_KINDS,
    Verdict, intolerance, invented_kinds, judge,
};
pub use schema::{HeaderEncoding, HeaderSent, RouteSchema, WireSchema, written};
#[cfg(test)]
mod fallbacks;
mod pairing;

pub use pairing::{SCHEMA_FILE, pairing_wire};

#[cfg(test)]
mod tests {
    use super::*;

    /// CI runs this on every platform, so a change to a type that crosses a
    /// Pairing fails until its schema is written again and committed with it.
    #[test]
    fn the_committed_schema_is_what_the_types_describe() {
        let committed = std::fs::read_to_string(SCHEMA_FILE).expect("the schema is committed");
        assert!(
            committed.replace("\r\n", "\n") == written(&pairing_wire()),
            "{SCHEMA_FILE} is stale: write it again with \
             `cargo run -p shaman-pairing-wire -- write` and commit it"
        );
    }

    #[test]
    fn the_schema_is_built_the_same_every_time() {
        assert_eq!(written(&pairing_wire()), written(&pairing_wire()));
    }

    #[test]
    fn the_schema_judged_against_itself_has_nothing_to_say() {
        let wire = pairing_wire();
        let verdict = judge(&wire, &wire).unwrap();
        assert!(verdict.changes.is_empty(), "{}", verdict.report());
        assert!(verdict.passes(), "{}", verdict.report());
    }
}
