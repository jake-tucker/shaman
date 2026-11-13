# What a Rust core beneath a Swift UI costs (#635)

Researched 2026-11-13 for the map in #631. The question: what it takes to ship Shaman's pairing, transport and protocol
code as a Rust library inside a SwiftUI app, and what that costs. This note lays out costs and what each precedent
found. It does not choose between a shared Rust core and native Swift.

Every claim names its source. A line that is my reading and not something a source states says **Inference**. A line I
could not check says **Unverified**. A figure I produced on this machine says **Measured here**; the method is in
[What was measured here](#what-was-measured-here).

Nothing was compiled for an iOS target. `rustup target list --installed` on this machine shows no iOS target
(`aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`), and the
ticket ruled out installing one. So "builds for iOS" below rests on each project's own statement of support, never on a
build of Shaman's code.

## Answer in brief

- **Bindings.** UniFFI 0.32.2 (September 2026) is what three of the four precedents use. It turns `async fn` into Swift
  `async` and Kotlin `suspend`, maps Rust enums with fields to Swift enums with associated values, and lets Swift
  implement a Rust trait. It has no stream type, no built-in cancellation, and only "partial support for Swift 6". Its
  own docs warn that "things may break in the future". Shaman's protocol is 240 structs and 124 enums; UniFFI does not
  read serde attributes, so each type either gains a UniFFI derive or is mirrored by a wrapper type.
- **Build.** Two Rust targets (device and Apple-silicon simulator), a static library each, one XCFramework, one Swift
  package. All four precedents ship exactly that shape. Their XCFramework zips are 45 to 222 MB, but they carry far
  more than Shaman would. A probe of Shaman's own protocol types plus a pinned-key TLS, HTTP/2 and WebSocket client
  links to 3.1 MB of arm64 code (**Measured here**, on macOS, without bindings).
- **Runtime.** tokio "guarantees support" for iOS and Android. rustls is platform independent, ring tests
  `aarch64-apple-ios` in CI, and Signal, Matrix, Bitwarden and Mozilla all run a tokio runtime inside an iOS app. The
  system's rules still apply: Apple says a suspended app's sockets may be reclaimed, and local network privacy is
  enforced "deep in the networking stack" for BSD sockets too.
- **Keys.** rustls can sign with a key it never holds, through `sign::SigningKey` and a client certificate resolver.
  The Secure Enclave signs with P-256 only, which is what Shaman's identity key is. Shaman's code holds the key as
  PKCS#8 bytes in three places, so each needs a signer in place of the bytes.
- **Android.** The same crate serves Kotlin through UniFFI (JNA and coroutines) and `cargo-ndk`. Matrix, Bitwarden and
  Mozilla do this from one core today.
- **Precedents.** All four report the same recurring costs: binary size work, a manual loop for anyone changing both
  sides, breaking changes that cross repositories, and hand-managed concurrency annotations. None reports abandoning
  the approach.
- **This repo.** The protocol types and the Session projection compile outside the root crate today with a one-line
  stand-in (**Measured here**). The pairing transport does not: it is private inside `src/serving.rs`, woven into the
  Serving side, and written against axum's body types. The client-side stream code is written against
  `reqwest::Client` on loopback, so it cannot speak a pairing as it stands.

## What was measured here

A throwaway crate outside the repository (`/tmp/shaman-core-probe`, not committed) pulled in this worktree's
`src/protocol.rs`, `src/protocol/`, `src/approval.rs`, `src/questionnaire.rs` and `src/session_projection.rs` by
symlink, unchanged, beside a small client that does what a pairing's redeeming side does: TLS 1.3 by pinned key with no
SNI, ALPN `h2`, an HTTP/2 request over hyper, and a WebSocket over tokio-tungstenite. `main` reaches all of it, so the
linker keeps it. Dependency versions came from this repo's `Cargo.lock`; the toolchain was the pinned rustc 1.98.1; the
machine an Apple M1 Max with 10 cores; the target `aarch64-apple-darwin`.

What it shows:

| Question | Result |
|---|---|
| Do the protocol types compile outside the root crate? | Yes, with `crate::paths::canonical` replaced by a one-line stand-in. They need `serde`, `serde_json`, `uuid`, `zeroize`, `blake3`, `dirs`, `dunce`. |
| Does the Session projection? | `session_projection.rs` yes, with `anyhow` added. Its child module `agent_reading` no: it reaches into `provider` and `shown_text`, so the probe left it empty. |
| Linked size, tokio runtime alone | 420,160 bytes |
| Plus rustls (ring), hyper HTTP/2 client, tokio-tungstenite | 1,663,520 bytes |
| Plus protocol types, serde and the projection | 3,119,888 bytes |
| Same, Cargo's default `release` profile | 6,268,400 bytes; 4,664,448 after `strip` |
| Clean build, about 84 crates | 57 s (size profile), 32 s (`release`) |
| Rebuild after touching the crate that holds the protocol | 47 s (size profile), 21 s (`release`) |

The size profile was `opt-level = "s"`, `lto = "fat"`, `codegen-units = 1`, `strip = true`, the same shape as Matrix's
and Bitwarden's shipping profiles (see [Precedents](#precedents)).

What it does not show: anything about an iOS target; the size of UniFFI's scaffolding and the generated Swift; `rcgen`,
`x509-parser`, `spake2` or `shaman-relay-protocol`, which a real core would also link; or the app's download size, which
is compressed. **Inference:** arm64 code size on macOS is a fair guide to the arm64 iOS slice, since both are the same
architecture linked by the same linker, but it is a guide and not a measurement.

## Bindings

### UniFFI as it stands

The current release is 0.32.2, published 22 September 2026 ([changelog][uniffi-changelog], [crates.io][uniffi-crate]).
It ships generators for Kotlin, Swift, Python and Ruby, with third-party ones for C#, Go, Kotlin Multiplatform and
React Native ([README][uniffi-readme]). Matrix pins 0.32.1 and Bitwarden pins `=0.32.0` (see
[Precedents](#precedents)).

**Async functions.** "UniFFI supports exposing async Rust functions over the FFI. It can convert a Rust `Future`/`async
fn` to and from foreign native futures (`async`/`await` in Python/Swift/Ruby, `suspend fun` in Kotlin etc.)"
([futures][uniffi-futures]). The foreign side drives the future: "the foreign bindings supply the 'executor'" and
"There's no requirement for a Rust event loop."

**Tokio.** That model does not fit code that needs a tokio reactor, which Shaman's transport does. UniFFI's internals
doc says so and names the two ways through ([async overview][uniffi-async-overview]):

> Beware that some Rust crates will present async APIs that silently start up runtimes in the background. For example,
> reqwest will start a tokio runtime.

> Can you integrate with Rust Runtimes like `tokio`? Yes. [...] Keep a reference to the runtime [and] use a bridge
> function like `tokio::Runtime::spawn` to await a future that runs in that runtime. There's also the
> `uniffi::async_runtime` macro attribute to help with this, but it's not clear if we want to continue to support it
> (#1726).

Matrix and Bitwarden both use `#[uniffi::export(async_runtime = "tokio")]` today, and 0.32.0 extended it to trait
exports ([changelog][uniffi-changelog]). So the attribute is in use and still being extended, while its own docs
question its future.

**Cancellation.** "We don't directly support cancellation in UniFFI even when the underlying platforms do. You should
build your cancellation in a separate, library specific channel" ([futures][uniffi-futures]). The generated Swift
awaits a Rust future with `withUnsafeContinuation` and no cancellation handler ([Async.swift template][uniffi-async-swift]),
so cancelling a Swift `Task` does not stop the Rust future. An open issue asks for this ([#2771][uniffi-2771]); users
there report passing a cancellation token object by hand. Matrix returns a `TaskHandle` with `cancel()` from each
subscription for the same reason.

**Streams.** UniFFI has no stream or async-sequence type. The futures page does not mention one, and the open issue on
the subject is a question about how to do it ([#2755][uniffi-2755]). Shaman's client is built on streams (the Session
stream, the catalog stream, the Subagent tree stream), so each would cross the boundary one of two ways:

- A listener: Swift implements a trait, Rust calls it for each event, and a handle cancels it. This is what Matrix and
  Signal do.
- A pull: an object with `async fn next() -> Option<Event>`, wrapped in an `AsyncStream` by hand on the Swift side.

**Inference:** either is a few dozen lines per stream on each side, written once, but it is hand-written glue that the
generator does not provide.

**Callbacks.** A Rust trait exported with `#[uniffi::export(foreign)]` can be implemented in Swift or Kotlin and handed
to Rust ([foreign traits][uniffi-foreign]). Methods may be async. Every method "should return a `Result<>` with a
compatible error type otherwise these errors will panic", and the docs warn that such traits "make it easy to create
cycles between Rust and foreign objects causing memory leaks."

**Errors.** A Rust error "must be an `enum` and implement `std::error::Error`", and becomes a thrown Swift error enum
([errors][uniffi-errors]). `anyhow::Error` cannot cross directly: "You can't yet use `anyhow` directly in your exposed
functions - you need a wrapper." A Rust panic inside a throwing function can be caught in Swift; inside a non-throwing
one it is a fatal error ([Swift overview][uniffi-swift]). Shaman's client code returns `anyhow::Result` throughout
`src/managed_client.rs`, so a binding layer needs its own error enum. `SessionError` with `SessionErrorCode` in
`src/protocol.rs` is already close to one.

**Swift 6.** "UniFFI has partial support for Swift 6. We would welcome all help improving this." "Most generated code
will conform to `Sendable`", and a Swift type that implements a foreign trait must be `Sendable` too
([Swift overview][uniffi-swift]). The tracking issue has been open since February 2025 ([#2448][uniffi-2448]).
Bitwarden compiles its generated Swift with `-suppress-warnings`; Signal, which does not use UniFFI, marks types
`@unchecked Sendable` by hand (see [Precedents](#precedents)).

**Stability.** The proc-macro docs say: "This facility is relatively new, so things may change often. However, this
remains true for all of UniFFI, so proceed with caution and the knowledge that things may break in the future"
([proc-macros][uniffi-proc]). 0.32.0 listed four breaking changes; the unreleased section lists two more
([changelog][uniffi-changelog]). Bitwarden's docs add that third-party language bindings "typically lag a few releases
behind the UniFFI core library."

### How the protocol's types would come across

Counted in this worktree across `src/protocol.rs`, `src/protocol/`, `src/approval.rs` and `src/questionnaire.rs`:

| What | Count |
|---|---|
| Public structs | 240 |
| Public enums | 124 |
| Types that reject unknown fields | 228 (156 in `protocol.rs` alone, the figure the map records) |
| Enums tagged by a field (`tag = "..."`) | 36 |
| `untagged` | 5 |
| `flatten` | 5 |
| `transparent` newtypes | 15 |
| Hand-written `Serialize` or `Deserialize` impls | 24 |
| Uses of `Uuid` | 23 |
| Uses of `PathBuf` | 42 |
| `usize` fields | 9 |

UniFFI does not use serde. It moves values across the boundary with its own encoding, so the serde attributes above
neither help nor hinder it. What it needs is that every type be one it knows:

- **Enums with fields are supported**, including struct-like variants and recursion through `Box`
  ([enumerations][uniffi-enums]). A serde enum such as `Activity` (`#[serde(tag = "kind")]`) becomes a Swift enum with
  associated values. **Inference:** this is the part of the protocol UniFFI maps most naturally.
- **`Uuid`, `PathBuf` and `serde_json::Value` are not built in.** The built-in table lists integers up to 64 bits,
  floats, `String`, `Vec<u8>`, `SystemTime`, `Duration`, `Option`, `Vec` and `HashMap` ([built-in types][uniffi-builtin]).
  Anything else goes through `custom_type!`, which names a bridge type such as `String` ([custom types][uniffi-custom]).
  A type from another crate needs the "remote" form of that macro.
- **`usize` is not in the built-in table.** The nine fields would be widened to `u64` at the boundary.
- **Newtypes** such as the id types use `custom_newtype!`.
- **Swift `Codable`** conformance for generated types is one switch, `generate_codable_conformance`
  ([Swift configuration][uniffi-swift-config]).

There are three ways to arrange this, and the precedents show two of them:

1. **Derive UniFFI on the protocol types themselves**, behind a Cargo feature so the desktop binary does not pay for
   it. Every one of the 364 types gains an attribute, and the handful of unsupported field types gain custom-type
   declarations. **Inference:** mechanical, and it keeps one definition of each type.
2. **Mirror them in a separate binding crate.** Matrix does this: its FFI crate is "mainly small Rust wrappers around
   the actual Rust SDK" ([Matrix bindings guide][matrix-contrib]). It keeps the core free of UniFFI and lets the Swift
   API differ from the wire, at the price of a second definition of every type that crosses, with conversions.
3. **Expose less.** Hand Swift a view model (rows for the Session list, items for a Transcript) and keep the wire types
   inside Rust. Fewer types cross, but the mapping is written by hand in Rust.

For scale, the generated Swift is large in both UniFFI precedents: Matrix's `matrix_sdk_ffi.swift` is 2.1 MB of source,
and Bitwarden's three largest generated files are 652 KB, 352 KB and 292 KB (sizes from each components repository; see
[Precedents](#precedents)).

A fourth arrangement keeps the types out of the binding altogether: Rust carries the transport and hands Swift the JSON
bytes, and Swift decodes them with `Codable` types of its own. That is the native-Swift cost for the protocol, paid in
addition to the binding. 1Password's typeshare generates Swift `Codable` types from Rust serde types
([typeshare][typeshare]); its documented example uses `#[serde(tag = "type", content = "content")]`, where Shaman's
enums use a tag alone. **Unverified:** whether typeshare accepts enums tagged by a field alone.

### Alternatives to UniFFI

| Tool | What it is | State | Bearing on this question |
|---|---|---|---|
| swift-bridge | Rust and Swift only, both directions, from a `#[swift_bridge::bridge]` module | 0.1.59, January 2026. Its book "is a work-in-progress with many chapter either sparse or empty". `Box<dyn Fn>`, `Arc<T>`, `&[T]` are "Not yet implemented" ([README][swift-bridge], [book][swift-bridge-book]) | Async functions are supported. No Kotlin, so Android would need a second binding layer. |
| cbindgen by hand | "creates C/C++11 headers for Rust libraries which expose a public C API" ([README][cbindgen]) | 0.29.4, mature | Gives a header and nothing else: the C API, the Swift wrappers, async and memory rules are all hand-written. Signal took this road with its own macro layer on top. |
| BoltFFI | A newer multi-language generator | Created December 2025; 0.32.1, October 2026. Its README claims "Full support" for Swift and Kotlin, and its docs describe a stream type that becomes Swift `AsyncStream` and Kotlin `Flow`, with a ring buffer that drops events when full ([README][boltffi], [streaming][boltffi-streaming]) | The one tool found with streams built in. Under a year old, and none of the four precedents uses it. Its claims are its own; nothing here tested them. |

## Build

**Targets.** `aarch64-apple-ios` (device), `aarch64-apple-ios-sim` (simulator on Apple silicon) and `x86_64-apple-ios`
(simulator on Intel) are tier 2 without host tools, installed with `rustup target add`
([rustc platform support][rust-ios]). They "require the corresponding iOS SDK (`iPhoneOS.sdk` or
`iPhoneSimulator.sdk`), as provided by Xcode." The deployment target follows `IPHONEOS_DEPLOYMENT_TARGET`; the minimum
is iOS 10.0. A first cut built on one Apple-silicon Mac needs the first two targets only (**Inference**).

**The artifact.** Apple's own guidance for code built outside Xcode is a static library per platform, combined by
`xcodebuild -create-xcframework -library ... -headers ...` ([creating an XCFramework][apple-xcframework]). Two rules
from that page bear on scripts copied from older projects: "Avoid using tools such as `lipo` to combine architecture
slices built for iOS and iOS Simulator into a single binary", and "Avoid wrapping a static library in a `.framework`
bundle". `lipo` is still what joins the two simulator architectures when both are built.

**Swift package.** A Swift package can hold a binary target that points at a zip by URL with a checksum, or at an
`.xcframework` by path inside the package ([distributing binary frameworks][apple-binary-swiftpm]). All four precedents
publish the URL form from CI. **Inference:** a first cut for one developer can use the path form, with the
XCFramework and the generated Swift produced by a script before `xcodebuild` runs.

**Generating the bindings.** `uniffi-bindgen-swift` reads the built library and writes Swift sources, headers and "a
Xcframework-compatible modulemap" ([uniffi-bindgen-swift][uniffi-bindgen-swift]). It "always inputs a library path and
runs in 'library mode'", so the proc-macro API needs no UDL file.

**Xcode build phases.** UniFFI documents running the generator from an Xcode build rule, but stops short of the Rust
build itself: "Sorry, configuring Xcode to compile the Rust crate into a staticlib is beyond the scope of this
document" ([Xcode integration][uniffi-xcode]). None of the four precedents builds Rust from inside Xcode; each runs a
script or an xtask first.

**Helpers.** `cargo-swift` (0.11.1, May 2026) packages a UniFFI crate as a Swift package in one command, but "does not
detect the UniFFI version of your project automatically", so its version must be matched to UniFFI's by hand
([cargo-swift][cargo-swift]). Matrix, Bitwarden, Mozilla and Signal each wrote their own script instead.

**Build time.** Documented or observed figures:

| Source | Figure | Kind |
|---|---|---|
| This probe, M1 Max | 57 s clean and 47 s after an edit with fat LTO; 32 s and 21 s without | **Measured here**, host target, no bindings |
| Matrix | "recompiling them at `opt-level = 3` roughly doubles the time of an incremental binding build", so own crates build at `opt-level = 1` for development | Documented ([Cargo.toml][matrix-cargo]) |
| Matrix, Element's two-slice release | 31 to 39 minutes in CI; five-slice release 69 minutes | Observed from workflow runs, 10 to 12 November 2026 |
| Bitwarden `build-swift.yml`, cached | 8 to 16 minutes, one run at 48 | Observed from workflow runs, 11 to 13 November 2026 |
| Signal "Release - iOS" | 13 to 16 minutes on `macos-15-xlarge` | Observed from workflow runs, 22 October to 10 November 2026 |
| Mozilla first-time setup | "The initial setup is likely to take a number of hours to complete." | Documented ([building][moz-building]) |

Those CI figures are for cores many times the size of what Shaman would ship. **Inference:** the cost that matters day
to day is the edit loop, where a change to Rust means a Rust rebuild, a bindings regeneration and a Swift rebuild before
anything runs, against a Swift rebuild alone for a native client.

**CI.** An iOS static library needs the iOS SDK, so it needs a macOS runner. This repo's `build.yml` already runs a
`macos-15` job for `aarch64-apple-darwin`. The research for #636 found GitHub's standard macOS runners free for a
public repository.

**Binary size.** The release assets, read from each project's release API or download headers:

| Project | Asset | Size | Slices |
|---|---|---|---|
| Matrix | `MatrixSDKFFI.xcframework.zip`, 26.11.11 | 128.2 MB | iOS, two simulator, two macOS |
| Matrix, same asset six weeks earlier | 26.10.02 | 279.6 MB | same |
| Element's fork | 26.11.12 | 50.5 MB | iOS and arm64 simulator |
| Bitwarden | `BitwardenFFI-4.0.0-6238472.xcframework.zip` | 195.8 MB | iOS and two simulator |
| Mozilla | `MozillaRustComponents.xcframework.zip`, nightly | 221.7 MB | iOS and two simulator |
| Mozilla, Focus subset | `FocusRustComponents.xcframework.zip` | 44.7 MB | same |
| Signal | `libsignal-client-ios-build-v0.107.0.xcframework.zip` | 142.3 MB | iOS and two simulator |

A zip of static libraries is not what reaches a phone: it holds every slice, unstripped object code and symbols, and
the linker discards what the app does not call. The only precedent figure for linked size is Matrix's: LTO "shrank the
framework linked into Element X iOS from 174 MB to 62 MB", and `opt-level = "s"` "measured ~33% smaller linked code"
([Matrix bindings guide][matrix-contrib]). Signal tracks one linked artifact per release, its stripped arm64 Android
library: 9.5 MB at v0.106.2 ([code_size.json][signal-code-size]).

Against those, Shaman's probe linked to 3.1 MB with a size profile (**Measured here**). That is the right order of
magnitude for a core that carries no database, no end-to-end encryption library and no bundled TLS library in C, and it
leaves out the binding scaffolding.

**Build settings the precedents found necessary.** From Matrix's build script and guide
([swift.rs][matrix-swift-xtask], [guide][matrix-contrib]):

- "`cargo build` would also produce the `lib` crate type, which silently disables the profile's LTO", so the static
  library is built with `cargo rustc --crate-type staticlib`.
- "Cargo's `strip` doesn't reach a static library either, since nothing links it."
- A debug build at `opt-level = 0` crashed on device, so development builds use `opt-level = 1`
  ([matrix-rust-sdk#4009][matrix-4009]).

Bitwarden leaves stripping off on purpose: "Stripping the binary reduces the size by ~30%, but the stacktraces won't be
usable anymore" ([Cargo.toml][bw-cargo]).

## Runtime

### Whether the stack builds and runs on iOS

| Piece | What its own project says | Source |
|---|---|---|
| tokio 1.54 | "Tokio currently guarantees support for the following platforms": Linux, Windows, Android (API level 21), macOS, iOS, FreeBSD | [tokio docs][tokio] |
| rustls 0.23 | "While Rustls itself is platform independent, by default it uses aws-lc-rs", with ring as the alternative. Shaman already builds rustls with `ring` and no default features. | [rustls docs][rustls] |
| ring 0.17 | Builds and tests `aarch64-apple-ios` in its CI matrix | [ring ci.yml][ring-ci] |
| rustls-platform-verifier | iOS: "iOS platform roots and keychain certificates", verified by "iOS `Security.framework`". Shaman uses it for an HTTPS relay only; a pairing pins a key and consults no trust store. | [README][rpv] |
| hyper, h2, tokio-tungstenite | No platform statement of their own. **Inference:** they do I/O only through tokio's traits, so they go where tokio goes. | |

The precedents confirm it in production. Signal's `libsignal-net` runs tokio, tokio-tungstenite and hyper in its iOS
app; Matrix runs reqwest over rustls; Bitwarden runs reqwest over rustls with ring and the platform verifier; Mozilla's
iOS HTTP backend "forwards requests to the Rust `reqwest` library" (see [Precedents](#precedents)).

**Unverified:** that Shaman's own transport compiles for iOS. Nothing here was built for that target. What `cargo tree
--target aarch64-apple-ios` does show is that dependency resolution succeeds for both the root crate (338 packages) and
`shaman-relay-protocol` (88), which says nothing about whether the code compiles.

**Runtime shape.** Tokio "will by default spawn one [worker thread] for each CPU core" ([tokio docs][tokio]). The
precedents size it down where memory is tight: Matrix offers "a lightweight tokio runtime, for processes that have
memory limitations (like the NSE process on iOS)" with at most four workers ([platform/mod.rs][matrix-platform]);
Mozilla's iOS backend creates "a multi-threaded runtime, with 1 worker thread"; Signal uses the default multi-threaded
runtime.

### What iOS does to a running core

**Suspension.** "The system suspends your app shortly after the user moves it to the background"
([TN3151][tn3151]). Apple's fullest description of what follows is an archived technote written for iOS 4, so its
detail may have drifted, but nothing found supersedes it ([TN2277][tn2277]):

> When the app is suspended, no code within the app's process executes. [...] while the app is suspended the system may
> choose to reclaim resources out from underneath a network socket used by the app, thereby closing the network
> connection represented by that socket.

> if your app gets suspended, the socket's resources might get reclaimed by the kernel, after which all networking
> operations on the socket will fail. The only thing you can do with the socket at this point is to close it.

On resume "the actual error returned by a socket's whose resources have been reclaimed is purposely not specified",
though "in many cases the error will be `EBADF`".

What that means for a Rust core, as **Inference** from those statements and the code:

- The tokio threads stop with the rest of the process and resume with it. Nothing in Rust is told.
- A pairing's connection may come back dead. Shaman's pairing client already treats a failed connection as something
  to redial (`SharedConnection`, `make_connection` and the direct-way retry in `src/serving.rs`), and the client's
  streams already reconnect with backoff (`src/managed_client/recovery.rs`). A core would reuse both; a native client
  would write its own.
- Timers do not run true across a sleep. Rust's `Instant` on Darwin reads `CLOCK_UPTIME_RAW`
  ([std docs][rust-instant]), which "does not increment while the system is asleep" (`clock_gettime(3)` on this Mac),
  and the standard library says it is "not specified whether system suspends count as elapsed time or not". Keepalive
  intervals and timeouts therefore measure something other than wall time around a suspension.
- Each reconnection is a new TLS handshake, so it signs with the identity key again. See [Keys](#keys).

The one report found tying suspension to a Rust core is against Matrix: iOS killed an app with `0xdead10cc` because
"the Rust SDK's tokio blocking thread pool continues holding SQLite file locks" at suspension
([matrix-rust-sdk#6366][matrix-6366], open, filed by a third-party user in March 2026, no maintainer reply). That is a
database problem. **Inference:** a core with no database does not have it, but it shows the kind of fault that only
appears on a device.

**Sockets and Apple's networking rules.** A Rust core dials with BSD sockets. Apple's position
([TN3151][tn3151]):

> Network framework is by far the best choice. BSD Sockets is an acceptable choice if you have compatibility
> constraints, for example: When writing cross-platform code. When using an existing library that's based on BSD
> Sockets.

The limitations it lists, and how each lands on Shaman:

| Apple's point | Bearing |
|---|---|
| No connect-by-name, so no Happy Eyeballs and "your program will still be incompatible with VPN On Demand" | A direct way is an IP address and port, so there is no name to resolve. A VPN that is connected on demand will not be brought up by a socket dial. |
| "BSD Sockets does not support TLS directly [...] add your own TLS implementation" | rustls is that implementation. |
| "Make sure your TLS implementation uses Apple's trust evaluation infrastructure" | Met for a relay over HTTPS by `rustls-platform-verifier`. Does not apply to a pairing, which trusts a pinned key and no certificate authority. |
| Use `SCNetworkReachabilityCreateWithAddressPair` "to monitor its viability. Without this, your program won't notice that a connection is stuck" | tokio does none of this. Shaman's HTTP/2 keepalive is what notices a stuck connection. |

Whether Apple's own stack can carry a pairing at all is #634's question, not this one's.

**Local network privacy.** The check applies to a Rust core exactly as to Swift: "The system implements these TCP and
UDP checks deep in the networking stack, and thus they apply to all networking APIs. This includes Network framework,
BSD Sockets" ([TN3179][tn3179]). Three consequences for a core that dials a direct way on a home network:

- The app's `Info.plist` needs `NSLocalNetworkUsageDescription`.
- The first dial can fail before the user answers: the system "may deny the operation immediately, before the user has
  responded to the alert", and for APIs that cannot wait for connectivity Apple says to "add appropriate retry logic".
- "The simulator doesn't support local network privacy. Test your local network privacy behavior on a real device."

## Keys

**What the Secure Enclave offers.** From Apple ([protecting keys with the Secure Enclave][apple-se]): it "works only
with NIST P-256 elliptic curve keys", usable "for creating and verifying cryptographic signatures, or for elliptic
curve Diffie-Hellman key exchange"; it "can't encode preexisting keys", and there is no "mechanism to transfer
plain-text key data into or out of the Secure Enclave"; it "requires hardware support". Signing is the ordinary
`SecKeyCreateSignature` call with an ECDSA algorithm ([signing and verifying][apple-signing]).

**What Shaman's key is.** A Server's identity key is made by `KeyPair::generate()` in `src/serving/identity.rs`, which
is ECDSA on P-256, and a relay verifies its proof with `ECDSA_P256_SHA256_ASN1`
(`crates/shaman-relay-protocol/src/proof.rs`). So the algorithm is one the Secure Enclave can perform. **Inference:**
the Secure Enclave's X9.62 signature is the same DER encoding that TLS 1.3's `ecdsa_secp256r1_sha256` and ring's
`ASN1` verifier expect. That is the standard encoding on both sides, but no handshake was run here to prove it.

**Whether rustls can sign with it.** Yes, by design. rustls's how-to chapter is about exactly this case, keys that
live elsewhere ([rustls how-to][rustls-howto]): "The main trait you must implement is `sign::SigningKey`", whose
`choose_scheme` returns a `sign::Signer` that performs the signature. The chapter says the API "is currently designed
for keys with low latency access", so signing is a blocking call, and points to `rustls-cng` as a complete example for
Windows keys. On the client side the key reaches a connection through a resolver:
`with_client_cert_resolver(Arc<dyn ResolvesClientCert>)`, whose `resolve` returns an `Arc<CertifiedKey>` holding the
certificate chain and the `SigningKey` ([ConfigBuilder][rustls-builder], [ResolvesClientCert][rustls-resolver]).

**Unverified:** that no published crate already implements a Secure Enclave `SigningKey`. A search found only
`rustls-cng`.

**Where the signing code could live.** Two places, and they differ in what Android reuses:

- In Rust, through the `security-framework` crate, which Shaman already uses on macOS at 3.7.0. It describes itself as
  "Security.framework bindings for macOS and iOS", generates a key with `Token::SecureEnclave` ("such that the private
  key is not extractable"), and signs with `SecKey::create_signature`
  ([Token][sf-token], [SecKey][sf-seckey]).
- In Swift, as a UniFFI foreign trait with one method that signs, implemented with CryptoKit or the Security
  framework. The same trait would be implemented in Kotlin over the Android Keystore.

**What would have to change in this repo.** The private key is held as exportable PKCS#8 bytes in
`IdentityMaterial { private_key: Vec<u8>, .. }`, and the `IdentityStore` trait keeps and returns bytes (`put`, `get`,
`delete` in `src/serving/identity_store.rs`). ADR-0050 chose that: the key is kept in the platform credential store and
"moves as it is". Three uses read the bytes, and each would take a signer instead:

| Use | Where | What replaces the bytes |
|---|---|---|
| TLS client authentication | `with_client_auth_cert(.., PrivateKeyDer::Pkcs8(..))`, `src/serving.rs` line 5975 | A `ResolvesClientCert` holding a `SigningKey` |
| Minting certificates: the identity certificate, and at each redemption one whose common name is `shaman-invite-{token}` | `enrollment_certificate`, `src/serving.rs` line 5995; `KeptKey::get`, `src/serving/identity.rs` | rcgen's `SigningKey` trait (`fn sign(&self, msg: &[u8])`, over `PublicKeyData`), which `self_signed(&impl SigningKey)` accepts ([rcgen][rcgen-signingkey]) |
| Proving the key to a relay | `IdentityKey::sign`, `src/serving/identity.rs` line 161 | The same signer; the call is already `rcgen::SigningKey::sign` |

**Inference:** all three are "sign this message with P-256 and SHA-256", so one signer type serves them, and the
desktop Server could keep its bytes behind the same type.

**Costs that come with a non-exportable key:**

- Access control decides when it can sign. Apple's example uses `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, which
  it calls "generally preferred unless your app operates in the background". A flag such as `biometryAny` makes the
  key "available only when the system can authenticate the user". **Inference:** since every reconnection signs, a
  biometric flag would prompt at every reconnection.
- The key cannot be backed up or moved to a new phone. A new phone is a new identity and pairs again.
- **Unverified:** whether the Simulator can create a Secure Enclave key. Apple's page says only that it "requires
  hardware support"; CryptoKit exposes `SecureEnclave.isAvailable` to ask. A simulator build would likely need a
  software key behind the same signer.

## Android

**Bindings.** UniFFI's Kotlin generator is first-party. It "relies on JNA for the ability to call native methods. JNA
5.12.0 or greater is required", and on kotlinx coroutines 1.6 or later for async ([Kotlin and Gradle][uniffi-gradle]).
There is no Gradle plugin: "We'd like to make a gradle plugin for that, but until then you can add to your
`build.gradle` the following". The docs also advise attaching tokio's worker threads to the JVM when Rust calls back
into Kotlin, because otherwise "JNA has to attach and detach a Java thread into the native thread in every function
call, which is a heavy operation." An experimental JNI generator is in UniFFI's unreleased changes
([changelog][uniffi-changelog]).

JNA has cost the precedents something. Matrix's changelog records that "with JNA direct mapping enabled they result in
invalid checks in ARM 32bit devices". Bitwarden's Android support code says "Ideally we would use JNI to get a
reference to the JavaVM, but that's not possible since UniFFI uses JNA". Signal uses JNI directly through its own
bridge.

**Build.** `cargo-ndk` configures the NDK toolchain and writes `.so` files in the `jniLibs` layout, for example
`cargo ndk -t armeabi-v7a -t arm64-v8a -o ./jniLibs build --release` ([cargo-ndk][cargo-ndk]). Matrix builds three ABIs
this way into an AAR; Bitwarden uses `cross` for four; Mozilla ships one "megazord" AAR.

**Runtime.** tokio guarantees Android from API level 21 ([tokio docs][tokio]). `rustls-platform-verifier` needs more on
Android than on iOS: "A small Kotlin component must be included in your app's build", a Proguard keep rule, and an init
call before any networking ([README][rpv]). Matrix found this the hard way: its Android platform code exists "for
fixing the `rustls` integration on Android, which was broken". Again this touches Shaman's relay path only.

**Keys.** The Android Keystore gives the same property as the Secure Enclave: "Key material never enters the
application process", and StrongBox supports "ECDSA, ECDH P-256" ([Android Keystore][android-keystore]). The API is
Java, signing with `Signature.getInstance("SHA256withECDSA")`. **Inference:** Rust cannot reach it without JNI, so on
Android the signer is most naturally a foreign trait implemented in Kotlin, which argues for the trait form on iOS too
if one shape is wanted for both.

**What is shared and what is not.** **Inference:** the core crate, the binding crate and its exported API are shared
whole. Not shared: the build script and packaging, the key signer, anything that reads the platform (hostname,
directories), and every line of UI. Sizes for scale: Matrix's Android AAR is 93.4 MB with every ABI; Signal's stripped
arm64 library is 9.5 MB.

## Precedents

State of each repository on 2026-11-13. Sizes and CI durations were read from release APIs, download headers and
workflow runs. Statements in quotation marks are the project's own.

### Matrix Rust SDK and Element X

- **Binding.** UniFFI 0.32.1 by proc-macros: "Attributes are preferred, where applicable"
  ([bindings guide][matrix-contrib]). The FFI crate is "mainly small Rust wrappers around the actual Rust SDK".
- **What Rust owns.** HTTP (reqwest with HTTP/2), TLS (rustls with aws-lc-rs), storage (bundled SQLite), encryption and
  sync ([matrix-sdk-ffi Cargo.toml][matrix-ffi-cargo]). Support for `native-tls` was removed; rustls is "the only
  supported" backend.
- **Async.** A macro rewrites exports to `async_runtime = "tokio"`. Event streams are listener traits that return a
  `TaskHandle`. Element "ended up contributing cancellable async bindings to uniffi"
  ([matrix.org, September 2023][matrix-blog]).
- **Build.** `cargo xtask swift build-framework` builds a static library per target, runs `lipo` on the simulator
  slices, generates bindings and calls `xcodebuild -create-xcframework` ([apple README][matrix-apple]). A separate
  components repository holds the generated Swift and a `binaryTarget`. Element keeps a fork that "only supports iOS
  and the iOS simulator".
- **Size work.** The profile is `opt-level = "s"`, `lto = true`, `strip = "debuginfo"`. It was added in July 2026 after
  an App Store warning that the binary "exceeds the cellular network download size limit"
  ([matrix-rust-sdk#6714][matrix-6714]). The published zip fell from 279.6 MB to 128.2 MB between October and
  November 2026.
- **Pain they record.** Debug builds crashed on device at `opt-level = 0`. LTO was silently skipped when Cargo built two
  crate types. The bundled SQLite clashed with the system's under the iOS 27 SDK, "so the bundled copy is silently
  dropped and the app crashes at launch on older systems", fixed by localising symbols with `ld -r`
  ([swift.rs][matrix-swift-xtask]). Element could not mock Rust objects in tests: "they're not using protocols
  throughout and because they're not open" ([element-x-ios#2266][element-2266]). A second UniFFI library in the same
  app was packaged oddly as "a workaround for duplicate uniffi symbols when embedding both", which later broke under
  Xcode 15.3 ([element-x-ios#2532][element-2532]). A notification extension ran out of memory inside a Rust call
  ([element-x-ios#1923][element-1923], a comment by an Element developer).
- **Developer loop.** `swift run tools build-sdk` clones the SDK, builds every architecture and points the app at the
  result ([Element X contributing guide][element-contrib]).

### Bitwarden

- **Binding.** UniFFI `=0.32.0`, proc-macros only, async on tokio ([bitwarden-uniffi][bw-uniffi]).
- **What Rust owns.** The stated aim is that "everything except presentational logic belongs in the SDK"
  ([SDK architecture][bw-arch]). The iOS architecture page still describes a Swift networking layer "on top of
  `URLSession`" and stores in Core Data and the Keychain ([iOS architecture][bw-ios-arch]). **Inference:** ownership is
  split or in migration; the app hands token providers and repositories to the SDK through UniFFI traits.
- **TLS.** rustls with ring and `rustls-platform-verifier`.
- **Build.** A shell script builds three targets, joins the simulator slices and creates the XCFramework. A separate
  `sdk-swift` repository is tagged per SDK commit.
- **Pain they record.** Breaking changes cross repositories: "Once an SDK breaking change lands on `main`, the client
  auto-PR fails to build — blocking every other team's SDK-update PR from merging to client `main` until the
  client-side fix ships" ([sdk-internal README][bw-readme]). The generated Swift is compiled with
  `-suppress-warnings`. Language bindings "typically lag a few releases behind the UniFFI core library"
  ([language bindings][bw-bindings]).
- **Developer loop.** `LOCAL_SDK=true ./Scripts/bootstrap.sh`.

### Mozilla application-services

- **Binding.** UniFFI, a mix of UDL files and proc-macros, pinned to a git revision ([Cargo.toml][moz-cargo]). UniFFI
  began here.
- **What Rust owns.** Storage and sync for places, logins and more, in one static library, the "megazord", chosen for
  "reduced code size thanks to distributing a single copy of the rust stdlib, low-level dependencies, etc."
  ([megazords][moz-megazords]). HTTP normally goes back out to the app: "Normally this means bridging to the
  application HTTP library." iOS is the exception: it "forwards requests to the Rust `reqwest` library"
  ([viaduct README][moz-viaduct]), built with `native-tls` and not rustls.
- **Async.** Rust functions were blocking and each app wrapped them. An ADR accepted on 14 October 2026 moves the
  wrapping into Rust because "this has been a steady source of issues on the iOS", aiming at "a simple contract: Rust
  calls never block the current thread" ([ADR-0010][moz-adr10]).
- **Build.** A script builds three targets and assembles the XCFramework by hand, which they note "does risk diverging
  from the expected format" ([ios-rust README][moz-ios-readme]). The separate `rust-components-swift` repository is
  archived; firefox-ios now takes the artifact directly.
- **Pain they record.** The local loop for a change on both sides is manual: build the XCFramework, repoint
  `Package.swift`, copy generated Swift, reset package caches ([how-to][moz-local]). The earlier Carthage setup was
  "not compatible with building on new M1 Apple Silicon machines" ([ADR-0003][moz-adr3]).

### Signal libsignal

- **Binding.** Not UniFFI. Signal's own `#[bridge_fn]` and `#[bridge_io]` macros "emit `#[cfg]`-guarded entry point
  for each supported language", handling "conversion of input arguments, panic trapping, and spawning of tasks for
  `async` functions" ([bridge README][signal-bridge]). One definition yields a C API for Swift, JNI for Java and a Node
  module. Their stated aim is narrower than UniFFI's: "It is explicitly *not* a goal for this layer to generate perfect
  C, Java, or TypeScript APIs." Idiomatic Swift is hand-written on top.
- **What Rust owns.** Networking for chat and other services, "with the possibility of eventually handling every
  connection to a server run by Signal" ([net README][signal-net]). Chat is a pair of persistent WebSockets over
  tokio-tungstenite. Storage stays in the apps (**Inference**, from the absence of a database crate in the bridge).
- **Async.** An async bridge function takes the runtime as an argument and completes through a C callback, which the
  Swift side wraps in a `CheckedContinuation` with cancellation ids
  ([TokioAsyncContext.swift][signal-tokio-swift]). Server pushes arrive through listener protocols.
- **TLS.** BoringSSL through Signal's fork, not rustls. System trust goes through `rustls-platform-verifier`, called
  asynchronously because "on Android we have observed it doing network activity".
- **Build.** A prebuilt XCFramework for the Rust part and Swift built from source, through SwiftPM since a commit of
  27 October 2026 ("swift: Add support for Swift Package Manager"); the CocoaPods podspec is no longer in the
  repository. The two parts must match exactly or "linking will fail with an undefined `signal_ffi_version_X_Y_Z`
  symbol" ([swift README][signal-swift]).
- **Pain they record.** Panic safety is asserted by hand ("Not ideal! tokio doesn't promise that a oneshot::Receiver is
  in fact panic-safe"). The promise bridging gives up Swift's checks ("we're sidestepping all of Swift's type safety!").
  Types are `@unchecked Sendable` with the comment "We'd like to remove the `@unchecked` above but Swift 5.10 still
  complains". CI warns when a change adds more than 100,000 bytes to the Android library.

### Across the four

| | Matrix and Element X | Bitwarden | Mozilla | Signal |
|---|---|---|---|---|
| Binding | UniFFI 0.32.1 | UniFFI 0.32.0 | UniFFI, git revision | Own macros over C, JNI, Node |
| Networking in Rust on iOS | Yes | Partly | Yes, though not on other platforms | Yes |
| TLS on iOS | rustls, aws-lc-rs | rustls, ring | native-tls | BoringSSL |
| Streams to Swift | Listener plus task handle | Host-implemented traits | Mostly blocking calls, moving to async | Listener protocols |
| Shipped as | XCFramework, SwiftPM binary target | same | same | same, plus Swift source |
| Size profile | `s`, LTO, strip debuginfo | `z`, LTO, one codegen unit, no strip | `s`, thin LTO | fat LTO, one codegen unit |
| Android | cargo-ndk, JNA | cross, JNA | megazord AAR, JNA | JNI |

What the four have in common, as **Inference** from the above: each built its own build script; each ships a binary
through a Swift package; each keeps a documented, manual procedure for working on both sides at once; and each has
done deliberate size work. None is a project of one developer. The nearest in scale of what crosses the boundary is
Signal's networking layer, and Signal chose to write its own binding.

## This repo

### What stands in the way today

The root crate is one library that holds everything. `src/lib.rs` declares the TUI, storage, the Provider runtimes, the
Server and the protocol as modules of one crate, and `Cargo.toml` gives it one dependency list. A crate that depends on
`shaman` therefore links all of it. For iOS that list includes:

| Dependency | Why it does not belong in an iOS client |
|---|---|
| `ratatui`, `ratatui-image`, `crossterm`, `termina`, `syntect`, `two-face` | The terminal UI |
| `arboard` | Has backends for Windows, macOS and other Unix only; iOS would select the X11 and Wayland one (**Inference** from its `cfg` gates) |
| `diesel`, `libsqlite3-sys` (bundled) | Server storage. Bundled SQLite does build for iOS (Matrix ships it), but a client with no Server stores no Sessions |
| `github-copilot-sdk`, `rmcp`, tokio's `process` and `signal` features | Provider runtimes that start child processes, which an iOS app may not do |
| `axum` | The Server's HTTP framework. It would compile, and the pairing client uses its body and error types today |
| `clap`, `tracing-appender`, `fs2`, `object`, `image` | Command line, log files, the election lock, build identity, clipboard images |

`shaman-relay-protocol` is the one crate already apart, and it depends on `base64`, `blake3`, `ring`, `serde`, `url`
and `x509-parser` only.

### Module by module

| Module | Lines | State | What it needs |
|---|---|---|---|
| `crates/shaman-relay-protocol` | 1,065 | Could be used as it is | Nothing. **Unverified** on an iOS target. |
| `src/protocol.rs`, `src/protocol/` | 10,454 | Moves as it is, bar one call | `WorkspacePaths::from_home` calls `crate::paths::canonical` (a one-line `dunce` wrapper) and `discover` calls `dirs::home_dir`. Both are for the owning machine and a client never calls them. Compiled outside the crate here. |
| `src/approval.rs`, `src/questionnaire.rs` | 838 | Move with the protocol | They and `protocol` import each other, so they travel together. |
| `src/session_projection.rs` | 3,074 | Moves as it is | Imports `crate::protocol` and `anyhow` only. Compiled outside the crate here. |
| `src/session_projection/agent_reading.rs` | | Stays behind | Uses `provider::provider_name`, `shown_text` and `time`. It renders a Session as text for an Agent, which a phone does not need. |
| `src/managed_client/session_projection.rs` | 42 | Moves as it is | A thin holder over the projection. |
| `src/managed_client/` stream readers (`session_stream`, `session_catalog_stream`, `subagent_tree_stream`, `event_stream`, `remote_connection`, the backoff in `recovery`) | about 2,600 | Need untangling | Written against `reqwest::Client`, a loopback base URL and a `watch::Receiver<RuntimeDescriptor>` that names the local Server. The decoding and reconnection logic is what a phone wants; the transport under it is not. |
| `src/managed_client.rs`, `launcher`, `lifecycle`, `keeper` | about 4,400 | Stay behind | Finding, starting and keeping a local Server: child processes, a lock file, the executable's own path. |
| Pairing transport in `src/serving.rs` (`PinnedServerKey`, `PairingHttpClient`, `SharedConnection`, `WayConnector`, `WayDialer`, `open_connection`, `dial_enrollment`, `paired_http_client`, `enrollment_certificate`, `parse_invite`, `InvitePayload`, `EnrollmentRequest`) | about 1,500 of 8,263 | Need untangling | All private. They share a file and a struct with the Serving side. See below. |
| `src/serving/short_invite.rs` | 2,058 | Close to movable | Imports `ByteStream`, `PairingFailure` and protocol types only, plus `spake2`, `curve25519-dalek`, `ring`. It holds both the issuing and the redeeming half. |
| `src/serving/pair_back.rs` | 1,886 | Stays behind for a first cut | Bound to axum routes and Serving settings. The map records that an offer to pair back is optional. |
| `src/serving/identity.rs`, `identity_store.rs` | 2,661 | Need a new backend and a new shape | The store is gated to macOS, Linux and Windows; on any other target it is `NoIdentityStore` and the key falls back to a file. The key is held as bytes. See [Keys](#keys). |
| `src/relays.rs`, `src/relays/` | 3,451 | Need untangling | One file holds the relay list and its persistence, login, the Serving side that waits to be reached, and the redeeming side that joins. It imports `ServingController` and settings types, and opens its WebSocket through `reqwest`. |

### The three untanglings

**1. The redeeming side out of `ServingController`.** One struct holds both halves of a pairing: the listener, the
invite ledger, the peers and the acceptor on one side; the remotes, the pinned-key clients and redemption on the other.
They share the identity key, the data directory and the timings. The redeeming half is identifiable (`redeem_invite`,
`redeem`, `probe_remote`, `proxy_remote`, `pairing_client`, the `keep_up` family) but not separable by moving a block:
it persists through `change_remotes` and `persist_remote`, reports through `PairingFailure` and `RemoteStatus`, and
reaches relays through the `RelayWays` trait.

Two seams already exist and would carry over: `ByteStream` (any byte stream the TLS runs over) and `RelayWays` (how a
relay way is joined, implemented in `src/relays.rs`). The pinned-key verifier and the TLS configuration are small and
self-contained. What is not: `dial_enrollment` and the client build requests with `axum::body::Body` and bound errors
with `axum::BoxError`, so the client half drags the server framework along unless those become `http-body-util` and
`hyper` types.

**2. A transport under the stream readers.** Today no Client speaks a pairing. The TUI's client speaks HTTP and SSE to
its own Server on loopback, and that Server speaks the pairing to the other machine and proxies it. A phone has no
Server, so it needs something that does not exist yet: the stream readers of `managed_client` running over the
pinned-key client of `serving`, asking `/v1/pairing/proxy/...` directly. That means putting an interface between the
readers and `reqwest::Client`, with one implementation for loopback and one for a pairing. **Inference:** this is the
largest piece of new design, and it is needed for a Rust core only. A native Swift client would write the readers
afresh over its own transport.

**3. The relay client.** Reaching a Server through a relay needs login at the relay, the proof of key, and the join.
All three live in `src/relays.rs` beside the Serving side's connection and the Server's persistence of its relay list.
The WebSocket is opened through `reqwest` "so it takes the system proxy and trust". **Unverified:** what reqwest's
`system-proxy` feature reads on iOS; its manifest ties the feature to `macos-system-configuration`.

### Platform assumptions inside the movable code

- `machine_hostname()` reads the system hostname and sends it at enrollment's commit, where the Serving side uses it to
  attribute what a Sidekick sends. **Unverified:** what `gethostname` returns on iOS. The map records the hostname as
  optional.
- `WorkspacePaths` and the 42 `PathBuf` fields describe paths on the owning Server. The types already carry a
  `PathStyle` so a client spells a remote path without resolving it, which is what a phone needs.
- Records (`remotes.json`, the relay list) are written with `replace_private_file` and `protect_current_user_file` from
  `src/runtime.rs`, which have Unix and Windows forms. An iOS client would write into its app container, so the
  directory would be handed in, not discovered.
- `SOCKET_USER_TIMEOUT` is already gated to Linux, Android and Fuchsia.

### What a Rust core would and would not carry

For weighing, without choosing. **Inference** throughout, from the tables above.

A core would carry, and a native client would rewrite: the 364 protocol types and their exact wire rules (228 reject
unknown fields, and `PROTOCOL_VERSION` must match exactly today); the Session projection that applies updates to a
snapshot; the reconnection logic of the streams; invite parsing and the two-phase enrollment; the pinned-key TLS with
its token-bearing certificate; the relay protocol, proof and join; and the short-code exchange over SPAKE2.

A core would not remove, and would add: the three untanglings; a binding crate and its error and stream glue; a signer
abstraction for the key; a build script, an XCFramework and a Swift package; an edit loop that crosses two toolchains;
and the standing cost of UniFFI's breaking releases. Apple's networking stack would be bypassed, with what that implies
under [Runtime](#runtime).

## Questions this research surfaced

1. **Does anything need the protocol types in Swift at all?** If Rust hands Swift a view model, the binding surface is
   tens of types and not hundreds. That choice is made before the binding is designed and shapes its whole cost.
2. **Who owns reconnection after a suspension?** The existing Rust logic would handle it unprompted, with timers that
   do not count sleep. The ticket on living with iOS waits on where the protocol code lives; this is the concrete
   question it inherits.
3. **Is the phone's key non-exportable from the first cut?** A Secure Enclave key cannot be added later without the
   phone pairing again, and it changes `IdentityMaterial` for every platform. It also decides whether a simulator build
   needs a second key backend.
4. **Does relaxing the exact-version rule change the sum?** A shared core keeps a phone and a Server agreeing by
   construction only while both are built from the same commit. Once the phone ships separately, a core pins the phone
   to the protocol of the day it was built, as native types would.
5. **Should the extraction happen regardless?** Splitting the protocol and the pairing client into crates would serve a
   desktop Client that pairs without a Server of its own, and would let the probe above become a real iOS build check.
   It is a prerequisite for a Rust core and independent of choosing one.
6. **Would an iOS build check be worth adding before the decision?** Installing the two iOS targets and running
   `cargo check` on `shaman-relay-protocol` and an extracted protocol crate would turn every **Unverified** in
   [Runtime](#runtime) into a fact, in minutes.

## Could not verify

- Any build of Shaman's code for an iOS target.
- The size of UniFFI's scaffolding and generated Swift for a surface of Shaman's size.
- The effect on an app's download or installed size for any precedent. Only Matrix's "174 MB to 62 MB" for the linked
  framework is documented.
- Local, as opposed to CI, build times for any precedent.
- Whether the Simulator can hold a Secure Enclave key.
- Whether a published crate already implements a Secure Enclave signer for rustls.
- Whether typeshare accepts serde enums tagged by a field alone.
- What `gethostname` and reqwest's system proxy lookup do on iOS.
- Any report from Element tying a lost connection after backgrounding to the Rust networking layer. Searches of
  element-x-ios found none.
- Crash symbolication practice on iOS for any precedent. Bitwarden avoids stripping to keep stack traces; none
  documents its handling of symbol files.

[uniffi-changelog]: https://github.com/mozilla/uniffi-rs/blob/main/CHANGELOG.md
[uniffi-crate]: https://crates.io/crates/uniffi
[uniffi-readme]: https://github.com/mozilla/uniffi-rs/blob/main/README.md
[uniffi-futures]: https://mozilla.github.io/uniffi-rs/latest/futures.html
[uniffi-async-overview]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/internals/async-overview.md
[uniffi-async-swift]: https://github.com/mozilla/uniffi-rs/blob/main/uniffi_bindgen/src/bindings/swift/templates/Async.swift
[uniffi-2771]: https://github.com/mozilla/uniffi-rs/issues/2771
[uniffi-2755]: https://github.com/mozilla/uniffi-rs/issues/2755
[uniffi-2448]: https://github.com/mozilla/uniffi-rs/issues/2448
[uniffi-foreign]: https://mozilla.github.io/uniffi-rs/latest/foreign_traits.html
[uniffi-errors]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/types/errors.md
[uniffi-swift]: https://mozilla.github.io/uniffi-rs/latest/swift/overview.html
[uniffi-swift-config]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/swift/configuration.md
[uniffi-proc]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/proc_macro/index.md
[uniffi-enums]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/types/enumerations.md
[uniffi-builtin]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/types/builtin_types.md
[uniffi-custom]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/types/custom_types.md
[uniffi-bindgen-swift]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/swift/uniffi-bindgen-swift.md
[uniffi-xcode]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/swift/xcode.md
[uniffi-gradle]: https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/kotlin/gradle.md
[typeshare]: https://github.com/1Password/typeshare
[swift-bridge]: https://github.com/chinedufn/swift-bridge
[swift-bridge-book]: https://chinedufn.github.io/swift-bridge/
[cbindgen]: https://github.com/mozilla/cbindgen
[boltffi]: https://github.com/boltffi/boltffi
[boltffi-streaming]: https://boltffi.dev/docs/streaming
[cargo-swift]: https://github.com/antoniusnaumann/cargo-swift
[cargo-ndk]: https://github.com/bbqsrc/cargo-ndk
[rust-ios]: https://doc.rust-lang.org/nightly/rustc/platform-support/apple-ios.html
[rust-instant]: https://doc.rust-lang.org/std/time/struct.Instant.html
[apple-xcframework]: https://developer.apple.com/documentation/xcode/creating-a-multi-platform-binary-framework-bundle
[apple-binary-swiftpm]: https://developer.apple.com/documentation/xcode/distributing-binary-frameworks-as-swift-packages
[apple-se]: https://developer.apple.com/documentation/security/protecting-keys-with-the-secure-enclave
[apple-signing]: https://developer.apple.com/documentation/security/signing-and-verifying
[tn3151]: https://developer.apple.com/documentation/technotes/tn3151-choosing-the-right-networking-api
[tn3179]: https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy
[tn2277]: https://developer.apple.com/library/archive/technotes/tn2277/_index.html
[android-keystore]: https://developer.android.com/privacy-and-security/keystore
[tokio]: https://docs.rs/tokio/latest/tokio/
[rustls]: https://docs.rs/rustls/latest/rustls/
[rustls-howto]: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html
[rustls-builder]: https://docs.rs/rustls/latest/rustls/struct.ConfigBuilder.html
[rustls-resolver]: https://docs.rs/rustls/latest/rustls/client/trait.ResolvesClientCert.html
[ring-ci]: https://github.com/briansmith/ring/blob/main/.github/workflows/ci.yml
[rpv]: https://github.com/rustls/rustls-platform-verifier/blob/main/README.md
[rcgen-signingkey]: https://docs.rs/rcgen/latest/rcgen/trait.SigningKey.html
[sf-token]: https://docs.rs/security-framework/latest/security_framework/key/enum.Token.html
[sf-seckey]: https://docs.rs/security-framework/latest/security_framework/key/struct.SecKey.html
[matrix-contrib]: https://github.com/matrix-org/matrix-rust-sdk/blob/main/bindings/CONTRIBUTING.md
[matrix-cargo]: https://github.com/matrix-org/matrix-rust-sdk/blob/main/Cargo.toml
[matrix-ffi-cargo]: https://github.com/matrix-org/matrix-rust-sdk/blob/main/bindings/matrix-sdk-ffi/Cargo.toml
[matrix-platform]: https://github.com/matrix-org/matrix-rust-sdk/blob/main/bindings/matrix-sdk-ffi/src/platform/mod.rs
[matrix-swift-xtask]: https://github.com/matrix-org/matrix-rust-sdk/blob/main/xtask/src/swift.rs
[matrix-apple]: https://github.com/matrix-org/matrix-rust-sdk/blob/main/bindings/apple/README.md
[matrix-blog]: https://matrix.org/blog/2023/09/matrix-2-0/
[matrix-4009]: https://github.com/matrix-org/matrix-rust-sdk/issues/4009
[matrix-6366]: https://github.com/matrix-org/matrix-rust-sdk/issues/6366
[matrix-6714]: https://github.com/matrix-org/matrix-rust-sdk/pull/6714
[element-contrib]: https://github.com/element-hq/element-x-ios/blob/develop/CONTRIBUTING.md
[element-1923]: https://github.com/element-hq/element-x-ios/issues/1923
[element-2266]: https://github.com/element-hq/element-x-ios/issues/2266
[element-2532]: https://github.com/element-hq/element-x-ios/issues/2532
[bw-cargo]: https://github.com/bitwarden/sdk-internal/blob/main/Cargo.toml
[bw-uniffi]: https://github.com/bitwarden/sdk-internal/blob/main/crates/bitwarden-uniffi/src/lib.rs
[bw-readme]: https://github.com/bitwarden/sdk-internal/blob/main/README.md
[bw-arch]: https://contributing.bitwarden.com/architecture/sdk/
[bw-ios-arch]: https://contributing.bitwarden.com/architecture/mobile-clients/ios/
[bw-bindings]: https://contributing.bitwarden.com/architecture/sdk/language-bindings
[moz-cargo]: https://github.com/mozilla/application-services/blob/main/Cargo.toml
[moz-building]: https://github.com/mozilla/application-services/blob/main/docs/building.md
[moz-megazords]: https://github.com/mozilla/application-services/blob/main/docs/design/megazords.md
[moz-viaduct]: https://github.com/mozilla/application-services/blob/main/components/viaduct/README.md
[moz-adr10]: https://github.com/mozilla/application-services/blob/main/docs/adr/0010-async-bridges-in-rust.md
[moz-adr3]: https://github.com/mozilla/application-services/blob/main/docs/adr/0003-swift-packaging.md
[moz-ios-readme]: https://github.com/mozilla/application-services/blob/main/megazords/ios-rust/README.md
[moz-local]: https://github.com/mozilla/application-services/blob/main/docs/howtos/locally-published-components-in-firefox-ios.md
[signal-bridge]: https://github.com/signalapp/libsignal/blob/main/rust/bridge/README.md
[signal-net]: https://github.com/signalapp/libsignal/blob/main/doc/src/net/README.md
[signal-tokio-swift]: https://github.com/signalapp/libsignal/blob/main/swift/Sources/LibSignalClient/TokioAsyncContext.swift
[signal-swift]: https://github.com/signalapp/libsignal/blob/main/swift/README.md
[signal-code-size]: https://github.com/signalapp/libsignal/blob/main/java/code_size.json
