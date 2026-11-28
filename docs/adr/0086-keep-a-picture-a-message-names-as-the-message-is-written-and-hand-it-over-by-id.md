# Keep a Picture a Message names as the Message is written, and hand it over by id

_Follows ADR-0085, which keeps a File Change's diff as a record of the edit and not of the file, and is held to ADR-0078 where it adds to what a Peer is answered._

An Agent shows its reader an image by writing a file's path as a Markdown image in a Message. As that Message stops being written, the Server reads each file it names, once, and keeps the bytes beside the Session, as it keeps an Attachment's. The Message then says what became of each name, and a Client reads a kept Picture by its id over the route that already hands over an Attachment. No route reads a file by a path its asker gives.

The files an Agent points at are the ones least likely to last: a screenshot in a temporary directory, an image a Provider generated under its own home, a chart in a Managed Worktree that is later Reclaimed. A Transcript is a record, and a Picture read later would show the file as it had become, or nothing.

- **Whose Messages.** An Agent's alone, a Subagent's among them in its own Session, and a Superseded Message's as any other's. A path written in a Prompt is never read.
- **Which files.** Any regular file the Server's user can read, wherever it stands, with symlinks followed. Nothing holds it to the Worktree or the Workspace. A device, a pipe and a directory are not read.
- **What counts as an image.** The four kinds an Attachment may be (PNG, JPEG, GIF, WebP), told by the file's first bytes and never by its name, and 5 MiB at most.
- **How a name is read.** By the Server alone, which knows its own platform. An absolute path is read as written. `~` is the Server's home, and `~user` is not read. A `file:` URL with no host, or `localhost`, is its path with the percent-encoding undone. A plain path is tried as written and then with its percent-encoding undone, where that differs. Any other scheme is never read, and one letter before a colon is a drive and not a scheme.
- **What a relative path is read from.** The Session's Execution Directory as the Message stops being written. Where no file is there, the root of the Worktree holding that directory. Where both hold one, the Execution Directory's is the Picture.
- **When.** Once, as the Message stops being written, however its Turn settled, so a Message cut off by an interrupt keeps its Pictures too. All of one Message's files are given five seconds together. Nothing is read again: a file that appears afterwards yields no Picture, and neither does any Message written before this.
- **How much.** The first 16 names of a Message, a name written twice counting once, and 20 MiB together, taken in order. The same bytes named twice, in one Message or several, are stored once.
- **What the Message carries.** One optional list with an entry for each name as it was written. A kept Picture's entry holds its id, its kind, its length in bytes and its width and height, so a Client holds its place before a byte arrives. One not kept holds a reason: no file there, not an image of those kinds, too large, could not be read, or too many for one Message. A Client matches what it draws to an entry by the name as written.
- **How the names are found.** The Server reads the Message as CommonMark, as the TUI does, and takes every image's destination, a reference-style one included, and none from inside code.
- **What a name with no entry is.** A link, as a URL is. That covers a Message still being written, a Message from before this, and a Serving Server that does not yet keep Pictures, of which a reader is told nothing.

## Considered Options

- **A route that reads the file when a Client asks**, which is what was first chosen when deciding how pictures are shown. Rejected: it shows the file as it now stands or not at all, where the files in question are short-lived. The size is unknown until the bytes arrive, so a row jumps. A relative path has to be read from where the Session stood when the Message was written, which only the Moves in the Transcript can say. And a Peer would gain a route that reads a file by path with no Turn and no Approval, which then needs a rule about which paths.
- **Only files inside the Session's Worktree or Workspace.** Rejected: it refuses the commonest cases, a screenshot in a temporary directory and Codex's generated images under `~/.codex`. It protects little, since a Peer may already begin a Session under any Approval Posture and answer its own Approvals.
- **Reading as each name streams in.** Rejected: a name can arrive before its file is finished, and a Message is rewritten as it streams.
- **The bytes inside the Message.** Rejected as it was for a diff: a Client opening a Session is handed its whole Transcript, so every Client would carry every Picture.
- **The Client reading its own disk.** Rejected when deciding how pictures are shown: a Remote's file is not on the Client's disk.

## Consequences

- A Picture outlives its file. Deleting the file does not take the Picture out of the Transcript; deleting the Session does.
- A misled Agent can make any image its Server's user can read appear in its own reader's Transcript, and so be kept beside the Session. It goes nowhere else: a URL is never fetched, so nothing is sent to another host.
- The Server reads Markdown, which until now only a Client did. Where a Client's renderer sees an image the Server did not, it is a link; where the Server kept one a Client does not draw, the bytes are kept unseen.
- The Session database grows by the Pictures it keeps, bounded for each Message and not for a Session. Pictures nobody opens are kept all the same.
- The list joins what a Peer is answered and changes by addition only from then on. The reason is an enum with a fallback for a kind a Client does not know.
- The route that hands over an Attachment now hands over a Picture too, by an id that is the hash of its bytes and is scoped to no Session, as an Attachment's already is.
- The TUI and the phone are handed the list and draw nothing from it yet; the TUI keeps writing the name out.
- A picture a Tool Call returned has a place to be kept in. How it is carried is decided apart from this.
- The limits (16 names, 20 MiB, five seconds) were chosen in conversation and never measured.
