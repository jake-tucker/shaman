# Let a thin Client stand at a Relay by an ordinary Login

_Extends ADR-0077, which left how a thin Client stands at a Relay to its own decision. Leaves ADR-0045, ADR-0046 and ADR-0048 as they are: a Relay carries bytes and holds no trust, admits by login, and a Login stands until removed._

A Relay joins two connections only where both hold Logins under one Account (ADR-0046), the dialing side as much as the waiting one, so a thin Client that is to reach a Remote through a Relay needs standing there. It holds a Login exactly as a Server does: formed by the same device login, tied to its identity key and proven by that key each time it connects, under the name the thin Client keeps for itself where a Server reports its hostname. The Relay is told nothing of which kind of holder it is, and nothing in the Relay or in what is said to it changes. This was chosen for the reason ADR-0077 gave of the Serving side: nothing a Relay could be told about a holder is anything it could check, and admission stays one rule. It also asks nothing of a Relay that is already released and held to ADR-0047.

A thin Client makes a Relay entry only by its user logging in at a Relay one of its Remotes offers, in an Invite or over the Pairing afterwards, and never by giving an address. Until then that Relay is listed with the Remote as needing a login and is not used, which is the rule a Server already keeps for a Relay it learns of. Pairing asks for the login only where no other way answers, so a phone beside its machine on one network pairs with no login at all. A thin Client does not ask a Relay which Logins stand under its Account: it pairs by scanning or pasting an Invite and has no short code, so a listed machine would lead it nowhere.

## Considered Options

- **A Login that says it only dials**, by a field added to the login, so a Relay could leave such Logins out of what it lists or mark them. Rejected for now: it is a Relay release and a migration for a label that rests on the holder's word, and a Relay from before it would list the phone all the same. It can be added later by addition (#649).
- **Standing by something other than a Login**, such as a paired Server vouching for the phone. Rejected: a Relay would admit on a Server's word, which is trust ADR-0046 keeps from it, and the phone's reach would hang on that Server.
- **Relay entries given by address**, as `shaman relay` makes them on a Server. Rejected: a Relay that none of a thin Client's Remotes is reached through is of no use to it, and an address is a poor thing to type on a phone.

## Consequences

- A thin Client's Login is listed to the Account's other machines (ADR-0064) as one that is not waiting, so `/pair` on each of them reads it as a machine that is not Serving and advises `/serve` there, which cannot be done on a phone (#649).
- It counts against the Account's cap on Logins. Its key never leaves the device, so a phone lost or replaced leaves a Login standing that only the Relay's operator can remove; it opens nothing, and ending what the phone could do remains the removal of the Peer on each Serving Server.
- The name a Relay holds is the one given at login. Renaming the phone afterwards changes what its Remotes know it by and not what the Relay lists.
- A phone logged in as another identity than its Remote's Server is refused by the Relay as under a different Account. Its direct ways are untouched.
- The connection a login is asked on must stay open and silent until the Relay answers, and a phone suspends an app that leaves the foreground. So the app sends its user to the identity provider without leaving the app, keeps the code where it can be approved from another device, and gives the login up if it is left.
