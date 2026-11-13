# Let a Client with no Server of its own pair as an ordinary Peer

_Extended by ADR-0079, which decides how a thin Client stands at a Relay._

_Amends ADR-0017's clause that a Client never dials a Remote itself, for a thin Client alone. Leaves ADR-0045 and ADR-0046 as they are: a Relay still carries bytes and holds no trust, and a Pairing is still formed only by redeeming an Invite._

A phone runs no Server, so the app on it cannot see anything remote "through its own Server" as ADR-0017 has every Client do. Such a Client, a **thin Client**, holds an identity key of its own, redeems an Invite itself, and stands as the redeeming side of an ordinary Pairing: every Server it knows is a Remote, and each of them holds it as a Peer, by key and by the name it gave itself. Nothing about the Pairing is new. The Serving side is not told which kind redeemed, gives a thin Client what it gives any Peer, everything short of its administration, and removes it the same way. This was chosen because the trust a phone needs is exactly what a Pairing already is — an Invite spent once, two pinned keys, removal from either side — and because nothing the Serving side could be told about its Peer is anything it could check.

A Client is thin only where its device has no Server. On a machine that has one it attaches to it as the TUI does, so a device never holds two identities or two sets of Pairings, and a later desktop app is one kind or the other by where it is installed.

## Considered Options

- **A relationship of its own beside Pairing.** Rejected: it would repeat enrollment, removal, the Relay path and the Peer list to hold a distinction that rests on the redeemer's word. A hostile redeemer says whichever it likes.
- **A kind declared at enrollment and shown as a hint.** Rejected for now: the name already tells a reader what the entry is, and nothing acts on a kind. Whatever first needs one, push notifications most likely, can add it then.
- **A narrower set of routes for a thin Client.** Rejected: it too would rest on the redeemer saying what it is. A wider one, reaching administration, would end "a Remote shares its work, not its administration".
- **One Server as a hub, through which a thin Client reaches that Server's Remotes.** Rejected: a Pairing shares one Server's own work, the far Server never agreed to trust the phone, and everything the phone sees would hang on one machine being awake. A thin Client pairs with each Server it is to see.
- **The Serving user naming the Peer.** Rejected for now as new Server work a phone does not need: iOS gives an app no name for its device but the model, so the app keeps a name its user can change and sends that.

## Consequences

- A Server needs no change for a thin Client to pair with it and work in it. At the time of writing nothing in redemption or the proxied API checks that the redeemer is a Server; the offer to pair back, the peer hostname, offered Relays and a Sidekick's author are each optional.
- A phone that is lost holds what any Peer holds, which includes Agents running commands on each of its Remotes. Removing the Peer on each Serving Server ends it, and where a thin Client keeps its key is a decision of its own.
- "Nothing reaches through" is the model and was not yet the code when this was written: a Peer's request for `/v1/remotes/…` was carried on to the Serving Server's Remotes (#646).
- A thin Client has no Server to turn back to. With no Remote it has no Outlook, and where the Pairing under its Outlook ends it turns to the Remote last looked into before, or to none.
- Only a thin Client keeps trying an Unreachable Remote, and only while its user has it open, so nothing follows a Remote on its behalf while it is closed.
- A Relay admits by a Login, which is defined as a Server's standing and listed to the Account's other machines by hostname and by whether it waits to be reached. How a thin Client stands at a Relay is left to its own decision.
- ADR-0017 refuses a Pairing whose protocol versions differ. A phone's app and a Server are updated apart, so how far that rule relaxes is also a decision of its own.
