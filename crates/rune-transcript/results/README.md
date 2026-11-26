# What the prototype measured

For [#774](https://github.com/jake-tucker/shaman/issues/774). Rune at `983e33b9`, a release build, on an Apple M1 Max
(32 GB, macOS 27.2.1, a 120 Hz display), 2026-11-26. `headless.md` and `window.md` are the runs as printed, with the
Sessions named by letter: the repository is public and their text and ids are the user's.

## What was read

Six Sessions copied from this machine's 966, chosen as the largest by bytes and by rows (A: 1,068 rows, 4.28 MB as
JSON), the most Messages (B: 241 of 843 rows), the second most rows (C: 944), the median by bytes (D: 42 rows,
0.33 MB), the longest Message (E: 27.6 KB) and the largest Activity (F: 80 KB). `make-scratch.sh` made the copy.

The Server was a real one, launched and read through `ManagedClient` over the loopback wire, with no Provider.
"Live" was a scripted Turn published into a Session through `SessionEventSink` and `AgentOutputSink`: the same
changes on the same wire as a Provider's, at a rate the script chose. No Provider ran, so no real model's cadence was
seen.

Two ways of measuring, and what each cannot say:

- **Headless** (`RT_MEASURE`): Rune's `Runtime` with no window. A frame's cost there is the view, layout, text
  shaping and paint list on the CPU. Nothing is drawn, so it holds no GPU memory and says nothing of the GPU.
- **The window driving itself** (`RT_AUTO`): a real window on the Mac, timed by Rune's own `on_frame_encoded`
  (`update_ms`: events, `poll`, view, layout, paint; `encode_ms`: uploads and draw calls, CPU time) and
  `on_frame_presented`. Nobody was looking, so it says nothing of how it looks or feels.

Each row drew a Message as kit Markdown, or an Activity as one line and the last 12 lines of what it printed.

## The numbers

| | Session A, 1,068 rows, 4.28 MB | Others |
|---|---|---|
| Asked → snapshot decoded on tokio's thread | 67 to 84 ms | 6.5 to 20 ms |
| Rows indexed in the model | 0.15 ms | under 0.2 ms |
| First frame, headless | 17 to 23 ms | 8 to 34 ms |
| Asked → first frame on the display, in the window | 115 to 123 ms | not run |
| Wheel foot to head, headless, 240 pt a frame | 859 frames, 0.41 s in all, p95 1.3 ms, worst 2.9 ms | p95 at most 2.4 ms but for F, 8.4 ms |
| Stepped head to foot in the window, 240 pt every 8 ms | 878 frames: update p50 0.96, p95 3.1, worst 15.6 ms; encode p95 0.34 ms | not run |
| A keystroke's commit, headless | p50 0.10 ms, worst 0.5 ms | E, the heaviest in view: p50 0.27 ms |
| A keystroke's commit in the window | update p50 0.4 to 0.9 ms | not run |
| Resident memory in the window | 99 MB with no Session, 120 to 125 MB with A open, 147 MB after scrolling all of it, 153 MB after three streamed Turns | not run |

Streaming into Session A:

| | Cost of a frame |
|---|---|
| 300 words, a commit for every update, headless | p50 0.38, p95 0.67, worst 1.3 ms |
| 1,500 words, a commit for every 16 ms of updates, headless | p50 2.2, p95 8.3, worst 22 ms |
| One 6,000-word Message, a commit for every 16 ms, headless | first tenth p50 1.8, p95 7.0 ms; last tenth p50 4.6, p95 12.5, worst 17 ms |
| 400 words at 80 a second, in the window, at the foot | 418 to 426 frames over 6.2 s; update p50 0.6 to 1.2, p95 0.8 to 2.9, worst 6.6 ms |
| 1,200 words at about 310 updates a second, in the window | 433 to 445 frames over 3.9 s; update p50 0.8 to 1.1, p95 1.1 to 3.0 ms |
| 400 words at 80 a second, in the window, scrolled back | update p50 0.4 to 0.9, worst 6.7 ms |

Scrolled back to the middle of every Session, and paged back one and four pages from the foot of two, no row in view
moved by any fraction of a point while 300 words streamed in at the foot.

Longer than this machine holds, by tiling Session A in the Client (the Server sent 1,068 rows; the rest are copies):

| Rows | First frame | Wheel end to end | Keystroke, keys versioned | Keystroke, keys asked of every row | Memory, headless |
|---|---|---|---|---|---|
| 1,068 | 17 ms | 0.41 s | 0.10 ms | not run | 79 to 111 MB |
| 10,680 | 23 ms | 4.3 s, p95 1.3 ms, worst 8.6 ms | 0.09 ms | 0.16 ms | 111 MB |
| 106,800 | 26 ms | 45 s, p95 1.4 ms, one frame of 171 ms | 0.11 ms | 0.79 ms | 319 MB |

Drawing every line of every Activity (`RT_ACTIVITY_LINES=0`) made Session A 4,838 wheel frames tall. A frame cost the
same: p95 1.07 ms, worst 13.9 ms.

Without `memo` around each row, Session A's keystroke commit went from p50 0.10 to 0.26 ms, and a streamed update's
from 0.38 to 0.63 ms.

## How the pieces met

- A tokio runtime of two workers on a thread of its own holds the `ManagedClient` and the `SessionSubscription`.
  Rune's `App::run` holds the main thread. The UI asks through a `tokio::sync::mpsc` sender kept in Rune's state.
- All three ways of crossing back worked and cost the same within this machine's noise: `AppHandle::update` once an
  event, once for every event already waiting (`SessionSubscription::try_next`), and a channel drained in `poll` and
  woken by `setup`'s `Waker`.
- However many crossings arrive between two frames, Rune commits once: 1,233 `update` calls in 3.9 s drew 433 to 445
  frames. Batching with `try_next` saved almost nothing, since the reader keeps up with the Server and finds the
  queue empty: 1,229 crossings for 1,233 updates.
- The model lives in Rune's state: the Server's snapshot kept whole and updated in place by `land_update`, and one row
  per Transcript item with a key and a revision. `land_update` answers where each change landed, which is what says
  which row's revision to move.
- The view re-runs once for each frame in which a handler, an `update` or a `poll` that answered true ran, and never
  for a scroll. `virtual_rows` then builds only the mounted rows, and `memo(key, revision, ..)` skips those whose
  item did not change.

## What Rune did not do, and what to watch

- **The foot eases at a capped speed.** Rune's growth stack limits how fast a growing row's drawn height may follow
  its real one. At 80 words a second nothing lagged. After 1,500 words sent in a little over two seconds the list was still
  animating 3.2 s later, and after one 6,000-word Message 11.7 s later (`at rest N frames after`, at 60 a second).
  Rows beneath a row still easing move as it does. `GrowthSpec`'s spring and edge tune it; none was tried.
- **A streaming Message costs more as it grows**, since each commit hands kit Markdown the whole source again. By the
  end of a 6,000-word Message a commit's p95 is 12.5 ms headless: inside a 60 Hz frame, outside a 120 Hz one. The
  longest real Message here is 27.6 KB, about 4,000 words.
- **A selection copies at most 4 MiB.** Selecting all of the 10,680-row tiling took 36 ms and yielded exactly
  4,194,304 bytes. Selecting all of Session A took 9 ms and yielded 828 KB, rows never built included.
- **`Tx::scroll_to_with` with a six-second spring drew two frames**, not a glide. Not looked into; the window's
  scroll was stepped with `scroll_to_instant` instead.
- **One frame of 171 ms, and in another run one of 458 ms**, while wheeling through 106,800 rows. Not found again at
  10,680, and not looked into.
- **One encode of 123 ms** in the window during one streamed Turn, once in four runs. Not looked into.
- **A paragraph does not wrap unless the column around it may shrink** (`min_w(0.0)`), flexbox's own rule.
- **A Markdown's selection group must be cleared** for a drag to cross Messages, as #768 found. With that, a drag
  across five real rows (a user Message, a Reasoning block, an Agent Message, two Commands) selected 983 bytes over
  15 lines, and the copy key put the same text on the headless clipboard.

## Not done

- No real Provider streamed, and nothing was run on Windows or Linux.
- Nothing here draws a Turn Fold, an Approval, a Questionnaire, an image or a diff, so a row here is lighter than a
  real one will be.
- Memory was read from `ps` as resident size, once per phase.
