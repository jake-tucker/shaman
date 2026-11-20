#if DEBUG
  import Foundation
  import ShamanCore

  /// Two long Sessions on the workstation, as a real one of a long day reads: hundreds of items of very uneven
  /// height, short bubbles beside long Markdown answers with code blocks and tables, Turn cards and Compactions. One
  /// has settled; in the other the last answer is still streaming in, growing a paragraph or a few lines of code at a
  /// time.
  enum FakeLong {
    /// The settled one.
    static let settled = "workstation-key/30"
    /// The one whose last answer still streams.
    static let streaming = "workstation-key/31"

    /// The Turns before the last.
    static let turns = 80

    /// The Sessions the `longSession` phone's listing carries beside the `paired` ones.
    static let listed: [FakeSession] = [
      FakeSession(
        id: settled, remote: "workstation", workspace: "shaman", title: "Port the transcript to the phone", glyph: nil,
        checkout: CheckoutLine(glyph: nil, words: "feature/phone-transcript"), standing: .done, running: 0, moved: 3,
        settled: false, reads: .long),
      FakeSession(
        id: streaming, remote: "workstation", workspace: "shaman", title: "Profile the transcript on the phone",
        glyph: nil, checkout: CheckoutLine(glyph: nil, words: "feature/phone-transcript"), standing: .working,
        running: 600, moved: 2, settled: false, reads: .longWorking),
    ]

    /// The conversation, the last answer streaming where `streaming`, standing as it does at first.
    static func items(streaming: Bool) -> [ConversationItem] {
      var items: [ConversationItem] = []
      for turn in 0..<turns {
        items.append(FakeConversation.user("long-prompt-\(turn)", "\(prompt(turn)) (#\(turn))"))
        if turn % 3 == 1 {
          // A Turn steered halfway: work, a short word, the steer, then the rest of its work.
          items.append(
            FakeConversation.card(
              "long-work-\(turn)-a", .stopped, "Stopped after \(turn % 50 + 9)s", "1 tool · 1 command"))
          items.append(agent("long-aside-\(turn)", "Looking at `src/transcript/\(file(turn)).rs` first (#\(turn))."))
          items.append(
            FakeConversation.user("long-steer-\(turn)", "Keep going, but skip the snapshot tests. (#\(turn))"))
        }
        items.append(
          FakeConversation.card(
            "long-work-\(turn)", turn % 11 == 10 ? .failed : .worked,
            "Worked for \(turn % 7 + 1)m \(String(format: "%02d", (turn * 7) % 60))s",
            "\(turn % 5 + 1) tools · \(turn % 3 + 1) commands · \(turn % 4) edits"))
        if turn % 11 == 10 {
          items.append(
            ConversationItem(
              id: "long-error-\(turn)", kind: .error(text: "The provider stopped answering: connection reset by peer")
            ))
        }
        items.append(agent("long-answer-\(turn)", answer(turn) + "\n\nThat was answer #\(turn)."))
        if turn % 17 == 16 {
          items.append(
            ConversationItem(
              id: "long-compaction-\(turn)",
              kind: .compaction(
                words: "Compacted context · 180K → 40K (automatic)", tone: .succeeded, detail: "compaction")))
        }
      }
      items.append(FakeConversation.user("long-last", "Last one: write up what we changed, with the numbers."))
      if streaming {
        items.append(FakeConversation.card("long-last-work", .working, "Writing the summary", "1 tool"))
        items.append(agent("answer", streamed(0), streaming: true))
      } else {
        items.append(FakeConversation.card("long-last-work", .worked, "Worked for 2m 40s", "3 tools · 1 command"))
        items.append(agent("long-last-answer", answer(turns)))
        items.append(
          agent("long-closing", "That's everything. The last item of this Session is this line.", model: nil))
      }
      return items
    }

    /// How many times the streaming answer grows after it first stands.
    static let growths = 40

    /// The streaming answer after it has grown `step` times: a paragraph, or a few lines of a code block, each time.
    static func streamed(_ step: Int) -> String {
      var parts = ["## Summary\n\nThe transcript now draws on the phone at full length."]
      for index in 0..<step {
        if index % 4 == 3 {
          parts.append("```rust\n" + code(index, lines: 4) + "\n```")
        } else {
          parts.append(
            "Step \(index + 1): measured the scroll with \(index * 37 % 400 + 100) items, and the frame time held at "
              + "\(index % 5 + 6)ms across the whole run, with no dropped frames while it streamed.")
        }
      }
      return parts.joined(separator: "\n\n")
    }

    private static func agent(_ id: String, _ markdown: String, model: String? = nil, streaming: Bool = false)
      -> ConversationItem
    {
      ConversationItem(id: id, kind: .agentMessage(markdown: markdown, model: model, streaming: streaming))
    }

    private static let files = ["projection", "markdown", "cards", "scroll", "anchor", "heights", "rows", "layout"]

    private static func file(_ turn: Int) -> String {
      files[turn % files.count]
    }

    private static func prompt(_ turn: Int) -> String {
      switch turn % 4 {
      case 0: "Next: \(file(turn))."
      case 1: "Why does `\(file(turn))` take so long to lay out?"
      case 2:
        "Take a look at how the \(file(turn)) module decides what to draw when a Turn is still running, and whether "
          + "it redraws everything above it each time a token lands. I think it does, and on a long Session that adds "
          + "up. If it does, keep what it drew before and only draw the tail again."
      default: "Ok"
      }
    }

    private static func answer(_ turn: Int) -> String {
      let name = file(turn)
      switch turn % 5 {
      case 0:
        return "Done: `\(name)` no longer redraws what stands above the running Turn."
      case 1:
        return """
          `\(name)` lays out every row on each change, because the list is rebuilt from scratch.

          - The projection is recomputed for **every** item, not just the changed ones.
          - Heights are measured twice: once to size, once to draw.
          - Nothing is cached between frames.

          Keeping the projection by id brings it down to the rows that changed.
          """
      case 2:
        return """
          ### What changed in `\(name)`

          The rows are now keyed by id, and a row's projection is kept until the item it projects changes. The \
          running Turn's card is the only row redrawn while tokens land.

          ```rust
          \(code(turn, lines: 18 + turn % 13))
          ```

          | Items | Before | After |
          |------:|-------:|------:|
          | 100 | 14ms | 3ms |
          | 300 | 41ms | 4ms |
          | 1000 | 140ms | 6ms |

          The table above is the median frame time while a Turn streams.
          """
      case 3:
        return """
          | Module | Lines | Rows drawn | Notes |
          |--------|------:|-----------:|-------|
          | projection | 412 | 1 | keyed by id |
          | markdown | 1210 | 1 | parsed once per change |
          | cards | 233 | 2 | the running one and the last |
          | scroll | 98 | 0 | anchored at the foot |
          | heights | 140 | 0 | measured once |

          Most of the time went to `markdown`, which parsed every Message again on each token. It now parses only the \
          one that changed.
          """
      default:
        let paragraphs = (0..<(4 + turn % 4)).map { index in
          "Paragraph \(index + 1) of the long answer about `\(name)`: the layout pass walks every row it holds, and "
            + "each row asks its children for their sizes, so a Message with a table or a code block in it costs "
            + "several times what a plain one does. Over a Session of hundreds of items that is most of a frame, and "
            + "it is spent again whenever anything above changes, which while a Turn streams is many times a second."
        }
        return paragraphs.joined(separator: "\n\n") + "\n\n```swift\n" + code(turn, lines: 8) + "\n```"
      }
    }

    private static func code(_ seed: Int, lines: Int) -> String {
      (0..<lines).map { line in
        switch (seed + line) % 4 {
        case 0: "let row_\(line) = projection.row(id_\(line)).cloned().unwrap_or_default();"
        case 1: "if row_\(line).changed_since(frame) { redraw.push(row_\(line).id()); }"
        case 2: "    // Keep what was drawn: only row \(line) of \(lines) moved since the last frame."
        default: "frame.heights.insert(row_\(line).id(), measure(&row_\(line), width));"
        }
      }.joined(separator: "\n")
    }
  }
#endif
