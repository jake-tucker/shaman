# A picture a Tool Call returns arrives inline from Claude and Copilot and from Codex's MCP calls, each under limits of its own, and as a path alone from Codex's `view_image`

Recorded 2026-11-28 on macOS 27.2 (arm64) against Claude Code 2.1.322, `codex-cli` 0.167.0 and
GitHub Copilot CLI 1.0.114, each driven the way its adapter in `src/provider` drives it, with every
line that arrived logged. Asked by
[See what each Provider reports for a picture a Tool Call returns](https://github.com/jake-tucker/shaman/issues/807),
for the desktop app, which shows such a picture under its Activity's row. Today all three adapters
count the part and drop it.

Each Provider has notes of its own, with the events quoted and every case in a table. This file is
what the three say together.

| Provider | Notes | Captures | Driver |
| --- | --- | --- | --- |
| Claude | [notes-claude.md](0807-tool-call-pictures/notes-claude.md) | `0807-tool-call-pictures/claude/` | `probe/claude_capture.py` |
| Codex | [notes-codex.md](0807-tool-call-pictures/notes-codex.md) | `0807-tool-call-pictures/codex/` | `probe/codex_capture.py` |
| Copilot | [notes-copilot.md](0807-tool-call-pictures/notes-copilot.md) | `0807-tool-call-pictures/copilot/` | `examples/copilot_tool_picture_capture.rs` |

## What was run

- A stdio MCP server, `probe/picture_mcp.py`, whose tools return pictures: one of eight kinds (PNG,
  JPEG, GIF, WebP, TIFF, BMP, HEIC, SVG), two with text between them, a PNG of 1, 3 or 6 MiB, an
  embedded resource, and a resource link. `probe/make_fixtures.py` writes the files, and the same
  files were read by each Provider's own tool: `Read`, `view_image`, `view`.
- **Claude** ran live on Haiku, 25 captures. **Copilot** ran live on `claude-haiku-4.5`, with one
  case on `gpt-5.6-luna`, 27 Turns.
- **Codex ran with its Model scripted.** The sign-in on this machine can no longer be refreshed, so
  no Turn reached a real Model. The driver points the real `codex app-server` at a loopback
  stand-in that answers with the call a Model would have chosen; the tool call, the notifications,
  the stored thread and the resume are Codex's own. Image generation, which only a ChatGPT sign-in
  is offered, was not seen.
- Every capture in the repo has passed through `probe/elide.py`: a base64 string is replaced by its
  length, the sha256 of its bytes and the kind those bytes begin as, and each line keeps
  `_raw_bytes`, its length as it arrived. No raw capture is in the repo.

## What arrives

| | Event | Where the picture is | Media type | Bytes |
| --- | --- | --- | --- | --- |
| Claude, `Read` and MCP | the `user` event holding the tool result | `message.content[i].content[j]`: `{"type":"image","source":{"type":"base64","media_type","data"}}`, and again in the event's top-level `tool_use_result` | `source.media_type` | Base64 inline, twice |
| Codex, MCP | `item/completed` for an `mcpToolCall` | `params.item.result.content[i]`: `{"type":"image","data","mimeType"}` | `mimeType` | Base64 inline, once |
| Codex, `view_image` | `item/started` and `item/completed` for an `imageView` | `params.item.path`, absolute | none | None: the path alone |
| Codex, image generation | not seen | the installed schema gives the item `savedPath` and one of `result` or `fileId` | | |
| Copilot, MCP | `tool.execution_complete` | `data.result.contents[i]`: `{"type":"image","data","mimeType"}`, and the Model's copy in `data.result.binaryResultsForLlm[j]` | `mimeType` | Base64 inline, twice |
| Copilot, `view` | `tool.execution_complete` | `data.result.binaryResultsForLlm[0]` alone: `{"type":"image","data","mimeType","description"}`. There is no `contents` | `mimeType` | Base64 inline, once |

- No `data:` URL in any of them, but for Codex's raw response items, which Shaman does not ask for.
- No streaming or partial event carries a picture in Claude or Codex. Copilot repeats it in three
  ephemeral events (`model.tool_execution`, `model.message`, `model.messages_snapshot`), so one
  3 MiB MCP picture made 25 MB of events in a Turn.
- An MCP result's order of text and pictures is kept by all three. Claude adds a text block after
  each MCP picture naming a file the CLI saved it to.
- An embedded resource holding a PNG arrives as an ordinary image block from Claude, and as
  `{"type":"resource","resource":{"uri","mimeType","blob"}}` from Codex and Copilot. A resource
  link carries no bytes anywhere and nobody fetches it.

## Whose bytes they are, and how large

| | Arrives unchanged | Past that |
| --- | --- | --- |
| Claude | up to 256,227 bytes seen | From 512,343 bytes seen, the CLI sends a JPEG it made itself at the same pixel size, called `image/jpeg`: a 3 MiB PNG arrived as 504,945 bytes and a 6 MiB one as 477,309. A 2600 px square came back 1932 px. `Read` and MCP alike. Nothing was refused. |
| Codex, MCP | a 781,113-byte picture | Once the whole result passes about 1 MiB as JSON it arrives as one `text` block, the result's JSON with its middle cut out, on a call still marked `completed`. A 796,498-byte picture arrived so. |
| Copilot, MCP `contents` | up to exactly 10,485,760 bytes | One byte more and the part is dropped, with `success: true` and a sentence in `content`. It is never cut short, as a completion's diff is. |
| Copilot, the Model's copy | up to about 3 MiB and 2000 px a side | Resized. For `view` this is the only copy, and `view` refuses a file over 10 MiB. |

The largest event seen that carried a picture: 1.35 MB from Claude (each picture rides twice),
1.05 MB from Codex, and 18.2 MB from Copilot for a 10 MiB picture. A picture of Claude's is never
past about 0.5 MiB, and one of Codex's never past about 0.78 MiB. Only Copilot hands over a picture
past the Attachment's 5 MiB.

## Which kinds

| Kind | Claude `Read` | Claude MCP | Codex MCP | Codex `view_image` | Copilot MCP | Copilot `view` |
| --- | --- | --- | --- | --- | --- | --- |
| PNG, JPEG | picture | picture | picture | item, path | picture | picture |
| GIF | picture | picture | picture | item, path | picture | arrives as a PNG |
| WebP | picture | picture | picture | item, path | dropped | picture |
| TIFF | error | text naming a saved file | picture | item, path | dropped | refused |
| BMP, HEIC | error | text naming a saved file | picture | no item at all | dropped | refused |
| SVG | its text | text naming a saved file | picture | no item at all | dropped | its text |

Codex passes an MCP block through whatever it claims to be, so it is the one Provider that hands
over a kind beside the Attachment's four. Copilot drops an MCP block whose bytes are not what its
`mimeType` says. Codex's `imageView` names no kind, so a GIF's or a TIFF's path reads like any
other, and a file Codex could not decode, or could not find, leaves no item and no error for a
Client: only the Model is told.

## Resume

| | What comes back |
| --- | --- |
| Claude | Nothing. `--resume` replays no event, and Shaman reads no history from the CLI. The CLI's own transcript does hold the picture. |
| Codex | `thread/resume` and `thread/read` answer with every stored item, but a result past about 64 KiB comes back as cut text. A 44,852-byte picture came back whole and a 53,283-byte one did not. An `imageView` comes back as its path. |
| Copilot | Nothing is replayed. History read with `session.getMessages` has lost `contents` and keeps the Model's copy alone, moved on CLI 1.0.114 into a `session.binary_asset` event and inline again on 1.0.116. |

No Provider can be asked again for a picture it once reported. One that is to be shown later has
to be kept as it arrives.

## What else returns something that is not text

- Claude's `Read` of a PDF returns a `document` block, base64 `application/pdf`. Its `ToolSearch`,
  which precedes the first MCP call of a Session because MCP tools are deferred, returns a
  `tool_reference` block. `omitted_result_parts` in `src/provider/claude/projection.rs` counts every
  block that is not text, so read from source, and not run through the adapter, both are counted as
  parts left out today.
- Claude's `Read` of a notebook returns an output's picture as an image block when it is tiny, and
  a sentence in its place otherwise.
- Copilot's `view` returns no `contents`, so `omitted_result_parts` in
  `src/provider/copilot/projection.rs`, which counts `contents`, counts nothing for a picture it
  showed the Model.

## How large a browser's screenshot is

Asked of Playwright's MCP server (`@playwright/mcp` 0.0.86, headless Chromium) directly over stdio,
with no Provider between, by `probe/browser_screenshot_sizes.py`; the answers are in
`0807-tool-call-pictures/browser/screenshots.jsonl`. The result is a text block naming the file the
server saved and one image block, base64, whose `mimeType` is the type asked for.

| Screenshot | PNG | JPEG |
| --- | --- | --- |
| The viewport, 1280 x 800 to 2560 x 1440 | 0.13 to 0.62 MiB | 0.11 to 0.46 MiB |
| The whole page, 29,000 to 65,000 px tall | 2.5 to 6.9 MiB | 2.8 to 5.9 MiB |

Seven of the eleven whole-page screenshots that came back were past 5 MiB, and one more failed in
the browser (`Unable to encode screenshot`). Two pages were asked for, one of text and one of
photographs, on Wikipedia and Wikimedia Commons. Headless, a device pixel is a CSS pixel; a window
on a display of twice the density makes a picture of four times the pixels under `scale: "device"`,
which was not run.

So a viewport's screenshot passes every Provider whole, but for Claude, which re-encodes one past
about half a megabyte as a JPEG. A whole page's is past what Codex hands over at all, is handed over
by Claude as a JPEG of about half a megabyte, and reaches Shaman as the browser made it only from
Copilot. Every whole-page screenshot was several times taller than the 8,192 px a side Rune draws.

## Not seen

- **Codex on a real account.** No Turn reached a Model, so that a real Model's calls produce the
  same items is inferred from the tools being Codex's own. Image generation was not seen at all:
  whether its item carries base64 inline or a `fileId`, of what kind and size, and whether
  `savedPath` names a file holding the same bytes. `probe/codex_capture.py image-generation` runs
  it once `codex login` has been done, and every other case runs again without `--scripted`.
- A Codex Model that is not in code mode, where a tool is called directly and not through `exec`.
- Claude's exact threshold for re-encoding, which lies between 256,227 and 512,343 bytes; a picture
  past 6 MiB; whether a picture that is wide and not square, or one from MCP, is scaled as the
  square one read from disk was.
- An animated GIF or WebP, a large JPEG, and audio.
- A failed call that returns a picture, a native Subagent's tool result, and an MCP server reached
  over HTTP, the Broker among them.
- Whether Copilot's three `model.*` events reach Shaman's own event drain, and so whether their
  cost is Shaman's.
- A screenshot from a browser with a window, on a dense display, or from another browser server.
- Windows and Linux. Nothing was run through an adapter of Shaman's.
