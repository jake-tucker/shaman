# Offer to pair back by a standing record, and a token bound to one key

_Leaves ADR-0017's one-way Pairing as it is. Extends it: the ceremony that forms one Pairing may offer the opposite one, which comes to be an ordinary second Pairing of its own, formed by redeeming an Invite (ADR-0046)._

A Pairing is one-way, so two machines that should reach each other take two ceremonies. `A` is the machine whose user pairs, redeeming `B`'s Invite: `A` holds a Remote of `B`, and `B` a Peer for `A`. **Configure Remote** on `A` offers *Also let `B` reach this machine*, off by default, since a Pairing lets the other machine run commands here. The second Pairing would have `B` hold a Remote of `A`, and `A` a Peer for `B`. `B` redeems `A`'s Invite.

`B` is commonly unattended and an Invite lives ten minutes, so the offer is no Invite. It is a **standing record**, carrying no token, held by both Servers across restarts until `B`'s user answers. `A` issues an Invite only once `B` says its user accepted, on a connection `A` has open to `B`, and sends it over the Pairing. `B` redeems it at once.

## An offer is made by the enrollment commit, and nothing else

"A Peer can make an offer only at the end of the ceremony that formed it" has to be a fact `B` can check. The way to make it one is to have nothing to check: **the offer's id travels in the enrollment's commit**, the very request that forms the Pairing, under the token `B`'s user issued. `B` writes the Peer and its offer in one write. There is no route by which a Peer makes an offer, so however cleverly timed, a Peer whose ceremony is over has no way to make, replace or remake one: not after a decline, not after a restart, not ever, short of `B`'s user giving it another Invite. That would be another ceremony and another Pairing (a name paired again is one), which replaces whatever stood.

This is why there is no offer window. A window opened by enrollment and closed by first use, a decline or a time bound would admit a late first offer, which a hostile `A` could hold back and use later, for a price paid only in the bound's length. The commit needs no window, and it has no first-offer-lost-in-transit case: a commit that does not arrive forms no Pairing, and redeeming fails as it always could. So "records an offer on `A` and sends it to `B`" is one act, with `A`'s record written before the commit goes (a crash then can leave `A` a record, never `B` one `A` does not know of).

**The record lives on the Pairing's own record.** `B`'s offer is a field of its stored Peer, and `A`'s a field of its stored Remote, both written whole through the existing atomic replace of `peers.json` and `remotes.json`. So a Peer holds one at most by its type, a removed Peer's record goes in the same write, and no separate file can strand one.

**What `B` holds of an offer**: its id, when `B` took it, by `B`'s clock, how it stands (waiting, accepted under the name `B`'s user gave it, or being redeemed under that name), and how its Notice stands (untold, given to one Client run, or passed by an answer that came first). Each is one value rather than flags that could disagree. The name can change while it waits and not while it is being redeemed: the Remote that comes of a redemption goes by the name it began under, and is known by it should `B` stop midway. There is no text from `A` in it. `B` names the offer by the name the Peer is already known by, held to what can be shown as it enrolled, with its fingerprint beside it, and the Remote's offered name comes from that through `shown`. A hostile `A` gets to choose nothing a reader sees that it could not already say by being a Peer. Each Peer is one record of bounded size.

## Who holds what, and every transition

`T` is the bound token, alive only in `A`'s memory and only for one delivery. `—` is nothing held. A crash on either side at any line is read off the "survives" column.

| # | Step | `A` holds | `B` holds | Trigger | A crash here |
|---|---|---|---|---|---|
| 1 | Configure Remote, option on | Remote, offer | — | user at `A` presses Enter; `A` writes before the commit | commit never sent: Remote and offer on `A` with no Peer on `B`. The Remote reads Unavailable until its user removes it, which takes the offer. `B` holds nothing |
| 2 | Commit lands | Remote, offer | Peer, offer (untold) | `B` writes Peer and offer in one write | both records stand. `A` stores nothing further, so a crash before the answer costs nothing but the answer: a commit `A` could not hear the answer to rolls its Remote back, as ever (`B` keeps a Peer and offer `A` no longer knows; see *stranded*, below) |
| 3 | Notice given | same | offer told to one Client run | first Client that can show it claims it | stored as given; never raised again |
| 4 | `B`'s user answers: **decline** | Remote, offer | Peer | local route on `B` | `B`'s record is gone. On `A`'s next connection `B` tells no offer, and `A` sweeps its own |
| 5 | `B`'s user answers: **accept**, naming the Remote | same | Peer, offer accepted | local route on `B` | stored; `B` says it is waiting on `A` |
| 6 | `A` connected, or next connects | same | same | `B` tells `A` the standing of `A`'s own offer over the stream it already tells its Relays on, at once and on each change | nothing was stored |
| 7a | `A` withdrawn, or the offer unknown to `A` | offer marked withdrawn | same | `A` says withdrawn; both clear | either side repeats it safely |
| 7b | `A` has no way to offer | same | accepted, held up by why | `A` says why as a closed set of reasons, never text | why is `B`'s memory alone, said again next connection. The offer stands |
| 7c | `A` has a way | + `T` | same | `A` issues `T`, bound to `B`'s key, and sends the Invite | `T` dies with `A`. `B`'s redemption fails, and step 6 comes again |
| 8 | `B` takes the Invite | + `T` | being redeemed | `B` checks id, acceptance, the bound on tries, that the key is `A`'s and how many ways it offers, then stores that it is being redeemed and spawns the redemption | at restart `B` takes back any Remote of `A`'s key under that name, and the offer is accepted again: step 6 comes again |
| 9 | `B` redeems | + `T`, Peer of `B` | + Remote of `A` | ordinary enrollment, `T` in `B`'s certificate | as 8. Enrolling again on `A` replaces the one Peer record, so there is never a duplicate |
| 10 | Redemption done | Peer of `B` | Remote of `A`, no offer | `B` clears its record in its own write, answers formed | `A` hearing nothing retires `T`, and on its next connection `B` tells no offer: `A` sweeps |
| 11 | Formed | Peer of `B`; offer gone | Remote of `A`; offer gone | `A` hears formed, or next told no offer | both records are gone; the result is two ordinary Pairings, each ended on its own |

**Exactly once.** `A` clears only on `B`'s word (formed, or no offer told), and `B` clears only as its Remote stands or its user says so. A redemption is one at a time, guarded by that stored standing, and its end holds whether or not it can be stored, so a failed write cannot leave the offer waiting on a redemption long over: the next write of the Peers stores it. Should `B` also stop before that write, it finds a redemption caught midway, as it would have anyway. A second delivery finds no record and is refused. A delivery lost after `B` redeemed leaves `A` a record that the next telling sweeps; a delivery lost before leaves `B` accepted, and a fresh token goes the next time. Neither a dropped connection nor a crash on either side at any step can form two Remotes: `B`'s record is what is redeemed against, it is cleared when the Remote stands, and at restart a redemption caught midway is taken back first. Enrolling twice on `A` replaces its one Peer.

**Pairing by hand meanwhile.** Should `B`'s user redeem an Invite of `A`'s by hand while a redemption is under way, that Pairing ends the offer, and what the redemption then leaves goes on `B` alone. Asking `A` to withdraw would drop its one Peer for `B`'s key, which the Remote made by hand reaches it by. A decline meanwhile, with nothing else of that key on `B`, does tell `A`.

**Withdrawal.** `A`'s user withdraws in `/connect`. `A` stores it, kills any `T` for that key, and from then on issues nothing for that offer. It reaches `B` at once where a connection stands, and otherwise on `A`'s next one. If `B` accepted meanwhile, ordering is decided at `A`: no Invite. If `T` is already in flight, killing it makes `B`'s redemption fail, unless its commit already landed. Then withdrawal lost the race: the second Pairing stands as an ordinary Peer on `A`, listed in `/serve`, and `A`'s user removes it as any other.

**Ending the original Pairing ends the offer on both.** `A` removing the Remote takes `A`'s record in that write, and `B`'s Peer and record go with it where `B` answers the withdrawal. `B` removing the Peer takes `B`'s record at once, and `A` sweeps its own when next refused, by a `401` from `B`'s pinned key. A key mismatch alone sweeps nothing, since a Relay in the path can cause one.

*Stranded.* `B` cannot call `A`, and `A`'s removal may not reach `B`: then `B` holds a Peer and an offer that nothing will ever answer, as it already holds the Peer. It is never hidden: it stands in `/connect` saying how long it has stood and that it waits on `A`, and `B`'s user can decline it or remove the Peer at any time. `A` holding a record `B` does not: swept by the next telling. `B` holding one `A` does not: `A` says withdrawn. Nothing is left to wait forever without a reader being able to see and end it.

## The bound token, and its exception to the supersede rule

Issuing an Invite supersedes every outstanding one, because a shown Invite may leak through scrollback. A token here is shown nowhere, so it neither supersedes nor is superseded. It is a ledger entry with `bound_to` set to `B`'s key.

- **Redeemable only by that key, proven by TLS.** The token is accepted in the handshake only with a certificate whose key is `bound_to`, and `enroll` checks the same key against what the connection proved, for the case of a key that is enrolled and so passes the handshake anyway. A try from any other key is refused exactly as an unknown token is, changes nothing, and **cannot burn it**.
- **Short-lived.** It lives for one delivery, `pair_back_invite_ttl` at the most, and is retired when the delivery ends however it ended. At most one is alive for a key: issuing kills the last. Serving stopping, or its listener moving, discards it with every other.
- **Touches no short code, shows on no surface.** It has none, spends none, and `/serve` never sees it. A Peer asking anything learns nothing of `A`'s other Invites, and nothing of it reaches a Log (ADR-0008). `InviteText` carries it, saying nothing in `Debug`.

`B` refuses an Invite unless it holds an accepted offer of that id from the Peer the connection proved, and the Invite names that Peer's own key. So `B` never redeems an unsolicited Invite, and a hostile `A` cannot offer `B` a Pairing to a different identity. Otherwise it is redeemed like any other, over the same enrollment over TLS 1.3, so nothing here forms trust on a new basis.

## The Peer-facing surface

Every other route this work adds is server administration on the loopback listener, and a Peer's proxied attempt at any of them is refused as such. This is the one deliberate exception, since `A` talks to `B` as a Peer. It is exactly two things, and each is scoped to the calling key:

1. The stream of `OFFERED_RELAYS_PATH`, which a Peer already holds open, also tells it its own offer's id and whether it is accepted. Nothing about any other Peer's offers can be said on it.
2. `POST /v1/pairing/offer` takes one of three words about the caller's own offer: *withdrawn*, *cannot offer* (a reason from a closed set), and the *Invite*. It can make nothing, name no other Peer, and a word about an id that is not the caller's is answered as no offer.

What a Peer can make `B` do is bound. One redemption is under way at once for each offer, with `pair_back_retry_interval` between one that failed and the next, on `B`'s side as well as `A`'s. An Invite offering more ways than a Server sends, 16 addresses and 16 Relays, is refused undialled, and a redemption is held to `pair_back_invite_ttl`, so a machine that takes the handshake and falls silent holds nothing up, and what it left is taken back: a hostile `A` cannot have `B` dial at will. Why an offer cannot be met changes no faster than that interval, since each change is told to every Client of `B`'s, and it is kept in `B`'s memory, so a Peer has no way to spend `B`'s disk.

**What a Peer has reach a reader** is the name it is known by, as it enrolled, and nothing else. Why a redemption failed is one of a closed set in `B`'s own words, for the reason a message in the offer was rejected: `A` writes the refusal `B` hears, and names the Relay `B` may be told to log in at. A refusal's code is `A`'s word as well, so what `A` answers is said as a refusal and no more, and the specific words are for what `B` judged for itself: a code of `A`'s choosing could otherwise send a reader to `/relay`, or round a rename, for nothing.

The proxy's guard judges a path as it means, percent decoded, in lower case, with empty and dot segments resolved, so what it refuses does not rest on the router declining to read `%70airing`, `/V1/` or `//`. It decodes three times, and refuses a path still encoded past that: each layer costs a pass of the whole path, and no route needs them.

These routes live on the Serving listener, not under `/v1/pairing/proxy`, and only `A`'s Server speaks them.

**A Sidekick has no way in.** Making, accepting and withdrawing are each local administration, offered by no Tool, and what a Sidekick sends a Remote goes through the proxy, which refuses `/v1/pairing/` as administration. A Sidekick keeping a Remote in view can cause a connection, and so can cause `A` to settle what `A`'s user already chose and `B`'s user already answered, as any Client's looking can. That decides nothing: the two decisions were each a reader's, on their own machine.

## What is settled, and how a failure surfaces

`A` settles on being told, while it is connected, and when its own ways change: so `A`'s user turning Serving on for this needs no further step. `A` connects only while someone there has `/connect` open or looks at `B`, so `B`'s surface says that rather than leave its user waiting on a machine nobody will touch. Where it has no way, `A` issues nothing, says why, and the offer stands. Where `B`'s redemption fails, a stale address or a Relay `B` holds no Login at, `B`'s surface says what failed, in its own words, and `A` tries again no sooner than `pair_back_retry_interval`. Each look at `B` is owed one try, so one that comes inside that bound holds its connection until the bound passes, for no longer than the interval, rather than be lost: a look may be a probe that lets go at once, and may be the only one for weeks. Tries beyond that one are made only while something else keeps `B` in view. Nothing loops per connection.

The ways offered are everything `A` can offer, up to the cap above: its listener's address where it is bound to one another machine could reach, or its interfaces' addresses as `/serve` lists them, and every Relay `A` Serves through with a Login not known to need renewing. A listener on loopback is no way, as `/serve` never offers one, and reads as having none rather than as a stream of failed tries. This gives up the choice among ways `/serve` gives for each Invite, since `A` issues it after both users have decided, and the option on Configure Remote says so. Opening is not done for the user: turning Serving on, or Serving through a Relay, stays a separate, visible choice, made in the manner of `/serve`'s row actions. A Relay row says what Serving through it opens before its key acts.

## What stands in the way, on `B`

A Remote already holding `A`'s key on `B`, by `/serve` and a paste for instance, ends the offer as it forms and refuses an acceptance, so no duplicate comes of it. A Remote formed by this offer and removed later ends only that second Pairing.

## Versions

`PROTOCOL_VERSION`, the Pairing's own version, moves on with this, as it does with each change to what Servers say. A Pairing whose versions differ is already refused as a whole (ADR-0017): enrollment refuses at its preparation, and the stream telling Relays refuses a Peer speaking another version, so an offer is never asked of a Server that does not know it. A record simply waits through a version mismatch, and does nothing to the Pairing it rides on; a Remote still reads as `ProtocolMismatch` and nothing else. The commit's new field is said only when an offer is made, so leaving the option off changes nothing on the wire but the version.

## Considered Options

- **An offer window, as a Peer-facing route opened by enrollment.** Rejected as above: it asks `B` to check timing where the commit needs no check.
- **A separate store of offers.** Rejected: it would need its own sweep for every way a Pairing ends, and every sweep is a chance to strand one.
- **Issuing the Invite as the Configure Remote option is chosen.** Rejected: it would leave a live token on a machine nobody may visit for weeks, and an Invite issued on every connection would supersede one a user had just issued from `/serve`.
- **Unbound tokens exempt from superseding.** Rejected: an exemption with no binding would be a standing bearer credential.
- **A name or message in the offer.** Rejected: it would be text a Peer chose, on a reader's screen, for no gain over the Peer's name.
- **An expiry on the offer.** Rejected by the issue: it stands however long the answer takes. It is visible and revocable from both sides instead.

## Consequences

- `A`'s user must have a way to offer, or be told why not, to choose the option: its row on Configure Remote shows `[-]` with why in a word, and says why in full and offers to put it right once the keys come to it, so a reader pairing one way is asked to read little more. A frame with no room for what a fix opens says so, and the key does nothing. The Server refuses it as well, before spending anything, so nothing rests on what a Client showed.
- `B`'s user sees a Notice once and an entry in `/connect` until answered, on a machine they may not visit for weeks: nothing here can assume a reader on either side.
- An acceptance with `A` away waits, and `B` says so. A machine started at login with no Client attached connects to nothing, so an offer it made waits for a Client too.
- A second Pairing arising this way is ordinary in every respect: each is ended on its own, and removing one leaves the other standing.
