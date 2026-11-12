# Copilot streams a shell's output twice, once as chunks and once as snapshots

Recorded 2026-11-03 against GitHub Copilot CLI 1.0.105 on macOS, driven through
github-copilot-sdk 1.0.29-preview.5 by `examples/copilot_tool_output_capture.rs` on
`claude-haiku-4.5`.

## The symptom

A running Copilot Command showed its output repeated: a shell printing `one`, `two` and `three`
streamed into the Transcript as `one one two one two three one two three`, and settled on the
same, because Shaman appended every `tool.execution_partial_result` it received as a new piece of
output.

## What the CLI sends

The capture ran `printf 'one\n'; sleep 2; printf 'two\n' >&2; sleep 2; printf 'three\n'` in a
synchronous shell, then `glob` and `web_fetch`. The shell's events, in stream order:

| ms | Event | Carries |
| --- | --- | --- |
| 5910 | `tool.shell_output` | `stdout`, `"one\n"`, sequence 0 |
| 5910 | `tool.execution_partial_result` | `"one\n"` |
| 7918 | `tool.shell_output` | `stderr`, `"two\n"`, sequence 1 |
| 7920 | `tool.execution_partial_result` | `"one\ntwo\n"` |
| 9926 | `tool.shell_output` | `stdout`, `"three\n"`, sequence 2 |
| 9926 | `tool.execution_partial_result` | `"one\ntwo\nthree\n"` |
| 9927 | `tool.execution_partial_result` | `"one\ntwo\nthree\n"` |
| 9934 | `tool.execution_complete` | `"one\ntwo\nthree\n<shellId: 0 completed with exit code 0>"` |

- `tool.shell_output` is append-only: each chunk is new output, tagged with the stream it came
  from. The SDK documents it as not persisted and not replayed to late subscribers.
- `tool.execution_partial_result` is everything the shell has printed so far, repeated whole
  each time and once more as the shell exits. The SDK deprecates it as a "merged replacement
  snapshot of shell output" in favour of `tool.shell_output`.
- `glob` and `web_fetch` sent neither: only a start and a completion.

The older captures in `tests/copilot_integration/fixtures/` show the same snapshot sent twice
(`"one\n"`, `"one\n"`) on the CLI they were recorded on, so the doubling predates 1.0.105.

## What Shaman does

The Copilot projection streams a running Command's output from `tool.shell_output` and takes
nothing from `tool.execution_partial_result`. Its completion still adds whatever the result
reports beyond what streamed.

## The marker closing a shell's result

A shell's result ends with a marker addressed to the Model, which reached the output a reader
sees until Shaman took it out. A second capture on the same CLI ran shells ending in each way:

| Shell | `success` | Result | `shell_exit` |
| --- | --- | --- | --- |
| `printf 'no-newline'` | true | `"no-newline\n<shellId: 0 completed with exit code 0>"` | shell 0, exit 0 |
| `true` | true | `"\n<shellId: 1 completed with exit code 0>"` | shell 1, exit 0 |
| `echo failing; exit 3` | true | `"failing\n<shellId: 2 completed with exit code 3>"` | shell 2, exit 3 |
| `read_bash` on a finished shell | true | `"bg-out\n<shellId: 4 completed with exit code 0>"` | shell 4, exit 0 |

- The marker stands on a line of its own: Copilot breaks the line itself after output that left it
  open, including output that was empty.
- The `shell_exit` part names the same shell and exit, so Shaman spells the marker from it and takes
  off exactly that, along with a line break the stream never carried. A result with no such part,
  or one ending in anything else, is output, even if it reads like a marker.
- A Command keeps the exit as its exit status. A `read_bash` is a Tool Call, which has no exit
  status, so what it read shows without the exit.
- The other markers are still output: `<command started in background with shellId: 4>` and
  `<command with id: 5 stopped>`, which are all such a Tool's result says.

Not verified: whether an MCP Tool's execution sends either event. No MCP server was configured
for the capture, and the SDK describes both events as shell output only.
