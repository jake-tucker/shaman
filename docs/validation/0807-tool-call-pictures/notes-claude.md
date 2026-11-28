# Claude: what arrives for a picture a tool call returns (#807)

Captured 2026-11-28 on macOS 27.2.1 (arm64) against `claude` 2.1.322 (Claude Code), signed in to
the real account, model `haiku` (the CLI reported `claude-haiku-5-5`). Driven by
`probe/claude_capture.py`; the elided captures are in `claude/`, one file per case, and every line
of a capture carries `_raw_bytes`, its size as it arrived. Sizes below are bytes unless marked.

## How it was driven

The driver launches the CLI with the flags `src/provider/claude/transport.rs` and `session.rs`
give a session: `--print --input-format stream-json --output-format stream-json --verbose
--setting-sources user,project --include-partial-messages --permission-mode default
--permission-prompt-tool stdio`, then `--session-id <uuid>` to mint or `--resume <uuid>` to
continue, then `--model`. It adds `--mcp-config <file> --strict-mcp-config` naming
`probe/picture_mcp.py` as the server `pictures`, `--allowedTools Read mcp__pictures` and
`--add-dir` for the fixtures. No `can_use_tool` request arrived in any run. Each prompt went in as
the stream-json user message `UserMessageEnvelope` writes, and each turn asked for one tool call.

Shaman resumes with `--resume` and nothing else: it passes no `--replay-user-messages` and reads
no history back from the CLI (nothing under `src/provider/claude` opens the CLI's transcript), so
the stdout of a resumed process is all the adapter is handed.

## The shape

A picture arrives in one place only: the `user` event that carries the tool result. It is there
twice, both times as base64 inline.

- `message.content[i]` is the `tool_result` block, and its `content` is a list. The picture is a
  block of that list: `{"type":"image","source":{"type":"base64","media_type":"image/png","data":"…"}}`.
- The same event's top-level `tool_use_result` holds the same bytes again (same sha256). Its shape
  depends on the tool: for `Read` it is an object, `{"type":"image","file":{"base64":"…","type":"image/png","originalSize":…,"dimensions":{…}}}`,
  where `file.type` is the media type; for an MCP tool it is the same list of blocks as the
  `tool_result`'s `content`.

So an event costs about twice the picture's base64 plus some 700 bytes: the 8,290-byte PNG is
11,056 base64 characters and makes an event of 22,819 bytes.

`Read` of `small.png`, elided:

```json
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01S3BKNJ2vwkMXvoUUXBU5S8","type":"tool_result","content":[
  {"type":"image","source":{"type":"base64","data":{"_elided":"base64","chars":11056,"bytes":8290,"sha256":"eebf856e…f2946c","sniffed":"image/png","head":"iVBORw0KGgoAAAANSUhEUgAA"},"media_type":"image/png"}}]}]},
 "parent_tool_use_id":null,"session_id":"a5cb940a-…","uuid":"3351cf5d-…","timestamp":"2026-11-27T16:29:11.154Z",
 "tool_use_result":{"type":"image","file":{"base64":{"_elided":"base64","chars":11056,"bytes":8290,"sha256":"eebf856e…f2946c","sniffed":"image/png","head":"iVBORw0KGgoAAAANSUhEUgAA"},"type":"image/png","originalSize":8290,"dimensions":{"originalWidth":64,"originalHeight":64,"displayWidth":64,"displayHeight":64}}},
 "tool_result_meta":[{"id":"toolu_01S3BKNJ2vwkMXvoUUXBU5S8","permission_decision":{"decision":"accept","source":"config","reason_type":"rule"}}],
 "_raw_bytes":22819}
```

MCP `two_pictures`, the `tool_result`'s `content` (the event's `tool_use_result` is the same list
again; `_raw_bytes` 28,570):

```json
[{"type":"text","text":"first, a PNG"},
 {"type":"image","source":{"type":"base64","media_type":"image/png","data":{"_elided":"base64","chars":11056,"bytes":8290,"sha256":"eebf856e…f2946c","sniffed":"image/png"}}},
 {"type":"text","text":"[Image: source: ~/.claude/projects/<cwd-slug>/1a31c4b3-…/tool-results/mcp-pictures-blob-1795797029056-4t9tsr.png]"},
 {"type":"text","text":"then a JPEG"},
 {"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":{"_elided":"base64","chars":2268,"bytes":1699,"sha256":"b0abb1fd…8603e4","sniffed":"image/jpeg"}}},
 {"type":"text","text":"[Image: source: ~/.claude/projects/<cwd-slug>/1a31c4b3-…/tool-results/mcp-pictures-blob-1795797029056-a6971u.jpg]"}]
```

The server's order is kept. After each MCP image block the CLI adds a text block of its own naming
a file it saved under `~/.claude/projects/<cwd-slug>/<session>/tool-results/`. `Read` adds no such
block and saves no such file.

Order of events around a tool result, in every case: the `stream_event`s of the assistant message,
with its `assistant` event (the `tool_use`) among them; then the `user` event with the result;
then `system` `status` and the next assistant message. No `stream_event` carries the picture, with
`--include-partial-messages` on: the only long base64 outside `user` events was a thinking block's
`signature`. Two `Read` calls made in one assistant message came back as two `user` events, one
`tool_result` each, each after its own `assistant` event (`read-two-in-one-message`).

## Read

| File | File bytes | `content` of the `tool_result` | Bytes arriving | Same as file | `_raw_bytes` |
|---|---|---|---|---|---|
| `small.png` | 8,290 | `image`, `image/png` | 8,290 | yes | 22,819 |
| `small.jpeg` | 1,699 | `image`, `image/jpeg` | 1,699 | yes | 5,245 |
| `small.gif` | 3,143 | `image`, `image/gif` | 3,143 | yes | 9,091 |
| `small.webp` | 34 | `image`, `image/webp` | 34 | yes | 799 |
| `small.tiff` | 15,680 | a string, `is_error: true` | none | | 748 |
| `small.bmp` | 12,342 | a string, `is_error: true` | none | | 746 |
| `small.heic` | 1,018 | a string, `is_error: true` | none | | 748 |
| `small.svg` | 112 | a string: the file's text, line-numbered | none | | 901 |

The error reads `<tool_use_error>This tool cannot read binary files. The file appears to be a
binary .tiff file. …</tool_use_error>`, and `tool_use_result` is then a string too, `"Error: …"`.
For the SVG `tool_use_result` is `{"type":"text","file":{…}}`: it is read as a text file.

Large pictures. No file's own bytes arrived once the file passed some size between 256,227 and
512,343 bytes; past it the CLI sent a JPEG it had made, at the same pixel size.

| File | File bytes | Pixels | Arrived as | Bytes arriving | Pixels arriving | `_raw_bytes` |
|---|---|---|---|---|---|---|
| `noise-100kib.png` | 101,850 | 184² | `image/png`, the file's bytes | 101,850 | 184² | 272,313 |
| `noise-250kib.png` | 256,227 | 292² | `image/png`, the file's bytes | 256,227 | 292² | 683,985 |
| `noise-500kib.png` | 512,343 | 413² | `image/jpeg`, re-encoded | 155,618 | 413² | 415,699 |
| `noise-750kib.png` | 765,878 | 505² | `image/jpeg`, re-encoded | 232,179 | 505² | 619,859 |
| `noise-1mib.png` | 1,048,817 | 591² | `image/jpeg`, re-encoded | 314,655 | 591² | 839,796 |
| `noise-3mib.png` | 3,147,775 | 1024² | `image/jpeg`, re-encoded | 504,945 | 1024² | 1,347,240 |
| `noise-6mib.png` | 6,293,543 | 1448² | `image/jpeg`, re-encoded | 477,309 | 1448² | 1,273,544 |
| `wide-2600.png` | 154,352 | 2600² | `image/png`, scaled down | 19,448 | 1932² | 52,581 |

The 6 MiB file was not refused. The media type in the block and `tool_use_result.file.type` name
what arrived (`image/jpeg`), not what the file was; `file.originalSize` and
`file.dimensions.originalWidth/Height` keep the file's own, and `displayWidth/Height` give the
pixels sent. The largest picture seen arriving was 504,945 bytes, a tenth of the 5 MiB attachment
limit, in an event of 1,347,240 bytes. Noise is the worst case for both encoders, so these are near
the top of what the CLI sends for pictures of these pixel sizes; that is an inference.

The scaled-down picture is followed at once by a second `user` event that is not a tool result
(`read-wide-png`, 368 bytes). It has `isSynthetic: true`, no `tool_use_result`, and one text block:

```json
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Image: original 2600x2600, displayed at 1932x1932. Multiply coordinates by 1.35 to map to original image.]"}]},"parent_tool_use_id":null,"session_id":"95b08f9c-…","uuid":"9c8f2081-…","timestamp":"2026-11-28T01:54:05.766Z","isSynthetic":true,"_raw_bytes":368}
```

## MCP

The first MCP call of every session was preceded by the model calling `ToolSearch`: the CLI lists
MCP tools as deferred (`total_deferred_tools: 22` in that result). Later turns of the same session
called the tool directly. The `ToolSearch` result is a `user` event whose `tool_result` `content` is `[{"type":"tool_reference","tool_name":"mcp__pictures__picture"}]`:
a block that is neither text nor a picture.

| Tool | `content` of the `tool_result` | Bytes arriving | Same as server sent | `_raw_bytes` |
|---|---|---|---|---|
| `picture` png | `image` `image/png`, `text` | 8,290 | yes | 23,228 |
| `picture` jpeg | `image` `image/jpeg`, `text` | 1,699 | yes | 5,654 |
| `picture` gif | `image` `image/gif`, `text` | 3,143 | yes | 9,500 |
| `picture` webp | `image` `image/webp`, `text` | 34 | yes | 1,216 |
| `picture` tiff | `text` only | none | | 1,058 |
| `picture` bmp | `text` only | none | | 1,056 |
| `picture` heic | `text` only | none | | 1,066 |
| `picture` svg | `text` only | none | | 1,070 |
| `two_pictures` | `text`, `image` png, `text`, `text`, `image` jpeg, `text` | 8,290 and 1,699 | yes | 28,570 |
| `big_picture` 1 | `image` `image/jpeg`, `text` | 314,655 | no, re-encoded | 840,198 |
| `big_picture` 3 | `image` `image/jpeg`, `text` | 504,945 | no, re-encoded | 1,347,638 |
| `big_picture` 6 | `image` `image/jpeg`, `text` | 477,309 | no, re-encoded | 1,273,942 |
| `resource_picture` | `text`, `image` `image/png`, `text` | 8,290 | yes | 23,376 |
| `linked_picture` | `text` only | none | | 942 |

- The trailing `text` after an image is the CLI's `[Image: source: <path>]` block. The file at that
  path holds the bytes the server sent (sha256 equal to the fixture), also when the event carries a
  JPEG: for `big_picture` 6 the event has the 477,309-byte JPEG and the saved `.png` is the
  6,293,543-byte original.
- The large pictures are re-encoded exactly as `Read` re-encodes them: the JPEGs have the same
  sha256 as the ones `Read` produced from the same files. The server's reply for `big_picture` 6
  was 8,391,518 bytes; the CLI took it without complaint.
- A kind outside PNG, JPEG, GIF and WebP never arrives as a picture. The result is one text block,
  not an error: `[Image from pictures] Binary content (image/tiff, 15.3KB) saved to ~/.claude/projects/<cwd-slug>/<session>/tool-results/mcp-pictures-blob-….bin`.
  The saved file holds the server's bytes; TIFF, BMP and HEIC were named `.bin`, SVG `.svg`.
- An embedded resource whose blob is a PNG becomes a text block `[Resource from pictures at picture://small.png] `
  followed by an ordinary `image` block and its `[Image: source: …]` text.
- A resource link is not fetched. The result is the text `[Resource link: small.png] file:///…/small.png`,
  and here `tool_use_result` is an object, `{"content":[…],"resourceLinks":[{"uri":"file:///…","name":"small.png","mimeType":"image/png"}]}`,
  not a list.

## Resume

The sessions of `read-png` and `mcp-two-pictures` were resumed with `--resume` and one turn that
used no tool (`resume-read-png`, `resume-mcp-two-pictures`). The resumed process emitted
`command_lifecycle` (queued, started), `system` `init`, `system` `status`, the new assistant
message's events, `rate_limit_event`, `result`, and `command_lifecycle` (completed). Nothing of the
earlier turns was replayed: no `user` event, and no base64 anywhere in either capture. A picture
from before a resume is therefore not handed to the adapter again.

The CLI's own transcript does hold it. `~/.claude/projects/<cwd-slug>/<session>.jsonl` has one
`user` line per tool result with the picture at the same two places, `message.content[i].content[j].source.data`
and the top-level result, spelled `toolUseResult` there (`toolUseResult.file.base64` for `Read`,
`toolUseResult[j].source.data` for MCP). It holds what the event held: for the 6 MiB cases the
JPEG, in a line of 1,273,837 bytes. The scaled-down picture's note is a `user` line with `isMeta`.
For MCP pictures the bytes the server sent are also on disk, in the `tool-results/` files above.

## Other built-in tools

- `Read` of a PDF (`read-pdf`): the `tool_result`'s `content` is a `text` block (`PDF file read: <path> (584 bytes)`)
  and then `{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"…"}}`,
  the file's own bytes. `tool_use_result` is `{"type":"pdf","file":{"filePath":…,"base64":"…","originalSize":584}}`.
  So `document` is a third kind of non-text block a tool result carries, after `image` and
  `tool_reference`.
- `Read` of a notebook whose cell output is a PNG. With an 8x8 PNG (`read-notebook-tiny`) the
  `content` is a `text` block with the cell and then an `image` block, `image/png`, the output's
  own base64; the second copy is at `tool_use_result.file.cells[0].outputs[0].image.image_data`,
  with `media_type` beside it. With the 64x64 PNG (`read-notebook`, an 11,320-byte file) no picture
  arrived: the text says `Outputs are too large to include. Use Bash with: cat <notebook_path> | jq '.cells[0].outputs'`.
- No screenshot tool is among the tools the CLI listed in `init`, so none was tried.

## Not seen or not tried

- A picture the CLI refused or failed on for size; anything larger than the 6 MiB, 1448² PNG; where
  between 256,227 and 512,343 bytes the re-encoding starts, and whether that boundary is in file
  bytes, base64 characters or something else.
- Scaling down of an MCP picture (only `Read` was given the 2600² one), and scaling of a picture
  that is not square.
- A large JPEG, GIF or WebP; an animated GIF; a picture whose media type the server misstates.
- A tool result with `is_error: true` that also carries a picture.
- A picture in the result of a tool call made by a subagent (`parent_tool_use_id` set), or from
  `Bash`, `WebFetch` or any tool other than `Read` and the MCP server.
- A multi-page PDF, or a PDF read with a page range (the CLI may hand pages back as pictures; not
  tried).
- An MCP server over HTTP rather than stdio, and MCP `audio` blocks.
- `--replay-user-messages`, which Shaman does not pass, and a resume after compaction.
- Other CLI versions and other models. Whether Haiku's choice to call `ToolSearch` first holds for
  other models was not checked.
- What Shaman's adapter does with these events: nothing was run through it. From the source,
  `omitted_result_parts` would count the `image`, `document` and `tool_reference` blocks alike.
