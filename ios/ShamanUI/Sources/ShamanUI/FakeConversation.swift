#if DEBUG
  import Foundation
  import ShamanCore
  import Synchronization

  /// What an open Session reads as over the fake, from the transcript prototype's canned Session: one conversation for
  /// each way its last Turn can stand, as the core would make it. A Session is first framed by what the listing
  /// knows, the loading mark standing a moment before the conversation lands in one piece.
  enum FakeConversation {
    /// How a fake Session's last Turn stands.
    enum Scenario: String, CaseIterable, Sendable {
      /// The Turn works: its card names the command running now, and its answer streams in beside it.
      case working
      /// The Turn settled having worked, its answer outside its card.
      case settled
      /// The Turn was stopped, and steered before it was.
      case stopped
      /// The Turn failed, its Error outside its card.
      case failed
      /// The Session is Working only for a Prompt not yet delivered, which stands at the foot as the Message it will
      /// be.
      case undelivered
      /// The Turn settled, and the Agent's Watch is still running.
      case monitoring
      /// The Turn waits on an Approval of a command.
      case approval
      /// The Turn waits on two Approvals, a File Change asked before another Tool.
      case approvals
    }

    /// How the conversation stands of the Session whose last Turn stands as `scenario`.
    static func items(_ scenario: Scenario) -> [ConversationItem] {
      let opening = earlier + [user("open", "Draw the invite as a QR code in /serve")]
      switch scenario {
      case .working:
        return opening + [
          card("work", .working, "cargo nextest run qr", "1 tool · 1 command · 1 edit"),
          agent("answer", streamed.first ?? "", streaming: true),
        ]
      case .settled:
        return opening + [
          card("work", .worked, "Worked for 3m 12s", "3 tools · 3 commands · 2 edits"),
          ConversationItem(
            id: "compaction",
            kind: .compaction(
              words: "Compacted context · 180K → 40K (automatic)", tone: .succeeded, detail: "compaction")),
          agent("answer", answer, model: "auto · gpt-5.5"),
        ]
      case .stopped:
        return opening + [
          card("work", .stopped, "Stopped after 41s", "1 tool · 1 command · 1 edit"),
          user("steer", "Sorry, carry on. But give it a quiet zone: my terminal has a light background."),
        ]
      case .failed:
        return opening + [
          card("work", .failed, "Failed after 12s", "1 command"),
          ConversationItem(
            id: "error", kind: .error(text: "The provider stopped answering: connection reset by peer")),
        ]
      case .undelivered:
        return [user("open", undeliveredPrompt)]
      case .monitoring:
        return opening + [
          card("work", .worked, "Worked for 2m 05s", "2 commands"),
          agent("answer", "Watching the nightly backup; I'll say when it lands."),
        ]
      case .approval, .approvals:
        return [
          user("open", "Implement ADR-0078: each side states one version and accepts the other at or above its floor.")
        ] + [card("work", .waiting, "Waiting on you", "3 tools · 1 command · 2 edits")]
      }
    }

    /// The Approvals the Session whose last Turn stands as `scenario` waits on, oldest first, as the core presents
    /// them.
    static func approvals(_ scenario: Scenario) -> [PendingApproval] {
      switch scenario {
      case .approval:
        return [
          approval(
            "approval-command", "Run this command?",
            facts: [
              mono("Command", "cargo nextest run -p shaman-relay"),
              mono("Directory", "~/Projects/shaman/.worktrees/peer-version-floor"),
              Fact(label: "Reason", value: "Needs network access outside the sandbox.", style: .prose),
            ])
        ]
      case .approvals:
        return [
          approval(
            "approval-file-change", "Make this file change?",
            facts: [
              mono("File change", "tokens/buttons.json"),
              mono("Path", "tokens/focus-ring.json"),
              Fact(label: "Reason", value: "Regenerate the palette from the new tokens.", style: .prose),
            ]),
          approval(
            "approval-tool", "Use this tool?",
            facts: [mono("Other tool", "figma/export_variables")],
            sections: [
              DetailSection(
                heading: "Input", body: "{\n  \"collection\": \"Buttons\",\n  \"mode\": \"dark\"\n}",
                style: .monospace)
            ]),
        ]
      case .working, .settled, .stopped, .failed, .undelivered, .monitoring:
        return []
      }
    }

    private static func approval(
      _ reference: String, _ title: String, facts: [Fact], sections: [DetailSection] = []
    ) -> PendingApproval {
      PendingApproval(
        reference: reference, title: title, facts: facts, sections: sections, note: nil,
        large: [
          DecisionOffer(decision: .decline, label: "Decline"),
          DecisionOffer(decision: .acceptOnce, label: "Accept once"),
        ],
        beneath: [
          DecisionOffer(decision: .declineAndInterrupt, label: "Decline and interrupt"),
          DecisionOffer(decision: .acceptForSession, label: "Accept for session"),
        ])
    }

    private static func mono(_ label: String, _ value: String) -> Fact {
      Fact(label: label, value: value, style: .monospace)
    }

    /// What the Session Working only for a Prompt not yet delivered was asked.
    static let undeliveredPrompt = "Note the floor in the pairing docs"

    /// The streamed answer of the working Turn, as it grows.
    static let streamed = [
      "The QR code is drawn from half blocks,",
      "The QR code is drawn from half blocks, so it stands 33 rows tall.",
      "The QR code is drawn from half blocks, so it stands 33 rows tall. A panel narrower than 41 columns says how "
        + "wide it needs to be instead.",
    ]

    /// A Turn before the last, settled, so the conversation reads as one with a history.
    private static let earlier = [
      user("earlier", "What does /serve show today?"),
      card("earlier-work", .worked, "Worked for 1m 23s", "2 tools · 1 command"),
      agent("earlier-answer", "`/serve` shows the **invite** as text, with a countdown beside it."),
    ]

    private static let answer = """
      `/serve` now offers **Draw as QR code** beneath a fresh invite.

      - The code is drawn from half blocks, so it stands 33 rows tall.
      - A panel narrower than 41 columns says how wide it needs to be instead.

      | Invite | Modules | Rows |
      |--------|---------|------|
      | Direct ways only | 33 × 33 | 17 |
      | With a relay way | 41 × 41 | 21 |

      Not done yet: the CLI. It would be:

      ```
      shaman serve invite --qr --error-correction low --margin 2
      ```
      """

    static func user(_ id: String, _ text: String, sending: Bool = false) -> ConversationItem {
      ConversationItem(id: id, kind: .userMessage(text: text, note: nil, sending: sending))
    }

    private static func agent(_ id: String, _ markdown: String, model: String? = nil, streaming: Bool = false)
      -> ConversationItem
    {
      ConversationItem(id: id, kind: .agentMessage(markdown: markdown, model: model, streaming: streaming))
    }

    static func card(_ id: String, _ outcome: TurnOutcome, _ line: String, _ count: String?)
      -> ConversationItem
    {
      // The card hands back its id as its Turn, whose work `FakeWork` reads.
      ConversationItem(id: id, kind: .turn(card: TurnCard(work: id, outcome: outcome, line: line, count: count)))
    }
  }
#endif
