# Keep a Picture a Tool Call returned as it arrives, and go on counting it among the parts not shown

_Follows ADR-0086, which keeps a Picture a Message names where an Attachment's bytes are kept, and is held to ADR-0078 where it adds to what a Peer is answered._

A Tool hands its Agent an image: a browser's screenshot, a file the Agent looked at, a picture its Provider generated. Every adapter dropped such a part and kept a count of it. Now, as a Tool Call settles, the Server keeps each image its result returned beside the Session, as it keeps a Message's Picture, and the Tool Call says what became of each. A Client reads a kept one by its id over the route that already hands over an Attachment.

It is kept as it arrives because no Provider gives it back. Claude and Copilot replay nothing on a resume, and a resumed Codex thread returns a tool's picture only under about 48 kB.

- **What is a Picture.** A part of the result that hands over bytes which are PNG, JPEG, GIF or WebP by their first bytes, never by the media type claimed for them: an image block, and an embedded resource whose blob is one. 10 MiB at most (10,485,760 bytes), whatever its size in pixels. A resource link hands over no bytes and is not one. A failed Tool Call's is kept as any other's.
- **The cap is every Picture's.** A file a Message names may be 10 MiB too, which amends ADR-0086. An Attachment stays at 5 MiB, since that is what a Provider accepts from a user. The number is the edge past which Copilot drops a tool's picture, so everything Copilot hands over fits.
- **How much.** The first 16 of one Tool Call and 20 MiB together, taken in order, as for a Message. The same bytes twice are stored once. Nothing limits a Session.
- **When.** The bytes are stored and joined to the Session before the Tool Call's completion is published, so no Client is handed an id it cannot fetch. A native Subagent's Tool Call binds to the Subagent's own Session. A Tool Call that never completes holds none. A Picture is kept as long as its Session, whether or not a reader's Setting shows Tool Calls.
- **Claude.** The block in the tool result of the `user` event. The copy beside it in `tool_use_result` is not read. What arrives is whatever its CLI made of the file.
- **Copilot.** The tool's own bytes in `contents`. The built-in `view` sends only the copy made for its Model, so that is kept for `view` alone. Nothing in the entry says which copy it is. The three `model.*` events that repeat the picture are never read.
- **Codex, an MCP tool.** The block as the tool sent it. A result past about 1 MiB arrives as one text block holding the result's JSON with its middle cut out, on a call marked completed. The adapter knows that shape narrowly: a completed MCP call whose only block is text that opens as a result's JSON and holds Codex's truncation marker. Where that text opens with an image block, the Tool Call has no output and one entry, too large. Otherwise it stays text, marked truncated.
- **Codex, `view_image`.** The item names an absolute path and nothing else, so the Server reads the file, once, as the item completes, by ADR-0086's rule for a file and within its five seconds.
- **Codex, a generated picture.** The bytes in `result` are decoded and kept. An item with no `result` has the file `savedPath` names read by `view_image`'s rule, and one with neither says it could not be read. The Tool Call's output stays the saved path, the one place a reader learns where the file is.
- **What the Tool Call carries.** One optional list, arriving with its completion, in the order the result held them. An entry has the shape of a Message's: a kept Picture's holds its id, its kind, its length in bytes and its width and height; one not kept holds a reason, from the one enum both lists share: no file there, not an image of those kinds, too large, could not be read, or too many. Every picture part has an entry, those past the 16th too. Bytes that cannot be decoded or stored leave "could not be read", and the Tool Call completes all the same.
- **What the count counts.** What it did: every part of the result that was no text, a Picture among them. The adapters are brought to agree on it. Copilot's `view`, Codex's `view_image` and Codex's generated picture each count one, where they counted none. Claude's `tool_reference` block is plumbing and stops being counted; a PDF's `document` block stays counted.
- **What a Client draws.** The desktop app draws a kept Picture under its Tool Call's opened row, after the output; where it stood among the text is not kept. An entry not kept is a line of words there, entries with one reason sharing a line, and the line saying how many parts were not shown counts only what has no entry. A Picture past what the app's renderer draws, 8,192 px a side today, is a frame saying it is too tall or too wide to show, and its bytes are not fetched. The TUI, the phone and a Sidekick's reading of a Session go on saying the count.
- **A Tool Call with no list.** It reads by its count, as before. That covers every Tool Call from before this and a Serving Server that does not yet keep Pictures.

## Considered Options

- **Asking the Provider when a reader opens the row.** Rejected: none can be asked.
- **An Attachment's 5 MiB**, which was first chosen. Rejected: Copilot hands a tool's picture over whole up to 10 MiB, and at 5 the tool's own bytes would give way to a copy shrunk for the Model. Raising a Tool Call's alone would leave two kinds of Picture with two caps, so every Picture's was raised.
- **Copilot's copy for the Model, always.** Rejected: it is shrunk past about 3 MiB or 2000 px a side, and a GIF comes back a PNG.
- **A count of only what has no entry.** Rejected: a Client that draws none, an older one across a Pairing among them, would say nothing was left out.
- **Entries for the first 16 alone, the rest falling into the count.** Rejected: a reader could not tell a picture that was one too many from a part that was never a picture.
- **A limit in pixels at the Server.** Rejected: the Server knows nothing of what a Client's renderer draws, and another Client or a later renderer may draw what this one cannot.
- **An allowance for each Session.** Not taken for now. It would hold for a Message's Pictures too.

## Consequences

- The Session database grows faster than a Message's Pictures made it. A browser's viewport is 0.13 to 0.62 MiB, so a Session that takes one each step for 500 steps adds 60 to 300 MiB, until the Session is deleted.
- A whole page's screenshot is kept where it arrives and is not seen in the desktop app's first cut, being tens of thousands of pixels tall. From Codex it does not arrive at all past about 780 kB, and reads as too large.
- The TUI and the phone say one part was not shown for a Copilot `view` and a Codex `view_image`, where they said nothing.
- `view_image` has the Server read a file its Agent chose, as a Message's name does, with the consequence ADR-0086 records: a misled Agent can make any image its Server's user can read appear in its own reader's Transcript.
- Two readings are fitted to Codex 0.167.0 and are looked at again when its pin moves: the cut result's shape, and an item that carries a `fileId` in place of its bytes, which the schema promises and no capture held.
- A resumed Codex thread is sent each generated picture again. It is stored once, by its hash, and costs the transfer.
- One event from Copilot holding a 10 MiB picture is 18 MB. The adapter decodes one copy from it.
- The list joins what a Peer is answered and changes by addition only from then on. The reason is the enum a Message's list already has, with its fallback for a kind a Client does not know.
- A Client's cap on what it fetches by the Attachment's route rises to 10 MiB.
- Nothing ran through an adapter, and the limits were chosen in conversation and never measured.
