# Release the Relay on WebSocket alone, and leave QUIC to be added beside it

A mobile client is expected to reach its Servers either directly, over a network such as Tailscale, or through a Relay. On a phone, QUIC and HTTP/3 would help in two ways: a connection could survive the phone changing networks, and on a lossy link one lost packet would stall only its own stream rather than everything carried with it. ADR-0047 fixes what a Server and a Relay say to each other once the Relay is released, so whether that first release should already speak QUIC was weighed before it. It need not: the Relay is released speaking only a WebSocket at `/connect`, over HTTP/1.1 and TCP, and QUIC is left to be added later beside the WebSocket, never in its place.

Nothing in version 1 stops that addition. A transport is chosen before any version is agreed, so the WebSocket at `/connect` has to stay for as long as any Server or Relay still speaks only it. But a later Relay can say that it also accepts QUIC, through `Alt-Svc` on the response that accepts the WebSocket or through a field a later version adds to `Challenge`. A Server that understands that can try QUIC first and fall back. Neither an older Server nor an older Relay ever sees a difference, and that is the change by addition ADR-0047 allows.

## Considered Options

- **Speak QUIC from the first release.** Rejected. The Rust HTTP/3 stack is still experimental, UDP is the traffic a restrictive network is likeliest to refuse (the WebSocket would be kept as the fallback anyway), and every operator would have to open UDP and put in a reverse proxy that forwards it. All of that would be paid before any mobile client exists to gain from it.
- **Change the version 1 handshake now, so that a Relay names its transports.** Rejected: `Alt-Svc`, and the protocol's tolerance of unknown fields, already give a later Relay a way to say so without breaking anyone.

## Consequences

- QUIC can be added at two depths. Carrying each join on a QUIC stream between each Server and the Relay gains surviving a network change and reconnecting faster, but the end-to-end TLS, with HTTP/2 inside it, is still one ordered stream, so a lost packet still stalls the whole join. Only QUIC run end to end, with the Relay forwarding packets it cannot read (still the byte carrier of ADR-0045), lets each stream lose a packet on its own. That is a new kind of join, introduced in a version both sides agree.
- Because what a Server and a Relay say tolerates fields it does not recognize, an older Relay silently ignores a new field on a message it already knows. New behaviour therefore arrives as a new message, which an older Relay refuses as `Unexpected`, or only under a version both sides have agreed, never as a field on an existing message whose meaning changes when it is ignored.
- A phone that reaches its Servers over Tailscale already keeps its address as it changes networks, so QUIC would gain it little more than per-stream recovery from loss. Whether to add QUIC is decided from what a mobile client is actually seen to suffer, not in advance.
