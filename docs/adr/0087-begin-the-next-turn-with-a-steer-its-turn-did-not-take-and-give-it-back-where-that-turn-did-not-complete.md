# Begin the next Turn with a steer its Turn did not take, and give it back where that Turn did not complete

A Prompt sent to steer a working Turn can still be waiting when that Turn settles: its Provider refused it, the connection was lost under it, or the Turn ended before the Provider was asked, which is a reader pressing Return as the Agent finishes. Shaman marked every such Prompt delivered as the Turn settled, so it stood in the Transcript as a user Message its Agent never saw and nothing followed for it. We decided that a waiting steer is never recorded in a Turn that did not take it. Where its Turn completes, it begins the next Turn ahead of every queued Prompt, and any others waiting with it steer that Turn in the order they were written. Where its Turn fails or is interrupted, it is withdrawn in the commit that settles the Turn, and its text returns to the composer of the client that wrote it.

A steer that begins the next Turn does what the same Prompt sent a moment later would have done, and it goes before the queue because its writer chose to steer and not to queue. One withdrawn was written for work under way that did not finish, so its writer chooses again. This is the rule ADR 0041 already has for a steer held behind a requested Compaction, now said of every steer its Turn could not take.

A steer its Provider refuses while the Turn works on stays a waiting steer until the Turn ends, and then follows the same rule. Shaman does not ask the Provider again. The Prompt says that its Provider refused it, and from then its writer may take it back, which the Session refused of every steer before. One the Provider is being asked about at that moment cannot be taken back, since the Agent may already have it.

## Considered Options

Joining the queue in the order it was written was rejected: Prompts queued earlier would run before a Prompt its writer meant the Agent to read at once.

Beginning the next Turn however the Turn ended, as a queued Prompt does, was rejected. Queueing says "after this", whatever becomes of this; a steer says "into this", and after an interrupt it would start the Agent again with words written for the work the reader had just stopped.

Returning a refused steer to the queue at once was rejected. It would put it on a card with every act a queued Prompt has, but it loses its place ahead of the queue, and a change of delivery from steer to queue is one a client's projection refuses today. Withdrawing a refused steer at once was rejected because the commonest refusal is Codex answering "no active turn" as the Agent finishes: that is the same race as never being asked, and it should end the same way, not with the text thrown back at its writer under an Error. Asking the Provider again at the Turn's next chance was rejected as machinery for a case the writer settles in two presses, by taking the Prompt back and sending it again.

## Consequences

After an interrupt a queued Prompt still begins the next Turn while a waiting steer comes back to its writer. That is deliberate, and follows from what each delivery says.

The text goes to whoever wrote it and not, as ADR 0024 has it for a Prompt an interrupt withdraws, to whoever interrupted: the interrupt stopped a Turn and asked for no Prompt back, and a failure has no interrupter at all. The Prompt carries the reason, as one withdrawn behind an unfinished Compaction does, so the client that wrote it need not have watched it wait. A client that does not know the reason shows the Prompt cancelled and gives nothing back, and a Sidekick that wrote one is told nothing of the withdrawal, as ADR 0073 has it for a held Prompt.

ADR 0024 has every client draw a pending steer in the Transcript's position as the user Message it will become. That now holds only until its Provider refuses it. A refused steer is no longer a Message of the working Turn but a Prompt waiting for that Turn to end, so a client draws it with the queued Prompts, first among them, where its writer can take it back. The Error saying why it was refused stays in the Turn where it was refused.

A Turn that fails because its Provider's connection was lost withdraws its waiting steers as any failed Turn does. Before, such a steer stayed pending on an idle Session and was recorded as delivered in whichever Turn settled next.

Both additions to what a Session says of a Prompt, the reason for the withdrawal and the mark that its Provider refused it, are optional, and nothing is removed. Toward a Remote whose Server has not taken this change a steer still reads delivered in a Turn that never took it, and a client cannot tell such a Remote apart.

A steer its Provider accepted is outside this decision, whatever the Provider then does with it.
