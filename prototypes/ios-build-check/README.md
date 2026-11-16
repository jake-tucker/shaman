# iOS build check

Throwaway, for [#645](https://github.com/jake-tucker/shaman/issues/645): does what a thin Client would link from
Shaman build, link and run for iOS? The answer is on the issue. This is what produced it.

It is not a workspace member and nothing depends on it. Delete it when the question of where a thin Client's protocol
code lives is settled.

## What is in it

- `src/protocol.rs`, `src/protocol/`, `src/approval.rs`, `src/questionnaire.rs` and `src/session_projection.rs` are
  symlinks to the root crate's own files, unchanged. `src/paths.rs` stands in for the one call they make into the root
  crate, and `src/session_projection/agent_reading.rs` is empty, since the real one reaches into Provider code.
- `src/client.rs`, `src/pairing.rs`, `src/relay_http.rs` and `src/short_code.rs` do what the redeeming side of a
  Pairing does, written after `src/serving.rs`, `src/relays.rs` and `src/serving/short_invite.rs`. Those are private to
  the root crate, so this is a copy of their shape and not the code itself.
- `src/selftest.rs` runs the client against stand-ins for a Serving Server and a Relay in the same process.

Each Cargo feature is a stage that adds to the one before, so the linked size of each can be told apart: none (tokio
alone), `net`, `protocol`, `pairing`, `relay-http`, `short-code`. `selftest` adds the stand-ins and is never part of a
size figure.

## Running it

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

# Does it compile?
cargo check --target aarch64-apple-ios --features relay-http,short-code
cargo check --target aarch64-apple-ios-sim --features relay-http,short-code

# Does it run? Needs a booted simulator. The address is optional and checks the platform's trust store.
cargo build --target aarch64-apple-ios-sim --features selftest
xcrun simctl spawn booted "$PWD/target/aarch64-apple-ios-sim/debug/shaman-ios-build-check" selftest https://github.com/

# How big is it on a device?
cargo build --profile size --target aarch64-apple-ios --features relay-http
stat -f %z target/aarch64-apple-ios/size/shaman-ios-build-check
```

`Cargo.lock` is a copy of the root's, so every version is the one Shaman builds.
