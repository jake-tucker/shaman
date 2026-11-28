# Copilot: what arrives for a picture a tool call returns

Notes for issue #807, the Copilot third of the probe. Everything here is from the captures in `copilot/` unless a
sentence says it is an inference.

## What was run

- 2026-11-28, macOS 27.2.1, `copilot-sdk` 2.0.1 (protocol 4), driven by `examples/copilot_tool_picture_capture.rs`.
- The CLI. `copilot --version` says 1.0.116, but started as Shaman starts it (`--server --stdio --no-auto-update`) the
  CLI that answers `status.get` is **1.0.114**. Every case below ran on 1.0.114, which is what Shaman gets on this
  machine. One case was repeated on 1.0.116 by starting the CLI without `--no-auto-update`
  (`cli-1.0.116-resume-cold.jsonl`); it differs in one place, the history, noted under resume. Each capture's first line
  names the version that answered.
- Models: `claude-haiku-4.5` for every case, `gpt-5.6-luna` for one (`luna-mixed.jsonl`). Both have the built-in `view`.
- The session is created with the options `start_copilot_session` gives one (client name, streaming, config discovery,
  skills, tool search on), permissions in manual mode with each request approved, and the probe's `picture_mcp.py` as a
  stdio MCP server on the session (`mcp.servers`), so its tools are named `pictures-<tool>`.
- Fixtures: the shared ones, and a few more made in scratch for edges, named where used: noise PNGs of an exact byte
  length (noise plus a padding `tEXt` chunk), a 4000 x 4000 PNG of flat bands (56,332 bytes), a 64 x 64 lossless WebP
  (1,572 bytes, hand-encoded, which `sips` decodes), and a directory of mislabelled files. Where a case used these
  through the MCP server, the tool's argument still reads `mib: 1|3|6` or `kind: …`; the byte length and sha256 in the
  capture say which file was served.
- 27 turns in all, about 447 MB of raw capture, kept in scratch.

The files in `copilot/` are the raw captures through `elide.py`, then cut down to the lines that matter with this `jq`
filter, because the raw ones also hold the system prompt and the model's own traffic:

```jq
(.type // "") as $t
| select(._probe
    or ($t | startswith("tool.") or startswith("permission."))
    or ($t | IN("session.idle", "session.error", "session.warning", "session.info",
                "session.binary_asset", "model.tool_execution", "model.messages_snapshot"))
    or ($t == "model.message" and .data.message.role == "tool"))
| if $t == "model.messages_snapshot"
  then .data.messages |= map(if .role == "tool" then . else {role, _dropped: true} end)
  else . end
```

`_raw_bytes` is the length of the event as the example wrote it, which is the event as the SDK handed it over plus about
40 bytes of `_phase` and `_elapsed_ms`. The closing `seen` line of each capture counts every event type that arrived,
kept or not, with the largest of each. Pixel sizes below were read from the PNG headers in the raw captures; the elided
ones hold only the byte length and sha256. The example grew during the probe, so the earliest captures' first lines lack
`fixtures`.

## In short

- The picture is in `tool.execution_complete`, at `data.result`. Which field depends on the tool.
  - The built-in `view` sends **no `result.contents` at all**. Its picture is only in
    `result.binaryResultsForLlm[0]`: `{type: "image", data, mimeType, description}`.
  - An MCP tool's image block arrives twice in the one event: as it was returned in `result.contents[i]`
    (`{type: "image", data, mimeType}`), and as the copy sent to the model in `result.binaryResultsForLlm[j]`
    (`{type: "image", data, mimeType}`).
- `data` is plain base64, inline, no `data:` prefix. The media type field is `mimeType`.
- A picture is never cut short. It is whole, or re-encoded (only the model's copy), or dropped entirely. The 5,000
  character cut that `detailedContent` gets past 20 KiB does not touch it.
- `contents` keeps an MCP picture's own bytes up to exactly 10 MiB (10,485,760 bytes); one byte more and the part is
  dropped. That is past Shaman's 5 MiB attachment limit: a 6 MiB and a 10 MiB picture arrive whole.
- Through MCP only PNG, JPEG and GIF arrive. WebP, TIFF, BMP, HEIC and SVG image blocks are dropped, as is any block
  whose bytes are not what its `mimeType` says.
- The same picture arrives up to four times in a turn: `tool.execution_complete`, and the ephemeral
  `model.tool_execution`, `model.message` and `model.messages_snapshot`.
- A resume replays nothing. History read back (`session.getMessages`) has lost `result.contents`; only the model's copy
  survives, and on 1.0.114 it has moved out of the completion into a `session.binary_asset` event.

## The shape

An MCP tool returning one PNG (`mcp-common.jsonl`; `toolTelemetry` left out):

```json
{
  "type": "tool.execution_complete",
  "data": {
    "toolCallId": "toolu_01ED3ZRrV5pPUeJPg1KqmNGY",
    "model": "claude-haiku-4.5",
    "turnId": "1",
    "success": true,
    "result": {
      "content": "",
      "detailedContent": "",
      "contents": [
        {
          "type": "image",
          "data": {"_elided": "base64", "chars": 11056, "bytes": 8290,
                   "sha256": "eebf856e016f41071cf75022c381da134f0749e5a8cf753eeedfcb3888f2946c",
                   "sniffed": "image/png", "head": "iVBORw0KGgoAAAANSUhEUgAA"},
          "mimeType": "image/png"
        }
      ],
      "binaryResultsForLlm": [
        {
          "type": "image",
          "data": {"_elided": "base64", "chars": 11056, "bytes": 8290,
                   "sha256": "eebf856e016f41071cf75022c381da134f0749e5a8cf753eeedfcb3888f2946c",
                   "sniffed": "image/png", "head": "iVBORw0KGgoAAAANSUhEUgAA"},
          "mimeType": "image/png"
        }
      ]
    }
  },
  "_raw_bytes": 22835
}
```

The built-in `view` on the same file (`view-common.jsonl`): no `contents` key.

```json
"result": {
  "content": "Viewed image file successfully.",
  "detailedContent": "Viewed image file at path /…/fixtures/small.png",
  "binaryResultsForLlm": [
    {
      "type": "image",
      "data": {"_elided": "base64", "chars": 11056, "bytes": 8290,
               "sha256": "eebf856e016f41071cf75022c381da134f0749e5a8cf753eeedfcb3888f2946c",
               "sniffed": "image/png", "head": "iVBORw0KGgoAAAANSUhEUgAA"},
      "mimeType": "image/png",
      "description": "Image file at path /…/fixtures/small.png"
    }
  ]
}
```

`result.content` and `result.detailedContent` never hold the picture. For `view` they are the two sentences above. For
an MCP tool they are the result's text blocks joined with nothing between them (`"first, a PNGthen a JPEG"`), or `""`
when there are none.

## Case 1: the built-in `view`

All on `claude-haiku-4.5`, CLI 1.0.114. "Same" means the sha256 of what arrived equals the file's.

| file | file bytes, pixels | what arrived in `binaryResultsForLlm[0]` | event `_raw_bytes` | capture |
| --- | --- | --- | --- | --- |
| `small.png` | 8,290, 64 x 64 | `image/png`, same | 12,260 | `view-common` |
| `small.jpeg` | 1,699 | `image/jpeg`, same | 3,477 | `view-common` |
| `small.gif` | 3,143 | `image/png`, 601 bytes: re-encoded as PNG (RGBA) | 2,008 | `view-common` |
| `small.webp` (1 x 1) | 34 | `image/webp`, same | 1,257 | `view-common` |
| 64 x 64 WebP | 1,572 | `image/webp`, same | 3,318 | `webp-64px` |
| `small.tiff`, `.bmp`, `.heic` | | nothing: `success: false`, no `result` | about 1,005 | `view-other` |
| `small.svg` | 112 | nothing: read as text, `content` is the source | 1,498 | `view-other` |
| `noise-1mib.png` | 1,048,817, 591 x 591 | same | 1,399,638 | `view-1mib-and-4000px` |
| flat 4000 px PNG | 56,332, 4000 x 4000 | 63,638 bytes, 2000 x 2000: resized | 86,076 | `view-1mib-and-4000px` |
| noise, 1020 px | 3,123,238 | same | 4,165,549 | `view-edge-3mib` |
| noise, 1023 px | 3,141,633 | same | 4,190,074 | `view-edge-3mib` |
| `noise-3mib.png` | 3,147,775, 1024 x 1024 | 3,140,913 bytes, 1023 x 1023: resized | 4,189,098 | `view-3mib` |
| `noise-6mib.png` | 6,293,543, 1448 x 1448 | 3,140,913 bytes, 1023 x 1023: resized | 4,189,098 | `view-6mib` |
| noise, exactly 10 MiB | 10,485,760, 1865 x 1865 | 3,140,913 bytes, 1023 x 1023: resized | 4,189,114 | `view-edge-10mib` |
| noise, 10 MiB + 1 | 10,485,761 | nothing: `success: false` | 1,063 | `view-edge-10mib` |
| noise, 20 MiB | 20,981,315 | nothing: `success: false` | 1,063 | `view-20mib` |

- The refusals' `error.message`: `Cannot view this image: image/tiff is not a format the model reads. Convert the file
  to PNG, JPEG, WebP or GIF and view that instead.` and `File too large`, the second with `result.content`
  `File is too large to view (10.0MB). Maximum file size is 10MB. …`.
- So `view` handed on the file's own bytes for every PNG, JPEG and WebP it took, up to a 3,141,633-byte PNG. A PNG
  past 2000 pixels a side, or past about 3 MiB, came back resized and re-encoded, and the GIF came back as a PNG. The
  byte edge lies between 3,141,633 (kept) and 3,147,775 (resized); that it is 3 MiB, 3,145,728, is an inference. Every
  noise picture past it came out 1023 x 1023, the largest that still encodes under 3 MiB, so the inference is that the
  CLI shrinks until the PNG fits.
- What `view` returns is therefore only ever the model's copy. The file's own bytes past 3 MiB are not in any event.

## Case 2: MCP tools

| tool call | served | `contents` | `binaryResultsForLlm` | event `_raw_bytes` | capture |
| --- | --- | --- | --- | --- | --- |
| `picture png` | 8,290 | `image` `image/png`, same | `image/png`, same | 22,835 | `mcp-common` |
| `picture jpeg` | 1,699 | `image` `image/jpeg`, same | `image/jpeg`, same | 5,261 | `mcp-common` |
| `picture gif` | 3,143 | `image` `image/gif`, same | `image/png`, 601 bytes, re-encoded | 5,719 | `mcp-common` |
| `picture webp` (1 x 1; 64 x 64) | 34; 1,572 | `[]` | absent | 808 | `mcp-common`, `webp-64px` |
| `picture tiff`, `bmp`, `heic`, `svg` | | `[]` | absent | 808 | `mcp-other` |
| `two_pictures` | | `text`, `image` PNG, `text`, `image` JPEG, in the order returned | PNG, JPEG | 27,595 | `mcp-mixed` |
| `resource_picture` | 8,290 | `resource` with `resource.{uri, mimeType, blob}`, blob same | `image` `image/png`, same | 22,879 | `mcp-mixed` |
| `linked_picture` | | `resource_link` with `uri`, `name`, `mimeType`; no bytes | absent | 795 | `mcp-mixed` |
| `big_picture`, 1 MiB | 1,048,817 | same | same | 2,797,571 | `mcp-1mib` |
| `big_picture`, 3 MiB | 3,147,775 | same | 3,140,913, 1023 x 1023 | 8,385,643 | `mcp-3mib` |
| `big_picture`, 6 MiB | 6,293,543 | same | 3,140,913, 1023 x 1023 | 12,579,999 | `mcp-6mib` |
| noise, 7.5, 9.0, 10.2 MB | 7,500,000; 9,000,000; 10,200,000 | same | 3,140,913 each | 14,188,607; 16,188,607; 17,788,607 | `mcp-under-10mib` |
| noise, 10,480,000 | | same | 3,140,913 | 18,161,943 | `mcp-edge-10mib` |
| noise, exactly 10 MiB | 10,485,760 | same | 3,140,913 | 18,169,623 | `mcp-edge-10mib` |
| noise, 10 MiB + 1 | 10,485,761 | `[]` | absent | 808 | `mcp-edge-10mib` |
| noise, 10,495,838; 20 MiB | | `[]` | absent | 808 | `mcp-over-10mib`, `mcp-20mib` |

- A dropped image block leaves `success: true`, `contents: []` and, as both `content` and `detailedContent`,
  `[MCP image omitted because its data was invalid or its MIME type unsupported]`. `toolTelemetry.metrics` counts it:
  `mcp_image_results_omitted: 1`. Nothing else marks it: no part, no size, no media type.
- The size edge is exact and is on the decoded bytes: 10,485,760 and 10,485,761 bytes make base64 of the same length
  (13,981,016 characters), and only the first arrives. The server did send the larger ones (27,975,214 bytes for the
  20 MiB call, by its log).
- The kind is checked against the bytes (`mcp-mislabelled`). All five of these were dropped: `image/png` over WebP
  bytes, `image/webp` over PNG bytes, `image/jpeg` over PNG bytes, `image/gif` over bytes that are no picture,
  `image/tiff` over PNG bytes. A valid 64 x 64 lossless WebP labelled `image/webp` was dropped too, though `view` passes
  the same file. So through MCP an image block arrives only as PNG, JPEG or GIF with a truthful `mimeType`.
- `contents` is the tool's own bytes, never re-encoded or resized. `binaryResultsForLlm` is treated as `view`'s is:
  resized past about 3 MiB, a GIF turned into PNG.
- An event's size is about 4/3 of the two copies together plus 800 bytes: 18.2 MB for a 10 MiB picture. The SDK
  delivered every one of them intact.
- `gpt-5.6-luna` gave the same shapes and the same sizes to within a few bytes (`luna-mixed`).

## Case 3: the SDK's typed event

Each `tool.execution_complete` was parsed as `ToolExecutionCompleteData` and written back out, and the two compared
(the `_probe: "typed"` lines). On 1.0.114 nothing was lost in any of 98 completions, live or read back. The parts came
out as `Image` (`data`, `mime_type`, `type`, which is all the raw part has), `Resource` and `ResourceLink`, with `Text`
in order between them. `binaryResultsForLlm` survives as `binary_results_for_llm`, as bytes and as a reference. On
1.0.116 the one loss was `result.toolReferences`, a field of the tool search's own result that SDK 2.0.1 has no place
for.

What follows from that for today's adapter, read from `omitted_result_parts` and not captured: it counts 1 for an MCP
image, a resource or a link, 2 for `two_pictures`, and 0 for a `view` (no `contents`) and for a dropped image (`[]`).

## Case 5: other events that repeat the picture

No `tool.execution_partial_result`, `tool.execution_progress` or `tool.shell_output` arrived for any of these tools.
Three other events did carry the picture, all `ephemeral: true`, none named by the SDK's types or by Shaman:

| event | where | which copy |
| --- | --- | --- |
| `model.message` (a `role: "tool"` message, just before the completion) | `data.message.content[i].image_url.url`, a `data:<mime>;base64,` URL | the model's |
| `model.tool_execution` (just after) | `data.toolResult.contents` and `data.toolResult.binaryResultsForLlm` | both, as the completion |
| `model.messages_snapshot` (once, at the end of the turn) | `data.messages[n].content[i].image_url.url` | the model's, for every picture of the conversation so far |

For one 3 MiB MCP picture that is 8,385,643 + 8,385,634 + 4,188,690 + 4,224,424 bytes, 25.2 MB of events in the turn
(`mcp-3mib`). For one `view` of the same file, 16.8 MB (`view-3mib`).

## Case 4: resume

The session was disconnected and resumed with the options Shaman resumes with, from a new CLI process
(`resume-cold`) and from the same one (`resume-warm`). Shaman's adapter does not read history; it takes the first
subscription after `session.resume`.

- The resume itself delivered only `session.*` events, the largest a 9.6 KB `session.skills_loaded`, cold and warm
  alike. No tool event and no picture is replayed.
- History, read with `session.getMessages`, is not what streamed. On 1.0.114, cold and warm alike:
  - `result.contents` is gone from every completion, and an MCP result's `detailedContent` with it. An MCP picture's
    own bytes are not in the history: of the 3 MiB picture only the resized 3,140,913-byte copy is left, of the GIF only
    the 601-byte PNG.
  - `binaryResultsForLlm` holds a reference in place of the bytes, and the bytes are in a `session.binary_asset` event
    just before the first completion that uses them, once for each distinct picture. In `resume-warm` the small PNG is
    used by four tool calls and has one asset.
  - A completion is then 700 to 1,300 bytes, and the asset is the picture's base64 plus under 500 bytes
    (4,188,249 for the 3,140,913-byte copy).

```json
{"type": "session.binary_asset",
 "data": {"assetId": "sha256:eebf856e016f41071cf75022c381da134f0749e5a8cf753eeedfcb3888f2946c",
          "type": "image", "mimeType": "image/png", "byteLength": 8290,
          "data": {"_elided": "base64", "chars": 11056, "bytes": 8290, "sha256": "eebf856e…", "sniffed": "image/png"},
          "description": "Image file at path /…/fixtures/small.png"},
 "_phase": "history", "_raw_bytes": 11536}
{"type": "tool.execution_complete",
 "data": {"toolCallId": "toolu_01SgFzisJdXCnxKnH13xLiVR", "success": true,
          "result": {"content": "",
                     "binaryResultsForLlm": [{"type": "image",
                                              "assetId": "sha256:b0abb1fdb40076dcd09123212eb4168627ed6d93dbf942d60c5b0076228603e4",
                                              "mimeType": "image/jpeg", "byteLength": 1699}]}},
 "_phase": "history", "_raw_bytes": 735}
```

- On 1.0.116 (`cli-1.0.116-resume-cold`) the history has no `session.binary_asset`: `binaryResultsForLlm` carries the
  bytes inline again, as it streamed, so the completion for the 3 MiB picture is 4,188,526 bytes. `contents` is gone
  there too. What streamed live on 1.0.116 matched 1.0.114 for `view`, an MCP GIF, a dropped WebP and the 3 MiB picture.
- `resume-cold` was captured before the example wrote every `session.binary_asset`; its `seen` line counts three, and
  the 2.6 KB one for the JPEG has no line of its own. `resume-warm` has them all.

## Not seen or tried

- A lossy WebP, an animated GIF or WebP, an APNG, a CMYK or progressive JPEG, AVIF. The only WebPs were lossless.
- A large JPEG or WebP: every picture past 3 MiB was a PNG, so what the model's copy of another kind becomes is not
  known.
- An MCP picture with large dimensions and few bytes; the 2000 pixel cap was seen only through `view`.
- Where between 3,141,633 and 3,147,775 bytes the model's copy starts being resized, and how the CLI picks the new
  size.
- An MCP `audio` block, and an embedded resource that is not a picture or is past 10 MiB.
- A failed tool call that returns a picture (`isError: true`), and a picture from a subagent's tool call.
- The built-in tools other than `view` that might return a picture, such as a browser or screenshot tool; none was in
  the tool set.
- `OmittedBinaryResult`, the "size-omitted marker" the SDK's types name for `binaryResultsForLlm`: it never appeared,
  live or in history, up to 10 MiB.
- What the session's files on disk hold; history was read only through `session.getMessages`.
- A history read after compaction, and a resumed session's next turn (whether the model is sent the picture again).
- CLI 1.0.116 beyond the one case; Windows and Linux; an HTTP MCP server.
- Whether the `model.*` events are sent to every client or follow from something in this machine's Copilot
  configuration.
