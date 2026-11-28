# A GPT Model shows a picture as a Markdown image and a Claude Model never does, under whichever Provider runs it

Recorded 2026-11-28 on macOS 27.2 (arm64) against Claude Code 2.1.322, `codex-cli` 0.167.0 and
GitHub Copilot CLI 1.0.117, each driven the way its adapter in `src/provider` drives it. Asked by
[See what each Provider's agent writes when it shows a picture](https://github.com/jake-tucker/shaman/issues/822).
[ADR-0086](../adr/0086-keep-a-picture-a-message-names-as-the-message-is-written-and-hand-it-over-by-id.md)
has the Server find a Message's pictures by reading it as CommonMark and taking every image's
destination, and what a real agent writes had been reasoned and never seen.

160 Turns were run, each a fresh session with one prompt. Every Message each agent wrote is kept
byte for byte under [`0822-message-pictures/captures/`](0822-message-pictures/captures), and each
was read by `pulldown-cmark` under the TUI's options, the parser the ADR names, to say what the
rule would find. [`table.md`](0822-message-pictures/table.md) has a line for every capture and
[`summary.json`](0822-message-pictures/summary.json) the same in full.

## What was seen, in short

- **It is the Model's family that decides, and not the Provider.** GPT Models write
  `![…](path)` when asked to show a picture, under Codex and under Copilot. Claude Models never
  do, under Claude Code and under Copilot: 0 of 75 Turns. Gemini, tried seven times under Copilot,
  does about half the time.
- **For a GPT Model the rule stands where the reader asked to see the picture**: 35 of 37 such
  Turns that made or had a picture, across Codex and Copilot. Every spelling the ADR reads was
  written but `~`: absolute, relative, a `file:` URL, percent-encoded, and angle brackets round a
  path with a space.
- **Told only where a picture is, or having made one unasked, no Model writes an image.** A GPT
  Model under Codex writes a link, `[name](/absolute/path)`, as Codex's own instructions tell it
  to for any file; under Copilot a link or a code span. The rule finds none of these, 0 of 25.
- **A Claude Model names the picture in a code span**, a relative or an absolute path, and takes
  the picture as shown by other means: its `Read` or `view` call ("it's shown above"), or `open`
  run on the reader's machine. In 4 of 26 Turns asked to show a picture already on disk, the
  Message did not name the file at all.
- **One sentence appended to Claude's system prompt changes that**: Opus then wrote an image in 7
  of 7 Turns and Sonnet in 3 of 7, one more of Sonnet's being an image CommonMark does not read,
  for a bare space in its destination.

## What was run

`probe/make_project.py` writes a small project: a README that embeds `docs/revenue.png`, which is
not there, `data/sales.csv`, four Python modules, and two pictures, `docs/architecture.png` and
`design notes/login flow.png`. Each run copies it afresh under the home directory, as a real
Workspace stands, makes it a git repository and is given a directory of its own under `$TMPDIR`
holding `screens/dashboard.png` and an empty `exports/`. A venv with matplotlib leads the PATH.

`probe/probe.py` sends one of sixteen prompts and keeps what came back:

| Asked | Cases | The prompt, in short |
| --- | --- | --- |
| to show a picture already on disk | `show-in`, `show-out`, `show-home`, `show-space`, `show-find` | "Show me docs/architecture.png"; the same of an absolute path under `$TMPDIR`; of `~/…/team-photo.jpg`; "Show me the login flow picture from the design notes"; "There's an architecture diagram somewhere in this repo. Can you show it to me?" |
| to make one and show it | `make-show-in`, `make-show-out`, `make-show-space`, `diagram-show` | "Plot monthly revenue from data/sales.csv as a bar chart and show it to me."; the same saved in the directory under `$TMPDIR`; in the design notes folder; a diagram of the modules as a PNG |
| to make one and say where it is | `make-where-in`, `make-where-out`, `make-where-space` | "…and tell me where you saved it."; "Keep it out of the repo, and tell me where you saved it."; "…put it in the design notes folder, and tell me where it is." |
| for other work that makes one | `unasked-in`, `unasked-out`, `unasked-space`, `unasked-fix` | "Add scripts/plot_revenue.py, which charts monthly revenue from data/sales.csv to a PNG, and run it to check it works."; the same writing under `$TMPDIR`; the same in a working directory named `acme dashboard`; "The README shows a revenue chart, but the picture it points at is missing. Fix that." |

No prompt says how to write a path, and only `show-in`, `show-out` and `show-home` spell one.

| Provider | Model | Turns | Driven by |
| --- | --- | --- | --- |
| Claude | Sonnet 5.5 | 26: all sixteen, and five of them twice more | `claude --print`, stream-json, the adapter's flags, every `can_use_tool` allowed |
| Claude | Opus 5.5 | 16 | |
| Claude | Fable 5.2 | 7 | |
| Claude | Sonnet 5.5 and Opus 5.5, with one sentence appended | 7 each | the same, with `--append-system-prompt` |
| Codex | `gpt-6.1-sol`, medium, this machine's default | 26 | `codex app-server`: `thread/start` and `turn/start` as the adapter sends them, `workspace-write`, every approval accepted |
| Codex | `gpt-6.2-sol`, low | 22: all sixteen, and two of them three times more | |
| Copilot | `claude-sonnet-5.5` | 26 | the CLI through the SDK, by `examples/copilot_message_picture_capture.rs`, every request approved |
| Copilot | `gpt-6.2-sol` | 16 | |
| Copilot | `gemini-4-neon` | 7 | |

Each ran under its user's own configuration, as a Shaman Session does: Claude Code's bundled
skills, and the plugins of this machine's Codex home, `visualize` and `spreadsheets` among them.
No instruction file of the user's speaks of pictures or paths. A capture keeps the prompt, the
Execution Directory, every Message in order, what the agent ran, and the pictures on disk once the
Turn settled. The home directory is written `/Users/reader`, the one change to a Message's bytes.

## How the final Message names the picture

One count a Turn, by the strongest way its last Message names a picture the Turn was about: an
**image** (`![…](…)` whose destination the ADR's reading takes to the file), a **link**, a **code
span**, or not at all. No earlier Message of any Turn named a picture more strongly than its
last. "Rule finds" counts the Turns in which ADR-0086's rule, as written, finds the picture.

| | Shown, already on disk | Made and shown | Made, told where | Made unasked | Rule finds |
| --- | --- | --- | --- | --- | --- |
| Claude, Sonnet 5.5 | code span 7, not named 2 | code span 6 | code span 5 | code span 6 | 0 of 26 |
| Claude, Opus 5.5 | code span 5 | code span 4 | code span 3 | code span 4 | 0 of 16 |
| Claude, Fable 5.2 | code span 3 | code span 2 | code span 1 | code span 1 | 0 of 7 |
| Copilot, `claude-sonnet-5.5` | code span 7, not named 2 | code span 6 | code span 5 | code span 5, no picture left 1 | 0 of 26 |
| Codex, `gpt-6.1-sol` | image 9 | image 6 | link 5 | link 2, code span 4 | 15 of 26 |
| Codex, `gpt-6.2-sol` | image 5 | image 8, no picture made 2 | link 3 | link 4 | 13 of 22 |
| Copilot, `gpt-6.2-sol` | image 5 | image 2, link 2 | link 2, code span 1 | code span 4 | 7 of 16 |
| Copilot, `gemini-4-neon` | image 1, code span 2 | image 2 | code span 1 | code span 1 | 3 of 7 |

Where a Message carries an image it often carries a link to the same file beside it, as Codex's
`gpt-6.2-sol` does: `![Architecture diagram](/…/docs/architecture.png)` and under it
`[Open the architecture diagram](/…/docs/architecture.png)`.

## How a path is spelled

**In an image's destination**, in the 38 Turns that wrote one without the appended sentence:

| | Absolute | Relative | `file:` URL | A path with a space |
| --- | --- | --- | --- | --- |
| Codex, both Models | 24 | 4 | 0 | 4, each in angle brackets with the spaces as they are: `![Login flow](</Users/reader/…/design notes/login flow.png>)` |
| Copilot, `gpt-6.2-sol` | 2 | 4 | 1 | 2, percent-encoded: `![…](design%20notes/login%20flow.png)` |
| Copilot, `gemini-4-neon` | 0 | 3 | 0 | 2, percent-encoded |

- Codex writes an absolute path even where its reader wrote a relative one, but for `show-in`
  on `gpt-6.1-sol`, which answered `![Architecture diagram](docs/architecture.png)` all three
  times, and one of its three runs of `make-show-in`. Under Copilot a GPT Model writes relative
  paths for a file under the working directory and absolute ones outside it.
- Every relative destination was relative to the Execution Directory, which here is also the
  Worktree's root.
- The one `file:` URL was `file:///Users/reader/.copilot/session-state/<session>/files/src-module-dependencies.png`:
  three slashes, no host, nothing in it needing an escape.
- No agent wrote `~` in a destination, a backslash escape, or, unprompted, a bare space. Each
  Codex Model once wrote `/private/tmp/…` in a link, for a file it made under `/tmp`.

**In a link's destination**, from GPT Models, 18 seen as a Turn's strongest naming: absolute in
13 and relative in 5, a space in angle brackets under Codex (2) and percent-encoded under Copilot
(1). The label is the file's name or its relative path, at times wrapped in a code span:
``[`design notes/monthly revenue.png`](design%20notes/monthly%20revenue.png)``.

**In a code span**, which is all a Claude Model writes: the span is the path and nothing else in
66 of the 70 Turns where one named the picture (in the other four it is the file's name alone).
Relative to the Execution Directory in about half and absolute in the rest, a space always as it
is: `` `design notes/login flow.png` ``. A Claude Model commonly gives both, "saved to
`docs/revenue.png` … Its full path is `/Users/reader/…/docs/revenue.png`". Opus wrote the one `~`
seen anywhere, repeating the path its reader had typed.

## Whether the rule stands, for each Provider

- **Codex.** It stands for a picture the reader asked to see: 28 of 28 Turns that had one, on two
  Models, by every spelling above. It finds nothing where the agent was asked only where the
  picture is, or made one unasked: 0 of 18, of which 14 name it by a link and 4 by a code span.
  The link is not chance. The instructions Codex's model catalog carries say "When referencing a
  real local file, prefer a clickable Markdown link", with an absolute target, angle brackets
  round a path with spaces, and no `file://`; they say nothing of images.
- **Copilot.** It depends on the Model the Session runs. On `gpt-6.2-sol`, 7 of 9 Turns asked to
  show a picture, the other two naming it by a link; on `gemini-4-neon`, 3 of 5; on
  `claude-sonnet-5.5`, 0 of 26.
- **Claude.** It does not stand: 0 of 49 Turns on three Models, whatever was asked.

## What a reader would have to take as a name

For a Claude Model's pictures to be found from its Message, under Claude or Copilot, the reader
would have to take **a code span that is, whole, a path the ADR's reading takes to a picture**:
relative to the Execution Directory or absolute, spaces as written. That finds the picture in 66
of the 74 Claude-Model Turns that had one. It also finds what nobody asked to see: in all 8
`show-find` Turns on a Claude Model the Message goes on to name the other picture in the repo,
"There's also `design notes/login flow.png`", in a code span of the same shape.

For the Turns where a GPT Model was not asked to show the picture, the reader would have to take
**a link's destination** as well: 16 of the 25 such Turns under Codex and Copilot name it so, and
the rest by a code span.

Nothing read from the Message finds the other eight Claude-Model Turns. In four, both Sonnets on
`show-out` and `show-home`, the Message describes the picture and never names it, its reader
having just typed the path. In four it names the file without its directory.

Three more things were seen that bear on the same choice and are not names in a Message:

- **The picture came back through a tool in every Turn where a Claude Model was asked to show
  one**: 44 of 44, by `Read` under Claude and `view` under Copilot. That is the picture ADR-0088
  keeps under its Tool Call's row, whatever the Message says. The agent takes this for the
  showing: "It's shown above" is said in 7 of those Messages, 6 of them Copilot's. Where it was not
  asked to show the picture, Claude Code still read it back in 19 of 20 Turns and Copilot's
  Claude Model in none of 10.
- **A Claude Model opens the picture on its reader's machine.** In 15 of the 44 the Message says
  the picture was opened in the reader's image viewer or Preview, by `open`, which under a
  Serving Server is the Remote's screen. Opus twice wrote that its reader could not see it: "I
  can't show the file inline in this terminal".
- **One sentence in the system prompt is taken up.** With "This client draws a Markdown image
  whose destination is a local file. To show your reader a picture that is a file on disk, write
  it in your message as a Markdown image: `![description](/path/to/picture.png)`." appended as
  Shaman's Broker note is, Opus wrote an image in 7 of 7 Turns, the two not asked to show among
  them, with a space once in angle brackets and once percent-encoded. Sonnet wrote one in 3 of 7,
  and in a fourth wrote `![Login flow](/Users/reader/…/design notes/login flow.png)`, which
  CommonMark reads as text, so the rule finds nothing. Every destination was absolute. This was
  tried on Claude Code alone.

## What else was seen

- **Codex may answer with a directive of its own app and no picture.** In 2 of 4 runs of
  `make-show-in` on `gpt-6.2-sol`, the `visualize` plugin its home bundles wrote an HTML chart and
  the final Message was, or began with, `visualize{"path":"/Users/reader/…/outputs/monthly-revenue.html"}`.
  There is no image file and nothing Markdown reads as one; a reader of Shaman's sees the line as
  text.
- **Copilot's GPT Model saves a picture it was given no place for in its Session's own
  directory**, `~/.copilot/session-state/<session>/files/`, in 3 Turns. The file goes when the
  Session is deleted, which is the case ADR-0086 keeps a Picture for.
- **Told to keep a picture out of the repo**, Claude Models wrote it to `~/Desktop` (twice) and
  `~/Documents/charts`, Codex to a directory it made under `/tmp`, and otherwise each wrote under
  `/tmp` freely.
- **A Claude Model that "checks" a picture reads it back**, so a picture made unasked usually
  arrives as a Tool Call's picture too.
- Sonnet under Copilot twice wrote a Message between tool calls ("The view raced the plot;
  retrying."), and Codex writes one or two `commentary` Messages before its `final_answer`. None
  names a picture.

## Not seen

- A screenshot: this Mac has no browser an agent can drive headlessly and no screen recording
  permission for a terminal, so no case asked for one.
- Windows and Linux, and so a drive letter or a backslash in a destination, where CommonMark
  takes a backslash before punctuation as an escape.
- A Subagent's Message, a Message cut off by an interrupt, a picture named in a Message of a
  resumed Session, and anything through an adapter of Shaman's.
- Haiku, the other GPT tiers, Gemini past seven Turns, and the other Models Copilot offers.
- The appended sentence under Copilot or Codex, any other wording of it, and whether it holds
  through a long Session.
- Codex without its bundled plugins, which need a Codex home of its own and so a second sign-in.

## Reproducing it

```sh
python3 -m venv <scratch>/venv && <scratch>/venv/bin/pip install matplotlib pillow
<scratch>/venv/bin/python docs/validation/0822-message-pictures/probe/make_project.py <scratch>/template <scratch>/shared
cargo build --example copilot_message_picture_capture --example message_picture_names
python3 docs/validation/0822-message-pictures/probe/probe.py --scratch <scratch> --provider claude --model sonnet
python3 docs/validation/0822-message-pictures/probe/probe.py --scratch <scratch> --provider codex --model gpt-6.1-sol --effort medium
python3 docs/validation/0822-message-pictures/probe/probe.py --scratch <scratch> --provider copilot --model gpt-6.2-sol \
    --copilot-capture target/debug/examples/copilot_message_picture_capture
target/debug/examples/message_picture_names docs/validation/0822-message-pictures/captures/*/*/*.json > names.jsonl
python3 docs/validation/0822-message-pictures/probe/classify.py names.jsonl --table
```

`<scratch>` stands under the home directory. Each run spends the signed-in account, and an agent
is free to run what it likes there: these wrote to `/tmp`, `~/Desktop` and `~/Documents`, and ran
`open` until a stand-in for it was put on the PATH.
