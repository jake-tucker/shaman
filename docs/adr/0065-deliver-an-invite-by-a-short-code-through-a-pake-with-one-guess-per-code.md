# Deliver an Invite by a short code, through a PAKE, with one guess for each code

_Extends ADR-0046, which left room for a convenience that ends "in an Invite's redemption or something as strong"._

A Pairing through a Relay asked for a long string to be carried between two machines, often with no good channel between them. Where a machine has been found at a Relay (ADR-0064), a reader may now type a short code that machine's `/serve` shows, and the same Invite is delivered to them. This is a **short Invite**: a second way of delivering an Invite and not a new way of forming trust. What it delivers is previewed, trusted and redeemed as any other, over the same TLS 1.3 transport with the enrollment token in the certificate (ADR-0017, ADR-0045).

## Why a short code is as strong as it asks

A code is eight characters from an alphabet of 32, the digits and capitals without `0`, `O`, `1` and `I`, shown in two groups as `K7QF-2MXP`. That is 32⁸ = 2⁴⁰ codes, so 40 bits, generated from the system's randomness with each byte's low five bits giving one character, which favours none since 256 is a multiple of 32. `0`, `O`, `1` and `I` are never shown, so a reader typing one is told to look again, with no guess spent.

**The offline-guess rule.** Nothing the code goes into may be searched offline. At a billion hashes a second, which one graphics card does, 2⁴⁰ candidates take about eighteen minutes, well inside the Invite's ten-minute lifetime on a few cards. So no hash of the code, and nothing else checkable against it, is ever sent; all the code does is enter a password-authenticated key exchange, which leaves each party a test of the one guess it put in, and only by being a party to the exchange.

**The one-guess rule.** Any failed exchange ends the code. So an attacker gets one try at a 2⁻⁴⁰ chance, about one in a trillion, and each further try takes a new code, which only a reader's asking for it makes, and which shows on their screen. A code lives no longer than its Invite, at most ten minutes, is superseded by the next Invite, and is dead once that Invite is spent. A full Invite's 256-bit token carries far more entropy, and is needed for that reason: it travels as a bearer string that anyone who sees it can test. A code can be tested only online, once.

The redeeming side keeps to the same rule: it makes one attempt for each thing its reader does, and never retries. A party standing in for the Serving Server gets one guess for each attempt, so the redeemer must make no extra attempts on its behalf.

## The exchange

A connection a Relay carried opens with the exchange or with TLS, told apart by its first byte. A TLS ClientHello's first byte says handshake, `0x16`; the exchange opens with `suru-short-invite`. The exchange travels in the bytes a join carries, which the Relay never reads, so the Relay protocol does not move (ADR-0047). A version byte follows the preamble, so the exchange can change later and another version is refused with nothing spent. The Serving listener never reads for it: a connection dialled there is TLS or nothing, and direct Pairing is untouched.

`R` is the redeeming Server and `S` the Serving one. `pA` and `pB` are SPAKE2's messages, `K_R` and `K_S` the identity keys:

```text
R → S  "suru-short-invite", version, K_R, pA
S → R  either: refused at once, naming why (no code live, too many at once, another version, malformed), and it ends
       or:     pB
R → S  MAC(k_redeemer, transcript)
S → R  either: failed, and it ends
       or:     MAC(k_serving, transcript), AEAD(k_invite, Invite, aad = transcript)
```

`R` is SPAKE2's side A and `S` its side B, fixed. SPAKE2 is given the canonical code as its password and `K_R` and `K_S` as its identities. Its output is the hash of `H(code)`, `H(K_R)`, `H(K_S)`, `pA`, `pB` and the shared point. The transcript is the SHA-256 of the label `suru short invite v1`, the Relay's address, `K_R`, `K_S`, `pA` and `pB`, each length-prefixed. HKDF-SHA-256, salted with the transcript, derives three keys from SPAKE2's output, under the info strings `suru short invite v1 redeemer proof`, `… serving proof` and `… invite`. The MACs are HMAC-SHA-256 and the AEAD is ChaCha20-Poly1305, its key sealing this one Invite under an all-zero nonce.

**What each message reveals.** SPAKE2's messages are uniformly random whatever the code is. What follows them depends on the shared point, and testing any guess against it but the one a party put into the exchange takes solving Diffie-Hellman.

- A passive Relay sees all of it and can test nothing, since it put no guess in.
- A party that guessed wrong as the redeemer is sent `pB` and then `failed`: not one byte derived from the key. It learns its guess was wrong, with the code by then spent. The Invite is sent only to a party that has proven the code, which is why the redeemer proves first.
- A party that guessed wrong as the Serving Server does get the redeemer's MAC, and can test it against the one guess it put into `pB`. That is the one guess for each attempt the redeemer's reader makes. The redeemer then refuses whatever comes next, since it cannot be proven.

Reflection fails: a message from one's own side is refused by SPAKE2's side byte, and the two MACs are keyed apart, so a redeemer's proof sent back is no proof.

**Where the guess is spent.** A code is spent as an exchange takes it, under the Invite ledger's lock, once the opening message has been read whole and before anything is answered. It is taken out of the ledger there. So two exchanges at once get one guess between them: the second is refused as finding no code, the first goes on undisturbed, and neither is told of the other. A party that hangs up or falls silent having sent its message has had its guess, since it could otherwise learn something for free; `/serve` says such an attempt never finished, and it costs what a wrong guess costs. A connection that closes before that has made no guess, and spends nothing. An exchange is closed within a bound, `short_invite_timeout`, and at most four are under way at once, past which one is refused, so they take up no more of the room TLS handshakes share. With no code live, an exchange is refused as soon as it has said it speaks the exchange, with no cryptography done and nothing held open.

**The redeemer's boundary.** Trying a machine's Relay ways in turn, it moves on only where nothing could have been guessed: a join that failed, with no exchange byte written, or a refusal at once as its first answer, before it has proven anything. Any other answer, or none, ends the attempt. A refusal can be forged by a Relay, which gains it nothing: `pA` tells it nothing, and only a proof could be tested against a guess.

## What the exchange binds, and what it does not believe

Both identity keys and the Relay's address enter every key, so an exchange's output is useless for any other pair. A Relay naming another key to the redeemer, carrying the exchange on to another Relay, or altering what either side says makes the two sides derive different keys, and it fails.

The Relay's address needs no agreeing on. Each Server names a Relay in the one way a Relay's address is written, and a Server answers only a challenge naming the address it knows the Relay at (`canonical_address`), so two Servers proving themselves at one Relay name it alike. Port or case differences are gone by then, and a Relay known by two names to its Servers already fails before any exchange begins. An exchange runs only over a Relay the Invite offers: one the reader left out carries no attempt, and is refused as having no code.

The exchange proves neither key's owner. The redeemer's `K_R` is what it claims, and the Serving Server's `K_S` is what the Relay listed. Neither signs anything, because nothing rests on it: holding the code is the credential here, and whoever holds it could fetch the Invite under any key it cared to name. The Pairing's TLS has each Server prove its own key afterwards, as for any Invite: the redeemer pins the key the Invite names, the Serving Server enrolls the key the redeemer's certificate proves. The delivered Invite is the ordinary one, its token a bearer token as ever, so it is not tied to `K_R`: tying it would leave the long string refused once anyone had fetched it, handing a party who burned a code a way to spoil the Invite.

The redeemer refuses a delivered Invite whose key is not the one listed, and says so. Both sides bound that key, so an honest pair always agree; where they do not, whoever delivered it knew the code, and the Relay's list was wrong or was lying. A delivered Invite's hostname is believed no more for being delivered, and reaches the preview as what can be shown.

## The crate

SPAKE2 from RustCrypto's `spake2` 0.4.0. It is a balanced PAKE, in pure Rust on `curve25519-dalek`, builds on all three platforms, and is what `magic-wormhole` uses. Its transcript hashes each identity on its own, with no ambiguous concatenation, and its messages carry their side. It has had no independent audit, as its README says, which is a risk accepted here on its small size and its wide use. The 0.5 line is still a pre-release on pre-release dependencies, so it was left until it is released. HKDF, HMAC, ChaCha20-Poly1305 and randomness come from `ring`, already built for `rustls` and `rcgen`. `spake2` keeps its own copy of the password, which `zeroize` cannot reach; Suru's own copies of the code and every derived key are wiped as they are dropped.

## Considered Options

- **Send the Invite encrypted with the PAKE's output in the second message, one round trip sooner.** Rejected, though it allows no offline search: it would hand the Invite to a party before it proved anything, and a wrong guesser a ciphertext to test its guess against rather than a refusal.
- **A hash of the code, or a code the Relay stores.** Rejected by the offline-guess rule.
- **CPace through `pake-cpace` or `pakery-cpace`.** Rejected: it would serve as well, but those are less used, and one has seen no release since 2023 while the other began in 2026.
- **OPAQUE through `opaque-ke`.** Rejected: an augmented PAKE, for a server holding a verifier for a client's long-lived password, where here both sides hold one short-lived code.
- **Approve or Deny on the Serving machine.** Rejected by the decisions behind ADR-0064: one careless press would give a phished Server what a Peer holds.
- **Normalizing `0` to `O` or `1` to `I`.** Rejected: neither is in the alphabet, so there is nothing to normalize to, and a reader who typed one has misread a character that is better looked at again than guessed at.

## Consequences

- A guess costs a new code. Griefing is accepted: a Server under the same Account, or the Relay itself, can burn a live code, which shows on the screen and gains nothing else. `/serve` says so, and any Client attached raises a Notice; the Invite stands, and one key gets a new code for it. That new code does not extend the Invite's lifetime.
- Burning a code does not touch the long string, so the two can be used together.
- A code is memory-only, as an Invite's token is: a Server that restarts has neither, and a Client attached is told. Nothing is stored, and no code or derived value reaches a Relay message, a Log (ADR-0008), a Relay's connection log, or any Client but the local one that issued it. The Relay's connection log gains a join for each exchange, as it does for any connection it carries.
- A code exists only where someone issued an Invite through a Client. Nothing issues one on its own, so a machine started at login (ADR-0063) refuses every attempt at once.
- Machines that reach a Relay by different addresses cannot use a short code, nor reach each other through it, for the reason the Relay's own challenge already gives.
