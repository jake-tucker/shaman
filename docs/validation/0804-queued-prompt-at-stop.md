# What becomes of a queued Prompt when its Server stops

Seen for [#804](https://github.com/jake-tucker/shaman/issues/804), on `982daad7`, macOS 27.2.1 on arm64, 2026-11-28.

## How it was seen

`tests/codex_integration/queued_prompt_at_stop.rs` is a probe, not a test. Each case:

1. starts a real `shaman __server` process against a scripted Codex that begins every Turn it is asked for and holds it until released;
2. begins a Session with Prompt **A**, whose Turn Codex is then working on;
3. admits Prompt **B** with `delivery: queue`, behind that Turn;
4. stops the Server one way;
5. starts a second `shaman __server` on the same state and data directories, reads the Session, leaves it alone for 1.5 seconds, and then does what a reader might: writes Prompt **C**, or cancels B, or interrupts, or promotes B ("Steer now").

What Codex was asked is read from the script's own log, so "delivered" below means Codex received a `turn/start` carrying the Prompt's text. The reports are in [`0804-queued-prompt-at-stop/`](0804-queued-prompt-at-stop), one a case.

Run it again with:

```
PROBE_804_OUT=<an existing directory> cargo nextest run --test integration -E 'test(/probe_804/)'
```

## The stops

| Stop | How it was made |
| --- | --- |
| Crash | `SIGKILL` to the Server process, 500 ms after B was admitted |
| Crash at admission | `SIGKILL` as soon as B's admission had answered `201` |
| Signal | `SIGTERM` |
| Manual stop | `POST /v1/server/stop` with `reason: manual` |
| Replacement | `POST /v1/server/stop` with `reason: replacement` |

A replacement was made as a Client's launcher makes one, by that request, and followed by a Server of the same build. A Server of another build was not run, and a Restart does not exist yet.

## What was seen

### Every stop leaves B pending, and nothing delivers it

After a crash, a signal, a manual stop and a replacement alike, the next Server reads:

- A's Turn `failed` ("The server stopped before this Turn finished."), though under the three graceful stops Codex had been asked to interrupt it and had answered;
- B still `pending` with `delivery: queue`;
- the Session `idle`, with no `working_since`.

Left alone, Codex was asked for nothing. During a graceful stop Codex was not asked to begin B either.

### B is delivered after the next Turn the reader begins

The reader writes C, by Return (`steer`, which an idle Session takes as a Turn of its own) or queued. Either way:

- C begins a Turn at once, ahead of B, which was admitted before it;
- when C's Turn settles, the Server asks Codex for a Turn carrying B.

So B runs out of order, behind a Prompt written after it and after the stop, however long ago B was written. The reader is told nothing: until then B reads as a queued Prompt on an idle Session.

### What a reader can do with B on the next Server

| Act | Answer | What follows |
| --- | --- | --- |
| Cancel (`/prompts/{id}/cancel`) | `200`, `cancelled` | B is gone from the queue. This works. |
| Interrupt (`/interrupt`) | `409 nothing_to_interrupt` | B stays pending. |
| Promote (`/prompts/{id}/promote`) | `200`, `delivery: steer`, still `pending` | Nothing is asked of Codex. |
| Promote, then write C | C begins a Turn | When C's Turn settles B reads `delivered` and stands in the Transcript as a user Message after C's answer. **Codex was never asked for it**: its log holds no `turn/steer` and no `turn/start` with B's text. |

The last is a Prompt lost while it reads as delivered.

### A crash at admission loses the Prompt altogether

Killed as soon as B's admission had answered, the next Server's Session holds no B at all: storage had not taken it. Where A's Turn had begun only moments before, the next Server did not hold that Turn either, and read A itself as a Prompt waiting for a Turn and withdrew it (`cancelled`, no Turn, revision 2). Where storage already held A's working Turn, A's Turn read `failed` and B was simply absent.

The storage writer saves on an idle tick (`IDLE_FLUSH_DELAY`, 100 ms), and an admission does not wait for it. Only "at once" and "500 ms later" were tried, so how wide the window is under a Turn that streams without pause was not measured.

### The control: a Prompt the reader was steering with

Admitted with `delivery: steer` behind A's working Turn, and never answered by this Codex, B reads `cancelled` on the next Server after a crash and after a replacement, with no `withdrawal` named. This is what `withdraw_stranded_prompts` says it does.

## What the source says beside this

- `withdraw_stranded_prompts` (`src/sessions/prompts.rs`) withdraws only a pending `steer` Prompt with no Turn, and says of a queued one that "its delivery is the Session's own to make when that Turn settles".
- `settle_stopped_turns` (`src/sessions/hydration.rs`) then settles that Turn as failed, at restore or at the Session's hydration, where no Provider actor runs to make the delivery.
- On a Server that keeps running, a Turn that fails or is interrupted is followed by the next queued Prompt (`defer_next_queued_prompt` in `src/provider/orchestration.rs`). A new actor starts with nothing deferred, and looks at the queue only once a Turn of its own has settled, which is why B follows C.
- An idle Session starts any Prompt admitted to it at once (`PromptAdmissionDisposition::StartImmediately`), without looking at the queue.
- A promotion changes the Prompt's delivery and tells no actor; a Turn's settlement marks every pending steer delivered (`finish_provider_turn` in `src/sessions/settlement.rs`).

## Not seen

- Windows and Linux; the probe's Codex is a shell script.
- Copilot and Claude. Nothing here is in an adapter, so they should read the same, but neither was run.
- A Server of another build as the replacement, and a Restart.
- A Session read lazily: every case read the Session over HTTP, which hydrates it.
- More than one queued Prompt, a queued Prompt with Attachments or a Skill Invocation, and a Subagent's Session.
- Whether a promoted Prompt is lost the same way on a Server that never stopped, when the Turn it was promoted under settles before a boundary takes it.
