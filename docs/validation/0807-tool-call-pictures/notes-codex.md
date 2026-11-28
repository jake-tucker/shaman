# Codex: what arrives for a picture a tool call returns (#807)

Captured 2026-11-28 on macOS 27.2.1 (arm64) against `codex-cli 0.167.0` (`codex app-server` over
stdio), driven by `probe/codex_capture.py`. The elided captures are in `codex/`, one file per case.

## Read this first: the model was scripted

No turn ran on the real account. This machine's Codex sign-in is a ChatGPT one that Codex can no
longer refresh (`auth.json` was last refreshed 2026-10-08), so every turn failed before reaching a
model:

```json
{"method":"error","params":{"error":{"message":"Your access token could not be refreshed. Please log out and sign in again.","codexErrorInfo":"unauthorized","additionalDetails":null,"misalignment":null},"willRetry":false,"threadId":"…","turnId":"…"}}
```

Signing in again is the user's to do, so the captures were made another way. The driver serves a
loopback Responses API and points `codex app-server` at it with launch-time `-c` overrides
(`model_provider="scripted"`, `model_providers.scripted.base_url=…`), under a Codex home of its own
in scratch. That stand-in answers each request with one scripted output item: the call a model
would have chosen. Everything after that is the real 0.167.0 app-server's own doing: it runs the
MCP call or `view_image`, emits the notifications, stores the thread and answers the resume.

What this does and does not cover:

- An MCP result and `view_image` are produced by Codex on this machine, so their notifications are
  what a signed-in run emits for the same call. That the same call is available to a signed-in
  model is an inference: the model catalog cached in the user's Codex home marks every listed
  model, `gpt-5.6-luna` included, `tool_mode: code_mode_only` (fetched 2026-10-17), which is the
  mode the stand-in ran in.
- Image generation was **not seen**. Its picture comes from OpenAI's service, and Codex offers the
  tool only under a ChatGPT sign-in.
- Nothing here says what a real model chooses to do with a picture, only what Codex reports.

The driver runs the same cases against the signed-in account when `--scripted` is left off, so the
whole set can be repeated once `codex login` has been done.

## What was run

- `initialize` with `clientInfo.name: "shaman"` and `experimentalApi: false`, then `initialized`,
  as `transport.rs` does.
- `thread/start` with `cwd`, `approvalPolicy: "on-request"`, `sandbox: "workspace-write"`,
  `ephemeral: false` and a `config` map holding `model_reasoning_effort: "low"` and
  `mcp_servers.pictures` (`command`, `args`, `env`, `tool_timeout_sec`,
  `default_tools_approval_mode: "approve"`): the stdio server `probe/picture_mcp.py`.
- One `turn/start` per case, naming `gpt-5.6-luna` at effort `low`.
- For resume, a new app-server and `thread/resume` with the same fields, as `session.rs` does, then
  `thread/read` with `includeTurns: true`, and for three threads `thread/turns/list` and
  `thread/items/list`.

No approval or elicitation request reached the client in any capture.

In this mode the model has one tool, `exec`, which runs JavaScript; every other tool is a function
on a `tools` object inside it (`tools.mcp__pictures__picture`, `tools.view_image`). The `exec` call
itself produces no item. The nested calls do, with ids of the form `exec-<uuid>`. Each case's
script is in `codex/model-side.jsonl` (`sent_by_stand_in`).

In the committed captures the host name, installation id, home directory and time zone are
redacted; `_raw_bytes` is the size of the line as it arrived.

## Summary

| Case | Notification and path | Block | Media type | Bytes in the event |
| --- | --- | --- | --- | --- |
| MCP image block | `item/completed`, `params.item.result.content[i]` | `{"type":"image","data","mimeType"}` | `mimeType`, as the server set it | Bare base64 in `data`, the server's own bytes, while the whole result is under about 1 MiB |
| MCP embedded resource | same | `{"type":"resource","resource":{"uri","mimeType","blob"}}` | `resource.mimeType` | Bare base64 in `resource.blob` |
| MCP resource link | same | `{"type":"resource_link","uri","name","mimeType"}` | `mimeType` | None: a `file://` URI |
| `view_image` | `item/started` and `item/completed`, `params.item` | `{"type":"imageView","id","path"}` | None | None: an absolute path |
| Image generation | not seen | | | |

The only notification that carried picture bytes was `item/completed` for an `mcpToolCall`.
`item/started` has `result: null`. `turn/completed` lists the turn's `agentMessage` alone
(`itemsView: "summary"`). No delta or progress notification carried a result.

## MCP tool result

The completed item for `picture {kind: "png"}`, 11,681 bytes on the wire:

```json
{"method":"item/completed","params":{"item":{"type":"mcpToolCall","id":"exec-6d977475-b9c9-4c67-b164-c4d4a2a0a62d","server":"pictures","tool":"picture","status":"completed","arguments":{"kind":"png"},"appContext":null,"mcpAppUi":null,"pluginId":null,"readOnlyHint":null,"result":{"content":[{"type":"image","data":{"_elided":"base64","chars":11056,"bytes":8290,"sha256":"eebf856e016f41071cf75022c381da134f0749e5a8cf753eeedfcb3888f2946c","sniffed":"image/png","head":"iVBORw0KGgoAAAANSUhEUgAA"},"mimeType":"image/png"}],"structuredContent":null,"_meta":null},"error":null,"durationMs":0,"startedAtMs":1795797331468,"completedAtMs":1795797331469},"threadId":"…","turnId":"…","completedAtMs":1795797331469},"emittedAtMs":1795797331470,"_raw_bytes":11681}
```

Codex passes the blocks through as the server wrote them. It does not check, convert or drop a
kind:

| Tool | Blocks in `result.content` | Picture | Same bytes as the fixture | `_raw_bytes` |
| --- | --- | --- | --- | --- |
| `picture` png | image `image/png` | 8,290 bytes, 11,056 chars | yes | 11,681 |
| `picture` jpeg | image `image/jpeg` | 1,699 bytes | yes | 2,895 |
| `picture` gif | image `image/gif` | 3,143 bytes | yes | 4,817 |
| `picture` webp | image `image/webp` | 34 bytes | yes | 675 |
| `picture` tiff | image `image/tiff` | 15,680 bytes | yes | 21,535 |
| `picture` bmp | image `image/bmp` | 12,342 bytes | yes | 17,081 |
| `picture` heic | image `image/heic` | 1,018 bytes | yes | 1,987 |
| `picture` svg | image `image/svg+xml` | 112 bytes | yes | 781 |
| `two_pictures` | text, image `image/png`, text, image `image/jpeg`, in the server's order | 8,290 and 1,699 bytes | yes | 14,068 |
| `resource_picture` | resource, `resource.mimeType` `image/png`, bytes in `resource.blob` | 8,290 bytes | yes | 11,722 |
| `linked_picture` | resource_link, `uri` `file:///…/small.png` | none | | 743 |

The event is the base64 plus about 625 bytes.

### Large pictures

A result over about 1 MiB does not arrive as an image. The item's `content` becomes a single text
block holding the result's JSON with its middle cut out (the base64 kept on either side of the
marker, about 524,000 characters each, is shortened here):

```json
"result":{"content":[{"type":"text","text":"{\"content\":[{\"type\":\"image\",\"data\":\"iVBORw0KGgo…7342995 chars truncated…AAAAASUVORK5CYII=\",\"mimeType\":\"image/png\"}],\"isError\":false}"}],"structuredContent":null,"_meta":null}
```

The call still reads `status: "completed"` with `error: null`. The text is 1,048,501 characters
whatever the picture's size, so no event passed 1,049,122 bytes.

| Picture | MCP server's answer | Arrives as | `_raw_bytes` |
| --- | --- | --- | --- |
| 715,203 bytes | 953,730 bytes | image, same bytes | 954,229 |
| 781,113 bytes | 1,041,610 bytes | image, same bytes | 1,042,109 |
| 796,498 bytes | 1,062,126 bytes | text, 1,048,501 chars | 1,049,122 |
| 1,048,817 bytes (`big_picture` 1) | 1,398,550 bytes | text, 1,048,501 chars | 1,049,121 |
| 3,147,775 bytes (`big_picture` 3) | 4,197,162 bytes | text, 1,048,501 chars | 1,049,122 |
| 6,293,543 bytes (`big_picture` 6) | 8,391,518 bytes | text, 1,048,501 chars | 1,049,122 |

The limit lies between a 1,041,610-byte and a 1,062,126-byte answer. That it is 1 MiB of the
serialized result, and that it counts the whole result and not each block, are inferences. A
picture of about 780 kB is therefore the largest that arrives whole, well under Shaman's 5 MiB
attachment limit. The sized pictures between the fixtures are noise PNGs made with
`make_fixtures.noise` and served as `picture {kind: "png"}` from a fixtures directory of their own
(`codex/mcp-picture-png-<n>kib.jsonl`, named for the size aimed at).

## `view_image`

The item carries the path alone, the same at start and at completion (433 and 446 bytes):

```json
{"method":"item/completed","params":{"item":{"type":"imageView","id":"exec-5aad97ca-0ee4-46bb-9b7e-4c808b7db96e","path":"/private/var/folders/…/codex/view-image-png/work/small.png","startedAtMs":1795797356146,"completedAtMs":1795797356147},"threadId":"…","turnId":"…","completedAtMs":1795797356147},"emittedAtMs":1795797356148,"_raw_bytes":446}
```

- The path is absolute. Asked for `small.png`, the item names it under the thread's working
  directory; asked for an absolute path, it names that path.
- No notification carries the bytes, a media type or a size. The largest line in any `view_image`
  capture is 1,941 bytes, with a 6 MiB file viewed.
- PNG, JPEG, GIF, WebP and TIFF each produced the item, as did the 3 MiB and 6 MiB PNGs, at the
  default detail and at `detail: "original"`.
- BMP, HEIC and SVG produced **no item at all**, started or completed, and no error notification.
  The tool failed inside `exec` with `unable to process image: invalid or unsupported image data`,
  which only the model is told. A missing file is the same: no item, and
  `unable to locate image at …: No such file or directory`.

To show the picture a client has to read the file at `path` itself. Nothing says its kind: GIF and
TIFF files are named by the item like any other.

## Image generation

Not seen, and not for want of the feature (`codex features list` shows `image_generation` stable
and on). What was tried under the stand-in:

- The model's tool list (`codex/model-side.jsonl`, case `all-tools`) has no image generation tool.
- With the scripted provider marked `requires_openai_auth = true`, a made-up API key and the
  stand-in also answering `POST …/images/generations` (case `all-tools-image-service`), the tool
  was still not offered, and the scripted call failed with `tools.image_gen__imagegen is not a
  function` (case `image-generation`).
- An `image_generation_call` output item handed back as the model's answer, the older route
  (case `image-generation-call-item`), produced no item, no notification and no file.

Two things that are read, not seen. In the vendored source (`references/codex` at `33ffb6f9`) the
tool is `image_gen.imagegen`, run by Codex itself, and offered only when the sign-in uses the
ChatGPT backend and the plan is not the free one (`core/src/tools/spec_plan.rs`,
`image_generation_available`). And the schema the
installed binary writes (`codex app-server generate-json-schema`) gives the `imageGeneration` item
`id`, `status`, `revisedPrompt`, `savedPath`, `failure`, `generationId`, `inputImages`,
`transparentBackground`, and one of `result` (a string) or `fileId` (a string). So a completed
item may name a file id and carry no inline bytes at all. Whether `result` is base64, what kind and
size it is, and whether `savedPath` names a file with the same bytes all wait on a signed-in run:
`codex_capture.py image-generation --scratch … --fixtures …`.

## Resume

`thread/resume` answers with the whole thread, and each stored item is in
`result.thread.turns[].items[]` (`itemsView: "full"`). `thread/read` with `includeTurns: true`
answers the same items at the same path. Shaman reads the thread's id, model, reasoning effort and
service tier from the answer and none of its turns, but receives all of it.

| Thread | `thread/resume` answer | `thread/read` answer | The picture in history |
| --- | --- | --- | --- |
| `picture` png, 8,290 bytes | 14,736 bytes | 13,747 bytes | image block, same bytes |
| `two_pictures` | 17,102 | 16,111 | both image blocks, same bytes, same order |
| `resource_picture` | 14,784 | 13,785 | resource, same `blob` |
| 44,852-byte PNG | 63,437 | 62,436 | image block, same bytes |
| 53,283-byte PNG | 69,089 | 68,088 | text block, 65,461 chars |
| 781,113-byte PNG | 69,095 | 68,092 | text block, 65,461 chars |
| `big_picture` 6 | 69,157 | 68,158 | text block, 65,433 chars |
| `view_image` png | 3,402 | 2,415 | `imageView`, the path alone |
| `view_image` 6 MiB PNG | 3,460 | 2,459 | `imageView`, the path alone |

History keeps much less than the live event. A picture that arrived whole comes back as a text
block once its result passes about 64 KiB, cut the same way (`…976126 chars truncated…`), so a
resumed thread carries a picture again only when it is under about 48 kB. The limit lies between
a 59,930-byte and a 71,170-byte MCP answer; that it is 64 KiB is an inference. The 6 MiB result
comes back as the cut-down text of the already cut-down text. `thread/turns/list` with
`itemsView: "full"` (`result.data[].items[]`) and `thread/items/list` (`result.data[].item`)
answered the same blocks, cut the same way. The stored rollout holds the cut item too (a
66,018-byte line), so the cut is made when the thread is written.

Each resume also brought a `deprecationNotice`: "Full-history hydration is deprecated for
paginated threads; use `excludeTurns: true`, then page with `thread/turns/list` and
`thread/items/list`."

## Raw response items

Shaman does not ask for these. With `experimentalApi: true` and `experimentalRawEvents: true` on
`thread/start`, `rawResponseItem/completed` repeats what the model was handed. When the script
passed the picture on, that is a second copy in another shape, at `params.item.output[i]` of the
`custom_tool_call_output`, as a `data:` URL (11,701 bytes for the MCP PNG, beside the 11,681-byte
item; 11,811 bytes for `view_image`, whose item has no bytes):

```json
{"type":"input_image","image_url":{"_elided":"base64","chars":11078,"bytes":8290,"sha256":"eebf856e016f41071cf75022c381da134f0749e5a8cf753eeedfcb3888f2946c","sniffed":"image/png","head":"iVBORw0KGgoAAAANSUhEUgAA","data_url_prefix":"data:image/png;base64,"},"detail":"high"}
```

## What the model is handed

A side observation from the requests the stand-in received (`codex/model-side.jsonl`), where the
script passed each picture on with `exec`'s `image()` helper. It bears on which kinds Codex itself
can decode.

| Kind | From an MCP image block | From `view_image` |
| --- | --- | --- |
| PNG, JPEG, WebP | `input_image`, `data:` URL, same bytes | same |
| GIF | re-encoded as a 607-byte PNG | same |
| TIFF | re-encoded as a 661-byte PNG | same |
| BMP, HEIC, SVG | text: `image content omitted because it could not be processed` | the tool fails |
| 1, 3 and 6 MiB PNG | same bytes, in a request of up to 8,440,245 bytes | same bytes, 8,440,307 bytes for 6 MiB |

So the 1 MiB cut applies to what the client is told, not to what the model is sent.

## Not seen or tried

- Any turn on the real account, with a real model choosing the call. Whether a real model's use
  of `exec` yields the same items is inferred from the tool being Codex's own.
- Image generation: the item as started and completed, inline base64 or `fileId`, kind, size,
  `savedPath` and its file.
- Another built-in tool that returns a picture. The tool list under the stand-in had none
  (`apply_patch`, `exec_command`, `write_stdin`, `view_image`, the goal, MCP resource and sub-agent
  tools). The browser and computer-use tools of the user's own Codex home were not loaded.
- A model without code mode, where MCP and `view_image` would be called directly and not through
  `exec`.
- An MCP result marked `isError`, an `audio` block, a `structuredContent` picture, and a picture
  from a streamable-HTTP MCP server such as the Broker.
- Whether the large-result limits are configurable.
- Windows and Linux.
