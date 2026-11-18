//! The schema of the Pairing wire, and the check that holds it to
//! compatibility (ADR-0078).
//!
//! Everything a Peer may ask of a Serving Server, the pairing ceremony
//! included, is described from its types in [`pairing_wire`] and written down
//! in `pairing-wire.json` beside this crate. CI writes it again and fails
//! where the file is stale, and judges it against the schema the change
//! started from: a removal, a rename, a retype, a newly required field or a
//! dropped default fails unless `PROTOCOL_FLOOR` rises to the change's
//! version with it, and any change to the wire fails unless
//! `PROTOCOL_VERSION` moves.
//!
//! The schema cannot see a field whose meaning changed while its shape did
//! not; review and the cross-version test catch those.

mod compare;
mod schema;

pub use compare::{Change, ChangeKind, Numbers, Verdict, judge};
pub use schema::{RouteSchema, WireSchema, written};
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
