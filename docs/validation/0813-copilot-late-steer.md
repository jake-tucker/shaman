# What becomes of a steer Copilot accepts as its Turn ends

Seen for [#813](https://github.com/jake-tucker/shaman/issues/813), on `14b786c1`, macOS 27.2 on arm64, 2026-11-28, against GitHub Copilot CLI **1.0.117** through the pinned `copilot-sdk` **2.0.1**. The Model was `gpt-6-luna`.

Copilot accepts it and begins a run of its own for it once its loop has gone idle. Shaman has settled the Turn on that idle, so the Prompt reads `delivered` in the settled Turn, the run's output is shown nowhere, and an approval the run asks for is never offered. This was reached with nothing in the way, 17 times in 63 tries aimed at the Turn's last milliseconds.

## How it was seen

Two probes, neither a test and neither for `main`.

`examples/copilot_late_steer_capture.rs` drives the CLI through the SDK as Shaman's Provider does, asks for one word (`ALPHA`), and sends a second message as `immediate`, which is Shaman's steer, at a chosen event. It records every `session.event` beside the milliseconds since the first send. The captures in [`0813-copilot-late-steer/captures/`](0813-copilot-late-steer/captures) are cut down to the events that say what the loop did, with nothing of the account or machine in them.

`examples/copilot_late_steer_live.rs` hosts a real Shaman Server on the Copilot Provider, begins a Session with the same Prompt, admits a steer a chosen time after the answer shows, waits, sends one more Prompt (`CHARLIE`), and writes what the Session read at each step. Its reports are in [`0813-copilot-late-steer/live/`](0813-copilot-late-steer/live), each beside a log of the frames that crossed Shaman's wire to the CLI.

That log comes from `examples/support/copilot_idle_hold.py`, which stands where the CLI does and relays both ways. With `PROBE_HOLD_MS` set it holds back the CLI's first `assistant.idle`, and everything behind it, for that long. What Shaman sends still reaches the CLI at once. That is the order of the race, made wide enough to hit every time: Copilot has stopped, and Shaman has not yet heard. With `PROBE_HOLD_MS=0` it only logs.

Run them again from an empty directory with:

```
CAPTURE_MODEL=gpt-6-luna copilot_late_steer_capture <none|mid|final_start|message|turn_end|idle> [ms]
PROBE_813=1 SHAMAN_COPILOT_PATH=examples/support/copilot_idle_hold.py COPILOT_REAL=copilot \
  PROBE_HOLD_MS=4000 PROBE_FRAME_LOG=frames.jsonl copilot_late_steer_live 1000
```

## What Copilot does with the message

Seven captures, one each. Every send was answered within 4 ms with a message id, whenever it was sent. Copilot later writes a `user.message` carrying that same `messageId` at the moment it takes the message up, and says how in `delivery`.

| The message was sent | `delivery` | What Copilot did |
| --- | --- | --- |
| while a tool ran (the control) | `steering` | Took it into the running loop at the next model call, under the same `interactionId` |
| as the final answer began to stream, 250 ms before its `assistant.message` | `queued` | Finished the answer, then ran the message as a run of its own, with a new `interactionId`. **No idle between the two**: one `assistant.idle` and one `session.idle`, at the end |
| on reading the final `assistant.message` | `idle` | Went idle 2 ms after the send, then began a run for the message 17 ms later and went idle again after it |
| on reading `assistant.turn_end` | `idle` | The same |
| on reading `session.idle` | `idle` | The same |
| 500 ms after `session.idle` | `idle` | The same |

So Copilot never refuses a late steer and never loses one. What changes is whether an idle stands between the loop the steer was written for and the run that answers it. The line is the loop's final `assistant.message`: sent before it, the message is `queued` and run with no idle between; sent from it on, the loop reports idle first.

`assistant.idle` and `session.idle` come as a pair, the first at most a millisecond ahead. Across 40 runs they followed the loop's final `assistant.message` by 2 to 12 ms, and by 4 or 5 ms in 28 of them.

## What Shaman shows

Shaman settles a Copilot Turn on whichever of `assistant.idle` and `session.idle` it reads first. Its adapter refuses to send a steer once that has happened (`require_running_turn` in `src/provider/copilot/session.rs`), and sends it as `immediate` otherwise. Nothing it does afterwards looks at how Copilot took the message.

| Case | The steer | What Copilot was sent | What the Session read |
| --- | --- | --- | --- |
| [`streaming`](0813-copilot-late-steer/live/streaming.json) | As the answer's first text showed | `immediate`; took it as `queued` | `BRAVO` in Turn 1, after the steer's Message. Nothing lost |
| [`session-idle-held`](0813-copilot-late-steer/live/session-idle-held.json) | A second after the answer, with `session.idle` alone held back | a plain send, not `immediate` | Turn 1 had settled on `assistant.idle`. The steer began Turn 2, and `BRAVO` is in it. Nothing lost |
| [`held-word`](0813-copilot-late-steer/live/held-word.json) | A second after the answer, idle held 4 s | `immediate`; took it as `idle` and answered `BRAVO` | **Lost.** The steer reads `delivered`, taken in Turn 1, a user Message after `ALPHA`. Turn 1 is `completed` and the Session `idle`. `BRAVO` is nowhere, twenty seconds on and after the next Turn. `CHARLIE` is asked and answered in Turn 2 as if nothing had happened |
| [`held-file`](0813-copilot-late-steer/live/held-file.json) | The same, asking for a command | `immediate`; took it as `idle` and asked permission for the command | **Lost, and the Session wedged.** No Approval is offered and the Session reads `idle` with nothing pending, while Copilot waits on the permission. `CHARLIE`, sent twenty seconds later, begins Turn 2, which Copilot queues behind the run that is waiting. Turn 2 was still `active` with nothing in it 105 s later |
| [`held-file-follow`](0813-copilot-late-steer/live/held-file-follow.json) | The same, with `CHARLIE` sent 7 s after the steer | the same | **Lost, and misfiled.** `CHARLIE` began Turn 2 before Copilot asked permission, so the Approval for the steer's command is offered inside Turn 2, under a Prompt that asked for one word. Unanswered, Turn 2 was still `active` 105 s later |

In both wedged cases the reader's interrupt freed the Session: Copilot cancelled the permission, the Turn read `interrupted`, and the next Prompt (`DELTA`) was asked and answered. `CHARLIE` was never answered: the abort dropped it from Copilot's queue. The command never ran in either.

The unseen exchange stays in Copilot's own conversation. The Agent goes on having read a Message, and written an answer, that the reader was never shown.

## How wide the stretch is

With nothing held back (`PROBE_HOLD_MS=0`, the shim only logging):

- **Twelve tries** admitted the steer 0 to 5 ms after the answer's Message read `completed`, which is as soon as a client can know the answer is finished. All twelve were too late: Shaman had settled Turn 1, and the steer began Turn 2. They reached the CLI 12 to 22 ms after its final `assistant.message`. ([`on-completed.txt`](0813-copilot-late-steer/on-completed.txt))
- **Sixty-three tries** admitted it blind, 236 to 256 ms after the answer's first text showed, three at each millisecond, since this Model's one-word answer ends about 250 ms after it starts. ([`natural-race.txt`](0813-copilot-late-steer/natural-race.txt))

| Outcome of the 63 | Tries | The steer reached the CLI |
| --- | --- | --- |
| Run inside Turn 1 (`queued`) | 23 | 21 to 0 ms before the final `assistant.message` |
| **Lost** (`immediate`, taken as `idle`, `BRAVO` nowhere) | 17 | from 2 ms before that message to 7 ms after it, and from 7 ms before the idle to 3 ms after it |
| Began Turn 2 | 22 | after Shaman had settled Turn 1 |
| Never sent to Copilot | 1 | reads `delivered` in Turn 1 with no Turn having taken it, which is the case [#811](https://github.com/jake-tucker/shaman/issues/811) settled |

In the 17, Copilot wrote the steer's `user.message` 18 to 51 ms after the idle that settled Turn 1, and its run went idle 1.2 to 2.6 s after that.

So the stretch is about ten milliseconds: from Copilot's final `assistant.message` until Shaman's adapter has read the idle that follows it, which is the 2 to 12 ms Copilot takes plus a few of Shaman's own. Seven of the 17 were sent after the idle had already left the CLI. Twenty-one of these ran at once on one machine, which will have widened Shaman's share.

Nobody can aim at ten milliseconds by hand, and no client can either: the only sign the answer is ending is the Message reading `completed`, and by then it is too late. A reader hits it by chance, and the habit that invites it is the one the composer is built for, pressing Return as the Agent finishes. For someone whose press falls within a second either side of the end, that is about one press in two hundred.

## Whether Claude or Codex has the like

Neither, as read from source and from earlier captures, and not run again here.

- **Codex** names the Turn a steer is for (`expected_turn_id` in `turn/steer`, `src/provider/codex/session.rs`) and refuses one for a Turn that has ended. The steer stays waiting, which #811 settled.
- **Claude** begins a loop of its own for a message still queued when its loop ends, as Copilot does. But it reports each message's fate under the message's own id (`command_lifecycle`, `docs/validation/0407-claude-folded-steer.md`), and Shaman's adapter keeps the Turn open while a message written into it still waits for a loop to take it up, so that loop runs in the Turn the steer joined and settles it (`src/provider/claude/turn_in_flight.rs`).

Copilot gives Shaman the same knowledge and Shaman does not use it: the send's answer names the message, and the `user.message` that takes it up names it again.

## What the source says beside this

- `steer_turn` (`src/provider/copilot/session.rs`) checks that a Turn is running, then expands Skill Invocations, then sends. The check and the send are not one step, so an idle read between them settles the Turn under a steer already on its way.
- `open_main_turn` (`src/provider/copilot/projection.rs`) refuses main output arriving with no Turn open unless a Continuation is owed (`owes_late_output`), which it is only while a Subagent works on, a Watch is live or one has just settled. A steer's run is none of these, so every event of it is dropped, its permission request among them.
- The adapter already holds a stretch open past its loop's idle for one reason: Copilot is still compacting (`turn.idled`, ADR 0042). `steer_delivering` is where a steer meets that.
