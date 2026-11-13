# Can Apple's networking stack carry a Pairing (#634)

Researched 2026-11-13 for the map in #631. The question is whether a Swift app can speak the Pairing transport as the
Server implements it today, with Apple's frameworks alone or with well-maintained Swift packages, item by item.

Every claim below names its source. Where a line is my inference and not something a source states, it says
**Inference**. Where I could not check something, it says **Unverified**. Server-side facts were read from this
repository at `8c36caa1`. Apple API facts were read from the iOS 27.2 SDK inside Xcode 27.2 (headers and
`.swiftinterface` files) and from Apple's documentation. Swift package facts were read from the packages' sources.

Two throwaway programs were built and run on this Mac (macOS 27.0.1, Swift 6.4) to check what the sources only imply.
They are described in [What was run](#what-was-run). Nothing was run on an iPhone or against a real Shaman Server or
Relay.

## Answer in brief

Yes. A Swift app can speak the whole Pairing transport as it stands, with no change to the Server. It cannot do so on
Apple's frameworks alone: the Relay way needs swift-nio, swift-nio-ssl and swift-nio-http2, and two small pieces of
cryptography (SPAKE2 and BLAKE3) have no home in any Apple framework or Apple-maintained Swift package.

| # | Item | Apple frameworks alone | With Swift packages | What it takes |
|---|---|---|---|---|
| 1 | Mutual TLS 1.3, pinned SPKI, no CA, no SNI, self-minted certificate | The TLS: yes with Network.framework, on paper. Minting the certificate: no API | Yes, ran it | swift-nio-ssl for TLS, swift-certificates to mint |
| 1 | Secure Enclave identity key | Yes on paper (`SecIdentityCreate` with a Secure Enclave `SecKey`) | Yes, ran it | `NIOSSLCustomPrivateKey` over a CryptoKit Secure Enclave key |
| 2 | HTTP/2 with long-lived SSE | URLSession only, and App Transport Security gets in the way. Network.framework has no HTTP/2 | Yes, ran it | swift-nio-http2 |
| 3 | The Relay way: TLS inside WebSocket binary frames, HTTP/2 inside that | No | Yes, ran it | swift-nio (NIOWebSocket), swift-nio-ssl, swift-nio-http2 and one small handler of our own |
| 4 | Short code: SPAKE2 as RustCrypto `spake2` 0.4 | No | No maintained Swift package found | The Rust crate behind a C interface, or a port onto libsodium's point arithmetic |
| 4 | Short code: HKDF-SHA256, HMAC, ChaCha20-Poly1305 | Yes, CryptoKit | | |
| 5 | BLAKE3 | No | No Apple-maintained package | The official C implementation as a C target, or a small community wrapper |
| 6 | What iOS imposes | See [item 6](#6-what-ios-itself-imposes) | | |

The conclusion I draw (**Inference**): if the protocol code is written in Swift, write it once on swift-nio for both
direct ways and Relay ways. The Relay way forces that stack in anyway, App Transport Security does not apply to it, and
the app, not the system, decides what certificate is acceptable. URLSession and Network.framework's own TLS can carry a
direct way but neither can carry a Relay way, so using them means two transports.

## What the Server asks of a client

These are the facts the Swift side has to match, each checked in the code.

**The pinned-key TLS** (`src/serving.rs`):

- The Serving side accepts TLS 1.3 only (`serving_tls`, line 2213:
  `with_protocol_versions(&[&rustls::version::TLS13])`). The redeeming side dials TLS 1.3 only, turns SNI off and offers ALPN `h2` alone (`paired_http_client`, line 5954).
- A direct listener offers ALPN `h2` then `http/1.1`. A connection a Relay carried must offer `h2` or the handshake
  fails (`MultiplexedOnly`, line 4273). A direct connection that agrees no `h2` is served as HTTP/1.1
  (`serve_connection`, line 3218).
- The Server's certificate is whatever rcgen mints by default for the name `shaman-server`
  (`src/serving/identity.rs`, line 229): self-signed, subject `CN=rcgen self signed cert`, valid from 1975 to 4096, no
  key usage extension (rcgen 0.14.10 `CertificateParams::default`).
- The client checks the server by comparing the certificate's SubjectPublicKeyInfo DER, byte for byte, with the key
  the Invite named (`PinnedServerKey`, line 4744; `public_key_from_certificate`, line 6231). Nothing else about the
  certificate is checked beyond the handshake signature.
- The server checks the client the same way (`PinnedPeers`, line 4654). It admits a certificate whose key is an
  enrolled Peer's, or whose subject common name is `shaman-invite-` followed by the unpadded base64url of a live
  32-byte enrollment token (`enrollment_token_from_certificate`, line 6239; `enrollment_certificate`, line 5995).
- The handshake signature is verified by rustls-webpki with the presented certificate as its own root, so the client
  certificate must be one webpki parses: X.509 v3.
- The Server does not read SNI. Its certificate resolvers ignore the name (`MultiplexedOnly` reads only ALPN; rustls
  0.23.43 `SingleCertAndKey::resolve` ignores its argument). So "no SNI" is the client keeping a name out of the
  clear. A client that sent one would still connect. **Inference** from the code; I did not test a client sending SNI.
- Requests go to `https://shaman-server/...` (line 5270). Nothing in `src/serving.rs` reads the authority or `Host` of
  a request it is sent, and `passed_on` (line 3887) strips `Host` before a request is carried on. **Inference** that
  any authority is accepted; the runs used `shaman-server`.
- The Serving side sends an HTTP/2 PING every 15 seconds and gives a connection up 30 seconds after one goes
  unanswered (`src/server.rs`, lines 476–477). SSE streams carry a keepalive comment every 10 seconds
  (`src/server.rs`, line 426).

**The identity key** is ECDSA P-256. Its public form everywhere is the 91-byte DER SubjectPublicKeyInfo beginning
`3059301306072a8648ce3d020106082a8648ce3d03010703420004` (`KEY_FORMS` in `src/serving/short_invite.rs`, line 100). Its
fingerprint is the lowercase hex BLAKE3 of those 91 bytes (`crates/shaman-relay-protocol/src/proof.rs`, `fingerprint`).

**The Relay way** (`src/relays.rs`, `crates/shaman-relay-protocol/src/lib.rs`, `crates/shaman-relay/src/connection.rs`):

- A WebSocket opened by an HTTP/1.1 upgrade of `GET {relay}/connect`, with no subprotocol and no extensions
  (`websocket`, line 1977). The Relay's own address is `http` or `https`, and its certificate is checked against the
  system trust store in the ordinary way (`relay_http_client`, line 2048).
- Each protocol message is one JSON text frame: `hello` with the versions spoken and the key, a `challenge` back, a
  `proof` that is an ASN.1 DER ECDSA P-256 SHA-256 signature over `proof_message`, then `join` naming the Serving
  Server's key, answered by `joined`.
- After `joined` the connection carries only binary frames, each passed on as sent. No frame may exceed 64 KiB either
  way (`MAX_MESSAGE_LEN`; the Relay sets `max_frame_size` to it). The Server writes its TLS bytes as frames of at most
  that size and reads them back as one ordered byte stream (`CarriedStream`, line 2364). The Relay pings idle
  connections; a text frame after `joined` is an error.
- The pinned-key TLS then runs end to end over those bytes with HTTP/2 inside, exactly as on a direct way.

**The short code** (`src/serving/short_invite.rs`, ADR-0065): an exchange over a join's raw bytes, before any TLS. It
uses RustCrypto `spake2` 0.4.0's `Ed25519Group` with side A for the redeemer, the code as password and the two identity
keys as identities; SHA-256 for the transcript; HKDF-SHA-256 salted with the transcript to derive three keys;
HMAC-SHA-256 proofs; and ChaCha20-Poly1305 with an all-zero nonce and the transcript as associated data. The HKDF, HMAC and AEAD come
from `ring`.

## 1. Mutual TLS 1.3 with pinned self-signed certificates

### swift-nio-ssl: possible, and run

swift-nio-ssl (2.37.6, 2 November 2026) wraps a vendored BoringSSL. Each requirement maps to a public API:

| Requirement | API | Source |
|---|---|---|
| TLS 1.3 only | `TLSConfiguration.minimumTLSVersion = .tlsv13` | [`TLSConfiguration.swift`][nio-ssl-config] |
| Pin by SPKI, no CA | `NIOSSLClientHandler(context:serverHostname:customVerificationCallback:)`, then `NIOSSLCertificate.extractPublicKey().toSPKIBytes()` | [`NIOSSLClientHandler.swift`][nio-ssl-client], [`SSLPublicKey.swift`][nio-ssl-pubkey] |
| No SNI | `serverHostname: nil` | [`NIOSSLClientHandler.swift`][nio-ssl-client] |
| Client certificate | `TLSConfiguration.certificateChain`, `privateKey` | [`TLSConfiguration.swift`][nio-ssl-config] |
| ALPN `h2` | `TLSConfiguration.applicationProtocols` | [`TLSConfiguration.swift`][nio-ssl-config] |
| Key that never leaves the Secure Enclave | `NIOSSLCustomPrivateKey` | [`CustomPrivateKey.swift`][nio-ssl-customkey] |

Three details matter:

- The custom verification callback "will override _all_ verification logic that BoringSSL provides"
  ([`SSLCallbacks.swift`][nio-ssl-callbacks]). It is given the certificates the peer presented and decides alone. So
  the Server's odd certificate (valid for two thousand years, no CA) is the app's business and nobody else's.
- The callback "will not be used if the `TLSConfiguration` ... has `certificateVerification` set to
  `CertificateVerification/none`" ([`NIOSSLClientHandler.swift`][nio-ssl-client]). Set `.noHostnameVerification` and
  supply the callback.
- SNI is set only inside `if let serverHostname`, so `nil` sends none
  ([`NIOSSLClientHandler.swift`][nio-ssl-client]). The rustls stand-in saw `SNI None`.

`NIOSSLCustomPrivateKey` exists for this case: "it is advantageous to store a TLS private key in some form of
high-security storage ... This protocol allows a type to implement callbacks that perform the specific operation
required by the TLS handshake" ([`CustomPrivateKey.swift`][nio-ssl-customkey]). The implementer names
`.ecdsaSecp256R1Sha256` and returns a DER signature from `sign(channel:algorithm:data:)`.

### Minting the certificate: swift-certificates

No Apple framework creates an X.509 certificate on iOS. Security.framework can read one (`SecCertificateCreateWithData`)
but has no call that builds or signs one; an Apple engineer's answer to a developer who needed a self-signed
certificate for a Secure Enclave key was to "file an enhancement request for an API to make this work"
([forum thread 744854][forum-se]). **Inference**: the absence is from reading the Security headers in the iOS 27.2 SDK
and that thread, not from a sentence that says "there is no such API".

swift-certificates (1.21.1) does it. The `Certificate` initialiser that takes an `issuerPrivateKey` "can be used to construct a self-signed key by passing the private key for `publicKey` as the `issuerPrivateKey`
argument" ([`Certificate.swift`][certs-certificate]). `Certificate.PrivateKey` has initialisers for
`P256.Signing.PrivateKey`, `SecureEnclave.P256.Signing.PrivateKey` and `SecKey`
([`CertificatePrivateKey.swift`][certs-privatekey]). The common name is written as a UTF8String, which the Server's
`as_str()` reads.

A certificate with one common name and a P-256 key is under 400 bytes of DER, so encoding it by hand without the
package is feasible. **Inference**; I did not write one.

### Network.framework: possible on paper, not run

The iOS 27.2 SDK declares everything the TLS needs (`Security/SecProtocolOptions.h`, `Security/SecIdentity.h`):

- `sec_protocol_options_set_min_tls_protocol_version` (iOS 13).
- `sec_protocol_options_set_verify_block` (iOS 12). An Apple engineer: "You can override server trust evaluation by
  via sec_protocol_options_set_verify_block" ([forum thread 104018][forum-verify]). Inside it,
  `sec_trust_copy_ref`, then `SecTrustCopyCertificateChain`, `SecCertificateCopyKey` and
  `SecKeyCopyExternalRepresentation` give the 65-byte point, which is the tail of the 91-byte SPKI.
- `sec_protocol_options_set_local_identity` with `sec_identity_create` over a `SecIdentity`.
- `SecIdentityCreate(allocator, certificate, privateKey)`, public and marked available from iOS 11.2
  ([Apple documentation][doc-secidentitycreate]). This builds the identity in memory from a certificate and a
  `SecKey`, so the certificate need not be stored in the keychain to be matched with its key. That matters for the
  enrollment certificate, which is used once.
- `sec_protocol_options_add_tls_application_protocol` for ALPN.

The same options are on the iOS 26 `NetworkConnection` API as `TLS().version(min:)`, `.certificateValidator`,
`.localIdentity` and `.applicationProtocols` (`Network.swiftinterface`).

Caveats:

- **SNI.** The only related call is `sec_protocol_options_set_tls_server_name`, which overrides the name; nothing
  documented turns SNI off. A connection to an IP address should carry none, since RFC 6066 section 3 forbids address
  literals there. A connection to a name, such as a Tailscale MagicDNS name, would carry it. **Inference**, and
  harmless to the Server as noted above.
- **System checks that ignore the verify block.** iOS 18.4 added a key usage check "to the Network framework path in a
  way that applies even if you supply your own verification block" ([forum thread 780430][forum-keyusage]). It concerns
  certificates that carry a key usage extension; the Server's carries none, so it should pass. **Inference**, and it
  shows that Apple's TLS can refuse a certificate before the app is asked.
- **Which Secure Enclave key.** A CryptoKit `SecureEnclave.P256.Signing.PrivateKey` is not a `SecKey` and cannot be
  turned into the matching one ([forum thread 744854][forum-se]). For this path the key must be made by
  `SecKeyCreateRandomKey` with `kSecAttrTokenIDSecureEnclave`.
- **Unverified**: none of this was run. In particular I did not confirm that Network.framework accepts the Server's
  certificate, or a Secure Enclave identity built by `SecIdentityCreate`, in a TLS 1.3 handshake on iOS 27.

### URLSession: possible in API terms, wrong fit

URLSession can pin (`NSURLAuthenticationMethodServerTrust`), present a client identity
(`NSURLAuthenticationMethodClientCertificate` with `URLCredential(identity:certificates:persistence:)`) and hold the
floor at TLS 1.3 (`tlsMinimumSupportedProtocolVersion`, iOS 13). App Transport Security is the obstacle; see
[item 6](#app-transport-security). It also dials for itself, so it cannot be handed a Relay join.

### Can the key be a non-exportable Secure Enclave key?

Yes.

- The Secure Enclave "works only with NIST P-256 elliptic curve keys", for signatures and key agreement
  ([Protecting keys with the Secure Enclave][doc-se]). The identity key is ECDSA P-256, and every use of it is a
  signature: the TLS handshake (`ecdsa_secp256r1_sha256`), the certificates it signs for itself, and the Relay proof
  (ECDSA P-256 SHA-256, ASN.1 DER, which is what CryptoKit's `signature(for:).derRepresentation` produces).
- The private key cannot be exported: "the Secure Enclave exports an encrypted block that only the same Secure Enclave
  can later use to restore the key" ([Storing CryptoKit Keys in the Keychain][doc-cryptokit-keychain]). The app stores
  that block.
- CryptoKit's default access control for such a key is `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` with no
  user-presence flag (`CryptoKit.swiftinterface`), so reconnecting needs no prompt.
- `P256.Signing.PublicKey.derRepresentation` is the 91-byte SPKI the Server pins: the run printed the same 27-byte
  opening as `KEY_FORMS`.
- Run: a key made by `SecureEnclave.P256.Signing.PrivateKey()` on this Mac signed its own certificate through
  swift-certificates and signed the TLS 1.3 handshake through `NIOSSLCustomPrivateKey`; rustls verified both.

What follows from it (**Inference**): the key is bound to one device and is in no backup, so a restored or replaced
phone is a new Peer that pairs again. The simulator's support for Secure Enclave keys is **Unverified**.

## 2. HTTP/2 over that connection, with SSE

- **swift-nio-http2** (1.46.0): possible, and run. `NIOHTTP2Handler` reads and writes byte buffers, so it sits directly
  above the TLS handler; `configureHTTP2Pipeline(mode: .client, ...)` returns a multiplexer that opens a channel for
  each stream, and `HTTP2FramePayloadToHTTP1ClientCodec` lets each stream be spoken as an ordinary request and response
  ([`HTTP2PipelineHelpers.swift`][nio-h2-helpers], [`HTTP2ToHTTP1Codec.swift`][nio-h2-codec]). It answers the Server's
  PINGs itself ([`HTTP2ChannelHandler.swift`][nio-h2-handler], `.ping` case); the run held a connection through three
  seconds of hyper's PINGs sent every 200 ms. An SSE stream is one stream channel that stays open while others come
  and go. What the package does not
  give is an HTTP client: request and response handling, SSE parsing, timeouts and reconnection are ours to write.
- **URLSession**: speaks HTTP/2 when the server agrees it by ALPN ([URLSession][doc-urlsession]), and
  `URLSession.bytes(for:)` reads a stream as it arrives. Its idle timer defaults to 60 seconds and "is reset whenever
  new data arrives" ([timeoutIntervalForRequest][doc-timeout]); the Server's 10-second SSE keepalive keeps it reset.
  Only for direct ways, and subject to App Transport Security.
- **Network.framework**: no public HTTP/2. In the iOS 27.2 `Network.swiftinterface` the only mentions of HTTP are
  ports, DNS-over-HTTPS and proxy configuration.

A direct listener also serves HTTP/1.1 to a client that agrees no `h2`, so HTTP/2 is strictly required only through a
Relay.

## 3. The Relay way

### Apple frameworks alone: no

`URLSessionWebSocketTask` covers the first half: it opens the WebSocket, sends and receives JSON text frames, and
sends and receives binary frames (`NSURLSession.h`). The second half has nowhere to go:

- **No Apple TLS 1.3 runs over bytes the app supplies.** Secure Transport is the one Apple API built for that: "you
  provide callback functions to perform I/O on the underlying network connections". It stops at TLS 1.2 ("Transport
  Layer Security (TLS) versions 1.0 through 1.2") and "is considered legacy" ([Secure Transport][doc-securetransport]).
  The SDK marks it deprecated with "No longer supported. Use Network.framework."
- **Network.framework can stack TLS above a custom protocol, but only inside its own connection.**
  `NWProtocolFramer.Instance.prependApplicationProtocol(options:)` exists for "adding a security protocol, like TLS,
  above a framer once that framer completes its initial handshake" (`Network/framer_options.h`). A framer that spoke
  the Relay's JSON and then turned binary frames into bytes could in principle put the pinned TLS above itself.
  Whether a framer can sit above `NWProtocolWebSocket` and write binary messages through it, or would have to
  implement WebSocket framing itself, is **Unverified**. The iOS 26 typed API rules the tidy version out: `TLS` there
  can only be built over `TCP` (`public init(_ builder: () -> TCP)`).
- **Even then there is no HTTP/2.** The plaintext that came out of such a connection would need an HTTP/2
  implementation, and Apple ships none as public API. URLSession's cannot be pointed at an existing connection.

So the verdict is "not with Apple frameworks alone": the last step needs swift-nio-http2 whatever carries the bytes.

### swift-nio, swift-nio-ssl and swift-nio-http2: possible, and run

A NIO channel is a pipeline of handlers that each transform what passes through, and handlers can be added to a live
pipeline. `NIOSSLHandler` takes byte buffers in and gives byte buffers out in both directions
([`NIOSSLHandler.swift`][nio-ssl-handler]); it has no notion of a socket. So the whole Relay way is one pipeline:

```text
TCP (and the Relay's own TLS for an https Relay)
  HTTP/1.1 client handlers, removed by the upgrade
  WebSocketFrameEncoder / WebSocketFrameDecoder      NIOWebSocket, added by NIOWebSocketClientUpgrader
  our handler: JSON text frames                      hello, challenge, proof, join, joined; removes itself
  our handler: binary frames <-> byte buffers        about forty lines
  NIOSSLClientHandler                                the pinned TLS 1.3, no SNI, ALPN h2
  NIOHTTP2Handler and its stream multiplexer
```

The pieces, all public API:

- `NIOWebSocketClientUpgrader` and `addHTTPClientHandlers(withClientUpgrade:)` perform the upgrade
  ([`NIOWebSocketClientUpgrader.swift`][nio-ws-upgrader]). Its `maxFrameSize` defaults to 16,384 bytes and must be
  raised to 65,536, or a full-size frame from the Relay is a protocol error.
- `WebSocketFrame` carries the opcode, the masking key a client must use, and the data
  ([`WebSocketFrame.swift`][nio-ws-frame]). NIOWebSocket does not answer pings by itself; the handler does.
- The one handler of our own turns each byte buffer written from above into binary frames of at most 64 KiB, and each
  binary frame read from below into a byte buffer. It is the Swift twin of `CarriedStream`.
- For an `https` Relay the outer TLS is an ordinary `NIOSSLClientHandler` with the Relay's hostname. On Apple platforms
  swift-nio-ssl checks the default trust roots through Security.framework's `SecTrust`
  ([`SSLContext.swift`][nio-ssl-context], [`SecurityFrameworkCertificateVerification.swift`][nio-ssl-sectrust]), which
  matches the Server's use of the system trust store.

**Inference** made **verified by the run**: no swift-nio document describes TLS inside WebSocket frames. That it works
follows from the handler types, and the loopback run confirmed it: TLS 1.3 with mutual pinned certificates and ALPN
`h2`, over binary frames that followed two JSON text frames, carrying three concurrent HTTP/2 streams, one of 300,000
bytes.

The short code's exchange also runs over a join's raw bytes, before TLS. In this pipeline that is the same point: the
exchange is spoken through the binary-frame handler with no TLS handler above it.

### NIOPosix or NIOTransportServices underneath

swift-nio runs on BSD sockets (NIOPosix) or on Network.framework (swift-nio-transport-services). Apple's guidance for
iOS is the second: TN3151 names "SwiftNIO Transport Services, which makes it straightforward to use Network framework
on Apple platforms", and warns that with BSD sockets "your program will still be incompatible with VPN On Demand"
([TN3151][tn3151]). `NIOTSConnectionBootstrap` gives an ordinary NIO channel, so the pipeline above is unchanged, and
its `tlsOptions` could give the outer hop to the Relay Apple's TLS
([`NIOTSConnectionBootstrap.swift`][niots-bootstrap]). The runs here used NIOPosix. That the pipeline behaves the same
on NIOTransportServices is **Inference**.

## 4. The short code

### SPAKE2: no Apple framework, no maintained Swift package

What has to be matched is small and exact (`spake2` 0.4.0, `src/ed25519.rs` and `src/lib.rs`):

- The group is edwards25519. The blinding points M and N are fixed constants taken from python-spake2.
- The password scalar is HKDF-SHA256 (empty salt, info `SPAKE2 pw`) expanded to 48 bytes, read as a big-endian integer
  and reduced modulo the group order.
- Side A sends `0x41` then `x·B + pw·M`; side B sends `0x42` then `y·B + pw·N`. Each finishes with
  `K = x·(Y − pw·N)` or its mirror.
- The key is `SHA256( SHA256(pw) ‖ SHA256(idA) ‖ SHA256(idB) ‖ X ‖ Y ‖ K )`.
- The Server additionally refuses a share that is not a canonical encoding of a torsion-free point of large order.
- The crate carries a known-answer test (`test_asymmetric`) with fixed scalars, which a port can be checked against.

Where it could come from:

- **CryptoKit**: no. The macOS 27 and iOS 27.2 interfaces declare no PAKE, and `Curve25519` exposes only signing and
  key agreement, not point addition or multiplication by an arbitrary scalar.
- **swift-crypto** (5.1.1): no public SPAKE2. Its vendored BoringSSL contains `spake25519.cc` and `spake2plus.cc`, but
  inside `CCryptoBoringSSL`, which is not a product of the package. BoringSSL's SPAKE2 is also not this one: it is not
  the python-spake2 construction. **Inference** on the second point; I did not compare constants.
- **Other Swift packages**: a search found Swift Ed25519 signing packages and no SPAKE2 compatible with python-spake2
  or magic-wormhole. "None found" is not "none exists".

So there are two real options:

1. **The Rust crate behind a C interface.** Exactly compatible by construction, and what a shared Rust core would give
   for free (#635).
2. **A Swift port onto libsodium.** libsodium has the needed primitives: `crypto_core_ed25519_is_valid_point`,
   `crypto_core_ed25519_add`, `crypto_core_ed25519_sub`, `crypto_scalarmult_ed25519_noclamp`,
   `crypto_scalarmult_ed25519_base_noclamp`, `crypto_core_ed25519_scalar_reduce`, `crypto_core_ed25519_scalar_negate`
   ([libsodium point arithmetic][libsodium-points]). swift-sodium (0.11.0, May 2026) ships libsodium as a prebuilt
   framework for iOS and exposes the C API as `Clibsodium` ([swift-sodium][swift-sodium]). libsodium's functions
   refuse points outside the prime-order subgroup; I checked with `curve25519-dalek` that M and N are both
   torsion-free, so they are accepted. The port would be roughly a hundred lines of our own cryptographic code over an
   exchange ADR-0065 already notes is unaudited. Whether swift-sodium's prebuilt framework includes these functions is
   **Unverified**.

A third option is not to need it: the short code is one of two ways to deliver an Invite, and the long string is the
other.

### HKDF, HMAC and ChaCha20-Poly1305: CryptoKit alone

- `HKDF<SHA256>.extract(inputKeyMaterial:salt:)` and `.expand(pseudoRandomKey:info:outputByteCount:)`, or
  `deriveKey(inputKeyMaterial:salt:info:outputByteCount:)`, iOS 14 (`CryptoKit.swiftinterface`). The Server extracts
  with the transcript as salt and expands once per label to 32 bytes.
- `HMAC<SHA256>`.
- `ChaChaPoly.seal(_:using:nonce:authenticating:)` and `open`, iOS 13. The nonce is `ChaChaPoly.Nonce(data:)` over
  twelve zero bytes. `ring` writes ciphertext then tag; CryptoKit's `SealedBox.combined` puts the nonce first, so build
  the box with `SealedBox(nonce:ciphertext:tag:)`.

**Inference**: that these agree with `ring` follows from both implementing RFC 5869, RFC 2104 and RFC 8439. I ran no
cross-check.

## 5. BLAKE3

- **CryptoKit**: no. Its hashes are SHA-1, SHA-2, SHA-3 and MD5 (`CryptoKit.swiftinterface`).
- **swift-crypto**: no. The vendored BoringSSL has BLAKE2 only, and not as public API.
- **The official C implementation** ([BLAKE3-team/BLAKE3][blake3], 1.8.7, August 2026, Apache-2.0 by GitHub's
  licence field) is a handful
  of C files with a three-call API (`blake3_hasher_init`, `_update`, `_finalize`). Its README gives a portable build
  and notes "the NEON implementation is enabled by default on AArch64". Vendoring it as a C target in the app's Swift
  package is the dependable route. **Inference**: I did not build it for iOS.
- **Community Swift packages** exist and are small: `JoshBashed/blake3-swift` (a binding, 4 stars, last pushed August
  2025) and `nixberg/blake3-swift` (13 stars, last pushed November 2023). Neither is well-maintained in the sense the
  ticket asks for.
- A Rust core would bring the `blake3` crate the Server already uses.

The fingerprint is how a key is shown to a person and how a Peer is named. A phone needs it at least to show the
fingerprint of an Invite's key before trusting it.

## 6. What iOS itself imposes

### Local network privacy

- A direct way over Wi-Fi triggers the prompt. "Outgoing traffic to a local network address requires local network
  access", and TN3179's table lists "Making an outgoing TCP connection" as needing it ([TN3179][tn3179]).
- It applies whatever the API: "The system implements these TCP and UDP checks deep in the networking stack, and thus
  they apply to all networking APIs. This includes Network framework, BSD Sockets, URL Loading System, and any APIs
  implemented on top of those."
- The app should carry `NSLocalNetworkUsageDescription`: "Any app that uses the local network, directly or
  indirectly, should include this description" ([NSLocalNetworkUsageDescription][doc-localnetwork]).
  `NSBonjourServices` is only for Bonjour.
- The first attempt can fail before the user has answered: the system "may deny the operation immediately, before the
  user has responded to the alert. To handle this smoothly, use an API that supports waiting for connectivity ... If
  you can't use one of these preferred APIs, add appropriate retry logic."
- Network.framework reports the denial precisely (`waiting` with `NWPath.UnsatisfiedReason.localNetworkDenied`) and
  retries when access is granted. BSD sockets get a bare errno. That is one more reason to put
  NIOTransportServices under swift-nio.
- There is no API to ask whether access has been granted, and the simulator does not implement any of this.
- Tailscale addresses are not local network addresses. "A local network is an IP network associated with a
  broadcast-capable network interface. Such interfaces include Wi-Fi and Ethernet, but not cellular (WWAN) or VPN."
  **Inference**: Apple does not name Tailscale; this is that definition applied.

### App Transport Security

- It applies to URLSession and nothing below it. "ATS doesn't apply to calls your app makes to lower-level networking
  interfaces like the Network framework or CFNetwork" ([Preventing Insecure Network Connections][doc-ats]). An Apple
  engineer: "If you're using lower-level APIs, like Network framework or BSD Sockets, ATS is not a concern"
  ([forum thread 747421][forum-ats]). So swift-nio, on either transport, is outside it.
- For URLSession it blocks a direct way twice over. "In iOS 17, iPadOS 17, and macOS 14, ATS no longer allows
  connections to IP addresses by default" ([NSAllowsLocalNetworking][doc-ats-local]). And "when ATS is enabled, you can
  no longer loosen trust evaluation requirements that way, but you can still tighten them", so a delegate cannot accept
  a self-signed certificate for a host ATS protects ([Preventing Insecure Network Connections][doc-ats]).
- Exceptions go in `Info.plist` and match what is dialled: addresses, CIDR ranges or names in `NSExceptionDomains`
  with `NSExceptionAllowsInsecureHTTPLoads`, or `NSAllowsArbitraryLoads` for everything. An app that lets its user type
  any address needs the last. That is harmless for an app deployed from Xcode and a point to justify at App Review.
- A Relay at a public `https` address with an ordinary certificate needs no exception. A Relay at an `http` address
  would, under URLSession. **Inference** from ATS covering "URLSession and things layered on top of that"; Apple's
  page for `URLSessionWebSocketTask` does not mention ATS.

### Suspension

- A suspended app's connections do not survive reliably. "While the app is suspended the system may choose to reclaim
  resources out from underneath a network socket used by the app, thereby closing the network connection"
  ([TN2277][tn2277]).
  An Apple engineer in 2019: "On current system, it runs as soon as your app get suspended"
  ([forum thread 117150][forum-suspend]).
- Suspension follows backgrounding closely. The app gets about five seconds in its delegate method, and a background
  task buys a finite, unguaranteed extension that an Apple engineer puts at "about 30 seconds"
  ([forum thread 85066][forum-bgtime]).
- There is no supported way for an ordinary app to hold a connection open in the background. Asked whether one can,
  an Apple engineer answered: "The short answer to this question is 'No'" ([forum thread 97824][forum-bgconn]).
  Background URLSession carries uploads and downloads only.
- While the app is in the foreground nothing limits a stream's life. **Unverified** as a guarantee; no Apple
  statement sets a limit.
- The Server will notice on its own: an unanswered HTTP/2 PING ends the connection within 45 seconds.
- TN2277's advice fits the Pairing: where reconnecting is cheap, "close the socket when the app goes into the
  background, and reopen it when it comes back into the foreground". Each return is a new TLS handshake and one
  Secure Enclave signature.
- A network change (Wi-Fi to cellular) ends a TCP connection too. Network.framework signals it
  (`betterPathUpdateHandler`, `viabilityUpdateHandler`); BSD sockets do not, and TN3151 warns that without monitoring
  "your program won't notice that a connection is stuck". The Server's PING timeout is the backstop either way.

### Tailscale

- With the Tailscale app's VPN on, another app reaches `100.x.y.z` addresses and MagicDNS names with no special code:
  Tailscale's iOS client is a Network Extension, and the system routes matching packets and DNS queries to it
  ([NEPacketTunnelProvider][doc-tunnel]; Tailscale's source names its "iOS network extension").
- iOS runs one VPN at a time: "iOS and Android enforce a limit of running only one VPN at a time"
  ([Tailscale][ts-othervpn]).
- No local network prompt and, off URLSession, no ATS, as above.
- With VPN On Demand, BSD sockets do not bring the tunnel up (TN3151). NIOTransportServices does.
  **Inference** for the second half.
- Tailscale can also be embedded: `libtailscale` and its Swift `TailscaleKit` put a tailnet node inside the app with no
  system VPN ([libtailscale][libtailscale]). It is a separate node that needs its own authorisation, and its README
  states no support level. Not needed for the first cut.
- **Unverified**: how MagicDNS names resolve from other apps in detail, and behaviour with the phone locked.

## What was run

Both programs were throwaway, built outside the repository, and are not committed.

**A loopback check of item 3.** One Swift executable holding both ends. Packages as resolved: swift-nio 2.105.0,
swift-nio-ssl 2.37.6, swift-nio-http2 1.46.0, swift-certificates 1.21.1, swift-crypto 5.1.1, swift-asn1 1.7.4. The
client is the pipeline in [item 3](#3-the-relay-way), on NIOPosix, to a plain `ws` stand-in for a Relay and a Serving
Server. Output, trimmed:

```text
phone identity key: Secure Enclave, SPKI 91 bytes, starts 3059301306072a8648ce3d020106082a8648ce3d03010703420004
phone certificate: 398 bytes of DER, self-signed by that key
server: heard text frame {"type":"join","server":"c2VydmVy"}
client: heard text frame {"type":"joined"}
client: server key pinned: true
client: inner TLS done, version tlsv13, ALPN h2
server: client certificate subject CN=shaman-invite-RSPUE-Zkd3OZgtInFoTzNLKI5R8VuuL5JIKIN748MyM, key pinned: true
server: GET /v1/pairing/proxy/a over HTTP/2          (and /b, and /large, at once)
client: HTTP 200, 300000 bytes
handshake signatures made by the held-elsewhere key: 1
binary frames: client sent 19, took 17; server sent 17, took 19
```

**An interoperability check of items 1 and 2.** The same Swift client TLS and HTTP/2, dialled directly to a small Rust
program built from the Server's crates (rustls 0.23.46 with `ring`, rustls-webpki 0.103.17, rcgen 0.14.10, x509-parser
0.18.1, hyper 1.12.0). The Rust side copies `serving_tls` and `PinnedPeers`, admitting only a certificate that names a
given enrollment token, and mints its certificate as `identity.rs` does. Output of the Rust side:

```text
verify_client_cert: certificate 397 bytes, key 91 bytes starting 3059301306072a8648ce3d020106082a8648ce3d03010703420004, invited true
verify_tls13_signature: scheme ECDSA_NISTP256_SHA256, valid
handshake done: version Some(TLSv1_3), ALPN Some("h2"), SNI None, client certificates 1
request: GET https://shaman-server/v1/pairing/proxy/hello HTTP/2.0
```

The Swift side pinned the rcgen certificate's key, got `HTTP 200`, and was still connected after three seconds of
hyper's keepalive PINGs at 200 ms.

**A check for item 4.** `curve25519-dalek` 4.1.3 reports `is_torsion_free()` true for both of `spake2`'s M and N.

What these do not show: a real Shaman Server or Relay, the enrollment POST, an iPhone, NIOTransportServices, or any
Network.framework or URLSession path.

## Unverified

- Network.framework's TLS accepting the Server's certificate and a `SecIdentityCreate` identity over a Secure Enclave
  `SecKey`, on iOS 27.
- Whether an `NWProtocolFramer` can sit above `NWProtocolWebSocket` and carry TLS above itself.
- The pipeline on NIOTransportServices rather than NIOPosix, and on a device rather than a Mac.
- Secure Enclave keys in the iOS simulator.
- Whether swift-sodium's prebuilt framework includes libsodium's edwards25519 point functions.
- CryptoKit's HKDF, HMAC and ChaCha20-Poly1305 against `ring`'s on the short code's exact inputs.
- BLAKE3's C implementation building as a Swift package target for iOS.
- A client that sends SNI being accepted by the Server.
- That a foreground app's stream may stay open without limit.

## Questions this raised

1. **Does the first cut need the short code?** It is the only thing that needs SPAKE2. Pasting the long Invite, or
   scanning it as a QR code from the Serving machine's screen, needs none of item 4. Belongs with #638.
2. **One transport or two?** swift-nio for every way, or URLSession for direct ways with swift-nio only for Relays.
   I recommend one. And beneath swift-nio, NIOTransportServices over NIOPosix, for local network errors, VPN On
   Demand and network-change signals. Belongs with #637 and #639.
3. **Which Secure Enclave key?** A CryptoKit key suits swift-nio-ssl; a `SecKey` suits Network.framework; one cannot
   become the other. Either way the key is in no backup, so a new phone pairs again. Belongs with #640.
4. **What a Swift-native core has to write**, for weighing against a Rust core in #637: an HTTP client on
   swift-nio-http2 (requests, SSE, timeouts, reconnection), the Relay conversation, the binary-frame handler, and
   either a SPAKE2 port or a C interface to the Rust crate, plus BLAKE3 in C. A Rust core has all of it already,
   including rustls, tungstenite, hyper, `spake2` and `blake3`.
5. **Should a prototype pair with a real Server before the spec is written?** Everything above was checked against
   stand-ins. Pairing a Swift client with a running Server, directly and through a Relay, on a phone, would retire most
   of the Unverified list in one go.
6. **A phone needs its own Login at a Relay**, by the same GitHub device flow, before any Relay way or short code
   works. The map's notes record the rule; the app needs a place for it.
7. **Reconnecting is the normal case on iOS**, on every return to the foreground. Does the Server want anything to make
   that cheaper, such as TLS session resumption, or is a full handshake each time acceptable? Belongs with the map's
   "Living with iOS".

[nio-ssl-config]: https://github.com/apple/swift-nio-ssl/blob/2.37.6/Sources/NIOSSL/TLSConfiguration.swift
[nio-ssl-client]: https://github.com/apple/swift-nio-ssl/blob/2.37.6/Sources/NIOSSL/NIOSSLClientHandler.swift
[nio-ssl-handler]: https://github.com/apple/swift-nio-ssl/blob/2.37.6/Sources/NIOSSL/NIOSSLHandler.swift
[nio-ssl-pubkey]: https://github.com/apple/swift-nio-ssl/blob/2.37.6/Sources/NIOSSL/SSLPublicKey.swift
[nio-ssl-customkey]: https://github.com/apple/swift-nio-ssl/blob/2.37.6/Sources/NIOSSL/CustomPrivateKey.swift
[nio-ssl-callbacks]: https://github.com/apple/swift-nio-ssl/blob/2.37.6/Sources/NIOSSL/SSLCallbacks.swift
[nio-ssl-context]: https://github.com/apple/swift-nio-ssl/blob/2.37.6/Sources/NIOSSL/SSLContext.swift
[nio-ssl-sectrust]: https://github.com/apple/swift-nio-ssl/blob/2.37.6/Sources/NIOSSL/SecurityFrameworkCertificateVerification.swift
[nio-h2-helpers]: https://github.com/apple/swift-nio-http2/blob/1.46.0/Sources/NIOHTTP2/HTTP2PipelineHelpers.swift
[nio-h2-codec]: https://github.com/apple/swift-nio-http2/blob/1.46.0/Sources/NIOHTTP2/HTTP2ToHTTP1Codec.swift
[nio-h2-handler]: https://github.com/apple/swift-nio-http2/blob/1.46.0/Sources/NIOHTTP2/HTTP2ChannelHandler.swift
[nio-ws-upgrader]: https://github.com/apple/swift-nio/blob/2.105.0/Sources/NIOWebSocket/NIOWebSocketClientUpgrader.swift
[nio-ws-frame]: https://github.com/apple/swift-nio/blob/2.105.0/Sources/NIOWebSocket/WebSocketFrame.swift
[niots-bootstrap]: https://github.com/apple/swift-nio-transport-services/blob/1.28.0/Sources/NIOTransportServices/NIOTSConnectionBootstrap.swift
[certs-certificate]: https://github.com/apple/swift-certificates/blob/1.21.1/Sources/X509/Certificate.swift
[certs-privatekey]: https://github.com/apple/swift-certificates/blob/1.21.1/Sources/X509/CertificatePrivateKey.swift
[doc-secidentitycreate]: https://developer.apple.com/documentation/security/secidentitycreate(_:_:_:)
[doc-se]: https://developer.apple.com/documentation/security/protecting-keys-with-the-secure-enclave
[doc-cryptokit-keychain]: https://developer.apple.com/documentation/cryptokit/storing-cryptokit-keys-in-the-keychain
[doc-securetransport]: https://developer.apple.com/documentation/security/secure-transport
[doc-urlsession]: https://developer.apple.com/documentation/foundation/urlsession
[doc-timeout]: https://developer.apple.com/documentation/foundation/urlsessionconfiguration/timeoutintervalforrequest
[doc-ats]: https://developer.apple.com/documentation/security/preventing-insecure-network-connections
[doc-ats-local]: https://developer.apple.com/documentation/bundleresources/information-property-list/nsapptransportsecurity/nsallowslocalnetworking
[doc-localnetwork]: https://developer.apple.com/documentation/bundleresources/information-property-list/nslocalnetworkusagedescription
[doc-tunnel]: https://developer.apple.com/documentation/networkextension/nepackettunnelprovider
[tn3179]: https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy
[tn3151]: https://developer.apple.com/documentation/technotes/tn3151-choosing-the-right-networking-api
[tn2277]: https://developer.apple.com/library/archive/technotes/tn2277/_index.html
[forum-se]: https://developer.apple.com/forums/thread/744854
[forum-verify]: https://developer.apple.com/forums/thread/104018
[forum-keyusage]: https://developer.apple.com/forums/thread/780430
[forum-ats]: https://developer.apple.com/forums/thread/747421
[forum-suspend]: https://developer.apple.com/forums/thread/117150
[forum-bgtime]: https://developer.apple.com/forums/thread/85066
[forum-bgconn]: https://developer.apple.com/forums/thread/97824
[libsodium-points]: https://doc.libsodium.org/advanced/point-arithmetic
[swift-sodium]: https://github.com/jedisct1/swift-sodium
[blake3]: https://github.com/BLAKE3-team/BLAKE3/tree/master/c
[ts-othervpn]: https://tailscale.com/docs/reference/faq/other-vpns
[libtailscale]: https://github.com/tailscale/libtailscale
