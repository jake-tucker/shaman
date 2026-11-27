# Copilot stops asking before a write once writes are approved for the Session, and cuts a large diff short on completion

Recorded 2026-11-27 against GitHub Copilot CLI 1.0.114 on macOS, driven through copilot-sdk 2.0.1
by `examples/copilot_write_report_capture.rs`, on `claude-haiku-4.5` (whose file tools are `edit`
and `create`) and `gpt-5.6-luna` (whose file tool is `apply_patch`). The captures are beside this
file in `0790-copilot-write-reports/`, one JSON line per event.

Asked by [See what Copilot reports for a write it does not ask about](https://github.com/jake-tucker/shaman/issues/790),
for ADR-0085, which draws a File Change's diff from what its Provider reports.

## What was run

The capture seeds a scratch directory, answers permission requests as `CopilotApprovals` does
under a posture, and asks the Model for a handful of writes. A request the handler leaves to the
reader is decided afterwards through `permissions.handlePendingPermissionRequest`, once each or,
for the first write, with an approval of writes for the Session.

| Capture | Model | Posture | A request left to the reader gets | Script |
| --- | --- | --- | --- | --- |
| `edit-ask-once` | `claude-haiku-4.5` | Ask | approved once | writes |
| `edit-ask-session` | `claude-haiku-4.5` | Ask | writes approved for the Session | writes |
| `edit-allow-all` | `claude-haiku-4.5` | Allow all | (none is left) | writes |
| `edit-auto-once` | `claude-haiku-4.5` | Auto | approved once | writes |
| `edit-auto-session` | `claude-haiku-4.5` | Auto | writes approved for the Session | writes |
| `edit-large-ask-session` | `claude-haiku-4.5` | Ask | writes approved for the Session | large |
| `patch-ask-once` | `gpt-5.6-luna` | Ask | approved once | writes |
| `patch-ask-session` | `gpt-5.6-luna` | Ask | writes approved for the Session | writes |
| `patch-large-ask-session` | `gpt-5.6-luna` | Ask | writes approved for the Session | large |
| `patch-limits-ask-once` | `gpt-5.6-luna` | Ask | approved once | limits |
| `patch-limits-ask-session` | `gpt-5.6-luna` | Ask | writes approved for the Session | limits |

- **writes**: create a file, edit a file twice, edit a file that is not UTF-8, change several files
  at once (one `apply_patch` call that updates two, adds one and deletes three, one of them 64 KiB
  and one not text; `edit` and `create` take a call a file and delete nothing), and create a file
  outside the working directory.
- **large**: create one file of 420 lines, a diff of 23 KiB.
- **limits**: `apply_patch` deletions whose diffs are 19.7 KiB, 21.7 KiB, and three of 8.5 KiB in
  one call.

## Whether Copilot still asks

| Posture, as Shaman sets it | Before writes are approved for the Session | After |
| --- | --- | --- |
| Ask (mode `manual`, handler answers nothing) | Every write asks. | No write asks. |
| Allow all (mode `manual`, handler approves once) | Every write asks, and the handler approves it. | Shaman never sends that approval under this posture. |
| Auto (mode `assisted`) | Every write asks, with the judge's verdict. | No write asks, and the judge is not consulted. |

- Once a write has been answered `{"kind":"approve-for-session","approval":{"kind":"write"}}`,
  Copilot makes every later write with no request at all: the handler is not called and no
  `permission.requested` event is sent. `tool.execution_start` is followed by
  `tool.execution_complete` within a few milliseconds. Seen for `edit`, `create` and
  `apply_patch`, and for a file outside the working directory.
- The approval takes effect inside the call it answers: an `apply_patch` over four files asked for
  the first, was approved for the Session, and wrote the other three unasked.
- `apply_patch` asks once a file, not once a call. A call over six files sent six requests, one
  after another, all carrying the call's `toolCallId`, each with its own file's `fileName` and
  `diff`.
- Under Auto the judge approved seven of the eight writes and held one back (`requireApproval`, for
  the file that is not UTF-8). A write the handler approved on the judge's word still asked the
  next time.

## What a completion carries

`tool.execution_complete` for `edit`, `create` and `apply_patch` carries, whether or not the write
asked:

- `result.detailedContent`: the diff, described below.
- `result.content`: a sentence for the Model (`File … updated with changes.`, `Created file … with
  17 characters`, `Added 1 file(s): … Modified 2 file(s): … Deleted 3 file(s): …`). No diff.
- `fileEdits`: one `{path, kind}` a file, `kind` being `create`, `edit` or `delete`. No content.
- `toolTelemetry.metrics.linesAdded` and `linesRemoved`: totals for the call, not per file.

Where the write asked, `detailedContent` is byte for byte the request's `diff`; for an
`apply_patch` over several files it is each file's `diff` in the order asked, joined by one line
feed. Where the write did not ask, `detailedContent` is byte for byte what the same write carried
in the capture that did ask (compared across each pair of captures, with the scratch path made
the same). So below the size where it is cut short, the completion alone is enough.

## The form of the diff

One block a file, each beginning with an empty line. Paths are absolute, so `a` and `b` run
straight into them. From the captures, with the scratch directory written `/S`:

An edit, by `edit` or `apply_patch`:

```
⏎
diff --git a/S/work/notes.txt b/S/work/notes.txt
index 0000000..0000000 100644
--- a/S/work/notes.txt
+++ b/S/work/notes.txt
@@ -1,7 +1,7 @@
 # notes
 colour = red
 size = 1
-shape = square
+shape = round
 weight = heavy
 last = line
 
```

A created file, by `create`:

```
⏎
diff --git a/S/work/created.txt b/S/work/created.txt
create file mode 100644
index 0000000..0000000
--- a/dev/null
+++ b/S/work/created.txt
@@ -1,0 +1,3 @@
+alpha
+beta
+gamma
```

A created and a deleted file, by `apply_patch`:

```
⏎
diff --git a/S/work/added.txt b/S/work/added.txt
create file mode 100644
index 0000000..0000000
--- a/dev/null
+++ b/S/work/added.txt
copilot:whole-file-v1
@@ -1,0 +1,2 @@
+one
+two
⏎
⏎
⏎
diff --git a/S/work/old.txt b/dev/null
index 0000000..0000000 100644
--- a/S/work/old.txt
+++ b/dev/null
copilot:whole-file-v1
@@ -1,2 +0,0 @@
-to be deleted
-second line
⏎
```

What a reader of it has to allow for:

- It is a unified diff with file headers, hunk headers that number lines, and three lines of
  context. It is not quite git's: a created file says `create file mode` where git says
  `new file mode`, its hunk says `-1,0` where git says `-0,0`, a deleted file has no
  `deleted file mode` line, the `index` line's hashes are zeros, and `/dev/null` is written
  `a/dev/null` and `b/dev/null`.
- A file `apply_patch` adds or deletes has a line `copilot:whole-file-v1` between the `+++` line
  and the first hunk. `create`'s diff of a new file has none.
- A hunk that reaches the end of a file ending in a line feed counts one more line than the file
  has and ends with a context line holding a single space: `notes.txt` is six lines and its hunk
  says `-1,7 +1,7`.
- `\ No newline at end of file` appears where git would put it.
- A byte that is not UTF-8 arrives as U+FFFD, in a file's context as well as its changes. The file
  on disk kept its byte. A file that is not text at all (the 11 bytes of a PNG header) is diffed as
  if it were, with U+FFFD and control characters in its lines.

## Where the completion's content is cut short

`detailedContent` is cut once the whole of it passes about 20 KiB, whatever tool made the write.
The request's `diff` is never cut: a 64.8 KiB deletion arrived whole in its request.

| `detailedContent` whole | What arrived |
| --- | --- |
| 19,721 bytes, one file | whole |
| 21,759 bytes, one file | cut |
| 23,427 bytes, one file by `create` | cut |
| 25,511 bytes, three files of 8,503 each in one call | cut |
| 68,758 bytes, six files in one call | cut |

- The telemetry beside each completion names the threshold: `large_output_threshold_bytes: 20480`.
  A cut one also has `largeSessionLogWrittenToFile: "true"` and
  `largeSessionLogOriginalSizeBytes`, the size it was.
- What is cut is the first 5,000 characters, ending wherever that falls (mid line, and for several
  files mid file), then an empty line and
  `[Truncated — full output (22.9 KB) temporarily saved to /var/folders/…/T/<ms>-copilot-tool-output-<hex>.txt]`.
  That file was gone once the capture ended; whether it outlives the tool call was not looked at.
- The cut is on the call's total, so three files that would each arrive whole alone are cut
  together.
- `tool.execution_start` still carries the tool's whole input, uncut, for every write: `file_text`
  for `create`, `old_str` and `new_str` for `edit` (with no place in the file), and for
  `apply_patch` its patch text (`*** Update File:`, hunks with no line numbers, and for a deleted
  file its path alone).

## What a request carries that a completion does not

`permissionRequest` for a write, beside `diff` and `fileName`:

- `fileEdits`: one entry, for the request's own file, with `before` and `after` as `{path,
  content}` holding the whole text. A created file has `after` alone and a deleted one `before`
  alone. For an `apply_patch` over several files each request has its own file's entry, never the
  call's.
- `fileEdits` is absent for the file that is not UTF-8 and for the one that is not text, edited or
  deleted. `diff` is still there, in the form above.
- `newFileContents`: the whole text after the write, on every request, an edit's included; empty
  for a deletion. Bytes that are not UTF-8 are U+FFFD in it.
- `intention` is `Create file`, `Edit file` or `Delete file`.
- `promptRequest` repeats the same fields beside `permissionRequest`.

## What this leaves for a File Change's diff

- The rule stands and gains one case. A request's `diff` where one arrives is whole and is what the
  completion would say. A completion's `detailedContent` reads as a diff unless it ends in the
  `[Truncated — …]` line, and that line is how a cut one is told.
- A cut completion with no request is the one case with no whole diff: a write past 20 KiB of diff
  made after writes were approved for the Session, under Ask or Auto. Under Allow all, and under
  Auto while the judge approves, every write asks and its request holds the whole diff.
- An `apply_patch` over several files gives one request a file but one completion for the call, so
  a completion's content is split at each `diff --git` line to give a file its patch.
- Counts of lines added and removed a file come from its patch; the telemetry's are for the call.

## Beside the question

The judge's verdict was not where Shaman reads it. `CopilotApprovals::judge_approves` reads
`permissionRequest.assistedApproval`. Every request in the two Auto captures carried the verdict
as `promptRequest.assistedApproval` (`recommendation`, `reason`, `model`, `evaluation`) and, in
brief, as `assistedApprovalAssessment` beside the two requests (`status`, `recommendation`,
`rationale`); `permissionRequest` had no `assistedApproval`. The capture reads the verdict where it
arrived. Read where Shaman reads it, every request under Auto would be left to the reader.

## Not seen

- Windows and Linux, where an absolute path in `diff --git a…` has another shape.
- A write that was declined, or failed after it was approved.
- Copilot's own `allow-all` mode, which Shaman does not set.
- A Model family other than those two, and so any file tool other than `edit`, `create` and
  `apply_patch`.
- A managed policy, a sandbox, or a native Subagent's writes.
- Whether the threshold is 20,480 bytes exactly: the nearest sizes tried were 19,721 and 21,759.
