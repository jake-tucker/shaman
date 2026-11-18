#if DEBUG
  import Foundation
  import ShamanCore

  /// What a fake Session's Turns did, from the transcript prototype's canned Session, as the core would fill it: each
  /// card's work and each row's detail. The working Turn's command is still running, and its output grows while its
  /// screen is up.
  enum FakeWork {
    /// The work of the Turn a card of a Session reading as `scenario` handed back, at once and as it changes.
    static func work(of turn: String, in scenario: FakeConversation.Scenario?) -> AsyncStream<TurnWork> {
      let work = scenario.map { work(of: turn, $0) } ?? TurnWork(title: "", items: [], empty: "Loading")
      return AsyncStream { continuation in
        continuation.yield(work)
      }
    }

    /// The whole of the Activity a row handed back, at once and as it changes: the running command's output grows a
    /// line at a time.
    static func detail(of activity: String, in scenario: FakeConversation.Scenario?) -> AsyncStream<ActivityDetail> {
      AsyncStream { continuation in
        guard activity == running else {
          continuation.yield(details[activity] ?? gone)
          return
        }
        let growing = Task {
          for lines in 1...testLines.count {
            continuation.yield(runningDetail(lines: lines))
            try? await Task.sleep(for: .milliseconds(700))
            guard !Task.isCancelled else { return }
          }
        }
        continuation.onTermination = { _ in growing.cancel() }
      }
    }

    /// The Activity of the working Turn that is still running.
    static let running = "run-qr"

    private static let gone = ActivityDetail(
      title: "", facts: [],
      sections: [DetailSection(heading: nil, body: "This is no longer in the session", style: .prose)])

    private static func work(of turn: String, _ scenario: FakeConversation.Scenario) -> TurnWork {
      switch (turn, scenario) {
      case ("earlier-work", _):
        return TurnWork(
          title: "Worked for 1m 23s",
          items: [
            row("read-serve", .toolCall, "Read src/tui/serve_overlay.rs", tone: .succeeded),
            row("grep-invite", .toolCall, "Grep \"invite\" src/tui", tone: .succeeded),
            row(
              "question-qr", .questionnaire, "Questionnaire · Answered", line: "Which error correction level?",
              trailing: "1 question", tone: .succeeded),
            row("run-serve", .command, "cargo run -- serve --print-invite", tone: .succeeded),
          ], empty: nil)
      case ("work", .working):
        return TurnWork(
          title: "Working",
          items: [
            row("read-panel", .toolCall, "Read src/tui/serve_panel.rs", tone: .succeeded),
            agent("note", "The invite fits a version 12 code at low error correction."),
            row("edit-qr", .fileChange, "Edited src/tui/qr.rs", tone: .succeeded),
            row(running, .command, "cargo nextest run qr", tone: .running),
          ], empty: nil)
      case ("work", .settled):
        return TurnWork(
          title: "Worked for 3m 12s",
          items: [
            row("read-panel", .toolCall, "Read src/tui/serve_panel.rs", tone: .succeeded),
            row("grep-prefix", .toolCall, "Grep \"shaman-v1-\"", tone: .succeeded),
            row("read-serving", .toolCall, "Read src/serving.rs", tone: .succeeded),
            row("add-qrcode", .command, "cargo add qrcode --no-default-features", tone: .succeeded),
            agent(
              "note",
              "The invite fits a version 12 code at low error correction. I'll draw it with half blocks so it stands "
                + "in 33 rows rather than 65."),
            row(
              "edit-panel", .fileChange, "Edited src/tui/serve_panel.rs", line: "and 1 more file", tone: .succeeded),
            row(
              "approve-tests", .approval, "Approval · Command · Accepted once", line: "cargo nextest run serve_panel",
              tone: .succeeded),
            row("check", .command, "cargo check", tone: .succeeded),
            row("search", .unknown, "Not shown: web search, from a newer Shaman", tone: .unknown, tap: .nothing),
            row("test-panel", .command, "cargo nextest run serve_panel", tone: .succeeded),
          ], empty: nil)
      case ("work", .stopped):
        return TurnWork(
          title: "Stopped after 41s",
          items: [
            row("read-panel", .toolCall, "Read src/tui/serve_panel.rs", tone: .succeeded),
            row("stop-edit-qr", .fileChange, "Stopped editing src/tui/qr.rs", tone: .stopped),
            row("test-qr", .command, "cargo nextest run qr", tone: .stopped),
          ], empty: nil)
      case ("work", .failed):
        return TurnWork(
          title: "Failed after 12s",
          items: [
            row("fail-qr", .command, "cargo test qr", line: "in crates/shaman-qr", trailing: "exit 101", tone: .failed)
          ], empty: nil)
      case ("work", .monitoring):
        return TurnWork(
          title: "Worked for 2m 05s",
          items: [
            row("start-backup", .command, "systemctl start backup.service", tone: .succeeded),
            row("watch-backup", .command, "journalctl -fu backup.service", tone: .succeeded),
          ], empty: nil)
      default:
        // A Turn just begun has done nothing yet.
        return TurnWork(title: "Working", items: [], empty: "Nothing done yet")
      }
    }

    /// The glyphs the core leads each kind's row with.
    private enum Kind: String {
      case command = "\u{ea85}"
      case toolCall = "\u{eb6d}"
      case fileChange = "\u{ea73}"
      case approval = "\u{eb53}"
      case questionnaire = "\u{eb32}"
      case unknown = "\u{eabd}"
    }

    private static func row(
      _ id: String, _ kind: Kind, _ title: String, line: String? = nil, trailing: String? = nil, tone: Tone,
      tap: Tap? = nil
    ) -> WorkItem {
      WorkItem(
        id: "activity:\(id)",
        kind: .row(
          row: WorkRow(
            glyph: kind.rawValue, title: title,
            titleStyle: [.command, .toolCall, .fileChange].contains(kind) ? .monospace : .prose, line: line,
            trailing: trailing, tone: tone,
            tap: tap ?? .detail(activity: id))))
    }

    private static func agent(_ id: String, _ markdown: String) -> WorkItem {
      WorkItem(id: "message:\(id)", kind: .agentMessage(markdown: markdown, streaming: false))
    }

    private static func prose(_ heading: String? = nil, _ body: String) -> DetailSection {
      DetailSection(heading: heading, body: body, style: .prose)
    }

    private static func output(_ body: String) -> DetailSection {
      DetailSection(heading: "Output", body: body, style: .monospace)
    }

    private static func fact(_ label: String, _ value: String, mono: Bool = false) -> Fact {
      Fact(label: label, value: value, style: mono ? .monospace : .prose)
    }

    private static func command(_ line: String, status: String, output body: String) -> ActivityDetail {
      ActivityDetail(
        title: "Command", facts: [fact("Command", line, mono: true), fact("Status", status)],
        sections: [output(body)])
    }

    private static func tool(_ name: String, _ input: String, output body: String) -> ActivityDetail {
      ActivityDetail(
        title: name,
        facts: [fact("Tool", name, mono: true), fact("Input", input, mono: true), fact("Status", "Completed")],
        sections: [output(body)])
    }

    /// The lines the running command writes, one at a time; the last is wider than the screen.
    private static let testLines = [
      "    Finished `test` profile [unoptimized + debuginfo] target(s) in 4.21s",
      "────────────",
      " Nextest run ID 6b1c2f9e with nextest profile: default",
      "    Starting 4 tests across 1 binary (212 tests skipped)",
      "        PASS [   0.012s] shaman tui::qr::tests::an_invite_is_drawn_from_half_blocks",
      "        PASS [   0.014s] shaman tui::qr::tests::a_narrow_panel_says_how_wide_it_needs_to_be",
      "        PASS [   0.019s] shaman tui::qr::tests::the_quiet_zone_stands_on_every_side_of_the_code_whatever_the_theme",
    ]

    private static func runningDetail(lines: Int) -> ActivityDetail {
      ActivityDetail(
        title: "Command", facts: [fact("Command", "cargo nextest run qr", mono: true), fact("Status", "Running")],
        sections: [output(testLines.prefix(lines).joined(separator: "\n"))])
    }

    private static let details: [String: ActivityDetail] = [
      "read-serve": tool("Read", "src/tui/serve_overlay.rs", output: "312 lines"),
      "grep-invite": tool("Grep", "\"invite\" src/tui", output: "src/tui/serve_overlay.rs:41: invite"),
      "question-qr": ActivityDetail(
        title: "Questionnaire", facts: [fact("Outcome", "Answered")],
        sections: [prose("Which error correction level?", "Low, so it stays small")]),
      "run-serve": command(
        "cargo run -- serve --print-invite", status: "Completed (exit 0)", output: "shaman-v1-AAAQ…"),
      "read-panel": tool("Read", "src/tui/serve_panel.rs", output: "312 lines"),
      "grep-prefix": tool("Grep", "\"shaman-v1-\"", output: "6 matches in 4 files"),
      "read-serving": tool("Read", "src/serving.rs", output: "2140 lines"),
      "add-qrcode": command(
        "cargo add qrcode --no-default-features", status: "Completed (exit 0)",
        output: "    Updating crates.io index\n      Adding qrcode v0.14.1 to dependencies"),
      "edit-qr": ActivityDetail(
        title: "File change", facts: [fact("Edited", "src/tui/qr.rs", mono: true)], sections: []),
      "edit-panel": ActivityDetail(
        title: "File change",
        facts: [fact("Edited", "src/tui/serve_panel.rs", mono: true), fact("Created", "src/tui/qr.rs", mono: true)],
        sections: []),
      "approve-tests": ActivityDetail(
        title: "Approval",
        facts: [
          fact("Command", "cargo nextest run serve_panel", mono: true),
          fact("Directory", "/Users/jake/Projects/shaman", mono: true),
          fact("Reason", "Runs the tests, which build the workspace"),
          fact("Decision", "Accepted once"),
        ], sections: []),
      "check": command(
        "cargo check", status: "Completed (exit 0)",
        output: "    Checking shaman v0.1.0 (/Users/jake/Projects/shaman)\n    Finished `dev` profile in 9.87s"),
      "test-panel": command(
        "cargo nextest run serve_panel", status: "Completed (exit 0)",
        output: "     Summary [   0.301s] 12 tests run: 12 passed, 204 skipped"),
      "test-qr": command(
        "cargo nextest run qr", status: "Stopped", output: testLines.prefix(3).joined(separator: "\n")),
      "stop-edit-qr": ActivityDetail(
        title: "File change", facts: [fact("Stopped editing", "src/tui/qr.rs", mono: true)], sections: []),
      "fail-qr": ActivityDetail(
        title: "Command",
        facts: [
          fact("Command", "cargo test qr", mono: true), fact("Directory", "crates/shaman-qr", mono: true),
          fact("Status", "Failed (exit 101)"),
        ],
        sections: [
          output("error[E0425]: cannot find value `quiet_zone` in this scope\n  --> src/lib.rs:41:9")
        ]),
      "start-backup": command("systemctl start backup.service", status: "Completed (exit 0)", output: ""),
      "watch-backup": command(
        "journalctl -fu backup.service", status: "Completed (exit 0)", output: "Started nightly backup."),
      "compaction": ActivityDetail(
        title: "Compaction",
        facts: [
          fact("Status", "Compacted context"), fact("Context", "180K → 40K"), fact("Trigger", "Automatic"),
        ],
        sections: [prose("Summary", "We drew the invite as a QR code in /serve and tested it.")]),
    ]
  }
#endif
