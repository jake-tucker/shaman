# iOS phone pairing spike

Throwaway, for [#652](https://github.com/jake-tucker/shaman/issues/652): does the Rust stack a thin Client's core
will be built from pair with a real Serving Server from an iPhone, directly and through a Relay, and does a stream
cross to Swift through UniFFI? The answer is on the issue. This is what produced it. Delete it once the spec is
written.

## What is in it

- `core/` is a Rust crate, not a workspace member. The protocol types and the Session projection are symlinks to the
  root crate's own files, as in the build check (`prototypes/ios-build-check`). The client beside them is a copy of
  the shape of the redeeming side in `src/serving.rs` and `src/relays.rs`, with one difference that matters: the
  identity key signs through a signer handed in from outside (`identity.rs`), so the core never holds private key
  bytes. `lib.rs` is everything Swift sees, exported through UniFFI 0.32.
- `core/src/bin/spike-cli.rs` drives the same core from a Mac with a key kept in a file.
- `app/Spike/` is a SwiftUI console: a Secure Enclave signer, the core's stream listener wrapped as an `AsyncStream`,
  the device login in the system's authentication sheet, and a log of everything found. It runs steps from buttons
  or from `SPIKE_RUN` in the environment it is launched with.
- `scripts/build.sh` builds the core for the simulator or a phone, generates the bindings, creates the Xcode project
  with `xcrun xcodeproj`, and builds the app. Nothing generated is committed.
- `scripts/spike-server.sh` runs `shaman` against a Server of its own in a scratch directory, on its own channel and
  port, so trying the spike leaves the machine's real Server alone.
- `scripts/phone.sh` installs the app on a cabled phone, starts it with steps to run, and copies its log back.

## Running it

```sh
# On the Mac, against a Server of its own
scripts/spike-server.sh serve invite --no-relays --json > /tmp/invite.json
cd core && cargo build --bin spike-cli
target/debug/spike-cli /tmp/spike-cli redeem "$(jq -r .invite /tmp/invite.json)" spike-cli any
target/debug/spike-cli /tmp/spike-cli sessions 0

# In the simulator
scripts/build.sh sim
xcrun simctl install booted build/sim/Build/Products/Release-iphonesimulator/Spike.app
SIMCTL_CHILD_SPIKE_INVITE="$(jq -r .invite /tmp/invite.json)" \
  SIMCTL_CHILD_SPIKE_RUN="identity,redeem:192.168.,connect:192.168.,nap,stop-streams,probe" \
  xcrun simctl launch booted com.jaketucker.shaman-phone-spike

# On a phone, unlocked and cabled
scripts/build.sh device size
scripts/phone.sh install
scripts/phone.sh run "identity,redeem:direct,connect:direct" "$(shaman serve invite)"
scripts/phone.sh log
```

Steps: `identity`, `preview`, `dial:<address>`, `redeem:<direct|relay|text a way contains>`,
`connect:<text a way contains>`, `probe`, `standing[:relay]`, `login[:relay]`, `forget-login[:relay]`, `nap`,
`stop-streams`, `drop-connections`, `wait:<seconds>`, `reset`.
