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
      /// The Agent delegated to Subagents and only waits on them: the Session heads the tree `FakeTree` holds.
      case delegating
      /// The Turn waits on a Questionnaire of three Questions: one choice with a note beside it, required; several
      /// choices or a written answer instead, which may be skipped; and a secret, which may be skipped.
      case questionnaire
      /// A long Session of hundreds of items of very uneven height, `FakeLong`'s, settled.
      case long
      /// The `long` Session, its last answer still streaming in.
      case longWorking
      /// A short Session whose answer is written again and again, long under the Model's name, then short without it.
      case rewriting
      /// The `long` Session ending in eight one-line answers.
      case longTailed
    }

    /// How the conversation stands of the Session whose last Turn stands as `scenario`.
    static func items(_ scenario: Scenario) -> [ConversationItem] {
      let opening = earlier + [user("open", "Draw the invite as a QR code in /serve")]
      switch scenario {
      case .working:
        return opening + [card("work", .working, "cargo nextest run qr", "1 tool · 1 command · 1 edit")]
          + agent("answer", streamed.first ?? "")
      case .settled:
        return opening + [
          card("work", .worked, "Worked for 3m 12s", "3 tools · 3 commands · 2 edits"),
          ConversationItem(
            id: "compaction",
            kind: .compaction(
              words: "Compacted context · 180K → 40K (automatic)", tone: .succeeded, detail: "compaction")),
        ] + agent("answer", answer, model: "auto · gpt-5.5")
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
      case .delegating:
        // Read from the tree it heads, as a stop there moves it.
        return []
      case .monitoring:
        return opening + [card("work", .worked, "Worked for 2m 05s", "2 commands")]
          + agent("answer", "Watching the nightly backup; I'll say when it lands.")
      case .approval, .approvals:
        return [
          user("open", "Implement ADR-0078: each side states one version and accepts the other at or above its floor.")
        ] + [card("work", .waiting, "Waiting on you", "3 tools · 1 command · 2 edits")]
      case .questionnaire:
        return [user("open", "Run the relay's tests before the release, wherever suits")]
          + [card("work", .waiting, "Waiting on you", "2 tools · 1 command")]
      case .long:
        return FakeLong.items(streaming: false)
      case .longWorking:
        return FakeLong.items(streaming: true)
      case .longTailed:
        return FakeLong.items(streaming: false, tail: true)
      case .rewriting:
        return [
          user("open", "Write the release notes"),
          card("work", .worked, "Worked for 40s", "1 tool"),
        ] + agent("answer", FakeLong.rewritten(long: true), model: "auto · gpt-5.5")
      }
    }

    /// The Interventions the Session whose last Turn stands as `scenario` waits on, oldest first, as the core
    /// presents them.
    static func interventions(_ scenario: Scenario) -> [Intervention] {
      if scenario == .questionnaire {
        return [.questionnaire(questionnaire: questionnaire)]
      }
      return approvals(scenario).map { .approval(approval: $0) }
    }

    /// The Questionnaire of the `questionnaire` scenario, as the core presents it.
    static let questionnaire = PendingQuestionnaire(
      reference: "questionnaire-tests",
      questions: [
        PendingQuestion(
          position: "Question 1 of 3", heading: "Machine", text: "Where should the relay's tests run?", hint: nil,
          choices: [
            choice("staging", "Staging", "The shared box, with the relay's real certificates.", recommended: true),
            choice("local", "Local", "This laptop; nothing reaches the network."),
            choice("ci", "CI", "A fresh runner: slower, but clean."),
          ], multiple: false, field: AnswerField(label: "Additional text", combines: true, secure: false), skip: nil,
          secret: nil),
        PendingQuestion(
          position: "Question 2 of 3", heading: "Suites", text: "Which suites should run?", hint: "Choose any",
          choices: [
            choice("unit", "Unit", "Fast, and nothing reaches the network."),
            choice("integration", "Integration", "Against an in-process relay."),
            choice("soak", "Soak", "An hour of reconnects."),
          ], multiple: true, field: AnswerField(label: "Or write your own", combines: false, secure: false),
          skip: "Skip", secret: nil),
        PendingQuestion(
          position: "Question 3 of 3", heading: "Token", text: "The staging deploy token, if the tests need one?",
          hint: nil, choices: [], multiple: false,
          field: AnswerField(label: "Answer", combines: false, secure: true), skip: "Skip",
          secret: "Answered (secret)"),
      ], note: nil, decline: "Decline", next: "Next", send: "Send answers", back: "Back")

    private static func choice(
      _ choice: String, _ label: String, _ description: String, recommended: Bool = false
    ) -> ChoiceCard {
      ChoiceCard(
        choice: choice, label: label, description: description, badge: recommended ? "Recommended" : nil)
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
      case .working, .settled, .stopped, .failed, .undelivered, .monitoring, .delegating, .questionnaire, .long,
        .longWorking, .rewriting, .longTailed:
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

    /// What the answer, the Message `answer`, of the Session whose last Turn stands as `scenario` grows to after it first
    /// stands, a moment apart each time.
    static func growing(_ scenario: Scenario?) -> [String] {
      switch scenario {
      case .working: Array(streamed.dropFirst())
      case .longWorking: (1...FakeLong.growths).map(FakeLong.streamed)
      // Short six times, then long six times, and so on: each stands 4.2 seconds.
      case .rewriting: (0..<FakeLong.growths).map { FakeLong.rewritten(long: $0 % 12 >= 6) }
      default: []
      }
    }

    /// The streamed answer of the working Turn, as it grows.
    static let streamed = [
      "The QR code is drawn from half blocks,",
      "The QR code is drawn from half blocks, so it stands 33 rows tall.",
      "The QR code is drawn from half blocks, so it stands 33 rows tall. A panel narrower than 41 columns says how "
        + "wide it needs to be instead.",
    ]

    /// A Turn before the last, settled, so the conversation reads as one with a history.
    private static let earlier =
      [
        user("earlier", "What does /serve show today?"),
        card("earlier-work", .worked, "Worked for 1m 23s", "2 tools · 1 command"),
      ] + agent("earlier-answer", "`/serve` shows the **invite** as text, with a countdown beside it.")

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

    /// The items the agent Message `id` is, cut as the core cuts one it reads.
    static func agent(_ id: String, _ markdown: String, model: String? = nil) -> [ConversationItem] {
      agentMessageItems(message: id, markdown: markdown, model: model)
    }

    static func card(_ id: String, _ outcome: TurnOutcome, _ line: String, _ count: String?)
      -> ConversationItem
    {
      // The card hands back its id as its Turn, whose work `FakeWork` reads.
      ConversationItem(id: id, kind: .turn(card: TurnCard(work: id, outcome: outcome, line: line, count: count)))
    }
  }
#endif
