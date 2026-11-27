# Record a File Change's diff from what its Provider reports, and answer for it when asked

_Follows ADR-0084, which has both Clients draw one model, and is held to ADR-0078 where it adds to what a Peer may ask. Leaves the TUI and the phone showing a File Change's paths alone._

A File Change holds a diff for each of its files: the change as its Provider reported it, kept by the Server from then on. The desktop app draws it under the File Change's row, in a sheet over the window, and inside an Approval of that change. A diff is of the edit as it was made, and it stays that edit however the file moves on afterwards, because a Transcript is a record and the row says what happened then.

Every Provider already reports this content and Shaman dropped all of it, keeping the paths. Nothing else Shaman has could stand in for it: a Session records no commit it began from, nothing computes a content diff, and a Workspace may be a directory under no source control at all.

- **Whose content it is.** The Provider's report, turned into one form by each adapter. The Server reads a file itself in one case: Claude asking before a write, where the request holds the old and new strings and no place in the file. The file is known to be untouched then, so the Server finds the string in it and builds the hunks. Where it cannot be found, the File Change holds no diff until it completes.
- **Its form.** One unified patch for each file, as unified diff text, with the context its Provider gave: one line from Codex, three from Claude. A created file is all additions and a deleted one all removals. Beside the patch stand the counts of lines it added and removed.
- **When it is recorded.** Twice at most. First when the Provider reports the change before writing, which is what an Approval of it is drawn from, and again on completion, when what the Provider reports then takes its place. A File Change that was declined, failed or was stopped keeps the diff it proposed, and its status says the change was not made. What a Provider streams while the change is still being written stays dropped.
- **An Approval holds none.** It names the row it gates, and a reader is shown that row's diff. An Approval that names no row shows paths alone.
- **Where it is kept and how it travels.** Beside the Activity, not in it, and deleted with its Session. A Client asks for one File Change's diffs when a reader opens it, by a route a Peer may ask, since a Remote's Session is read through the proxy. The Activity carries only what a closed row needs: for each file, whether a diff was kept and the two counts.
- **What is too large.** A patch past 256 KiB for one file, or past 1 MiB for one File Change with its files taken in order, is not kept. That file keeps its counts, and its place reads that the diff is too large to show.
- **A File Change with none.** One recorded before this, one whose content is not text, one its Provider said nothing of: it shows its paths alone and does not open. Nothing is filled in afterwards from the Worktree as it stands.

What each Provider reports, as found when this was decided:

| | Before the write | After the write |
|---|---|---|
| Codex | The item holds a unified diff for each file, with line numbers; a created or deleted file comes as its whole text. An Approval names that item. | The same, on the completed item. |
| Claude | An Approval's request holds the old and new strings, or a new file's whole text. | The result holds hunks with line numbers and the whole original file. |
| Copilot | The permission request holds a unified diff and, where it can, the whole texts before and after. | A field said to preserve diffs, whose form was not seen. |

## Considered Options

- **The Workspace's source control, asked when a reader opens the change.** Rejected: it can only say how a file stands now against some base, so every File Change of one file would open onto the same diff, a change not yet made would have none, and neither would a Workspace outside source control. It also cannot rebuild an uncommitted state a later edit has written over.
- **Both whole texts, before and after.** They would let a fold between two hunks open onto more of the file. Rejected for now: Codex never reports both for an update, so the Server would read the file after every edit and work backwards, and a one-line change to a long file would store the file twice.
- **The content inside the Activity.** No new route, and the diff there as a row opens. Rejected: a Client opening a Session is handed its whole Transcript at once, so every Client would carry every diff, the phone among them though it draws none, and the Server rewrites an Activity's stored form each time it changes.
- **The Server's own reading of each file**, as a File Change starts and as it completes, for every Provider. Rejected: it races a Provider that writes at once, and sees nothing of a change that was declined.

## Consequences

- A file changed by a Command, such as a formatter or `sed`, has no File Change and so no diff. Codex's diff of a whole Turn covers its patch tool alone and stays dropped. Whether the Aside lists what a Command changed is decided apart from this.
- The diff is what the Provider said it did. Where a Provider misreports, so does the diff, and nothing checks it against the file.
- The same edit reads with more or less context by Provider, and the folds between hunks do not open.
- The route and the counts join what a Peer may ask and change by addition only from then on. A Serving Server from before them answers that it has no such route, which a Client treats as a File Change with no diff. The phone's wire carries counts it does not show.
- The Session database grows by the patches it keeps, bounded by the two limits.
- A reader with an Approval's diff open may see it replaced as the change completes, so a Client asks again when the Activity says its diffs changed.
- Copilot's content after a write is taken only where it reads as a diff, and whether Copilot still asks before a write it was told to allow for the Session was not seen. Where neither gives a diff, the File Change holds none.
