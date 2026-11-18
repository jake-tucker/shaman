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
            kind: .compaction(words: "Compacted context · 180K → 40K (automatic)", tone: .succeeded)),
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

    private static func user(_ id: String, _ text: String) -> ConversationItem {
      ConversationItem(id: id, kind: .userMessage(text: text, note: nil))
    }

    private static func agent(_ id: String, _ markdown: String, model: String? = nil, streaming: Bool = false)
      -> ConversationItem
    {
      ConversationItem(id: id, kind: .agentMessage(markdown: markdown, model: model, streaming: streaming))
    }

    private static func card(_ id: String, _ outcome: TurnOutcome, _ line: String, _ count: String?)
      -> ConversationItem
    {
      ConversationItem(id: id, kind: .turn(card: TurnCard(outcome: outcome, line: line, count: count)))
    }
  }

  /// A Session the fake holds open: framed at once, its conversation landing a moment later, and a working Turn's
  /// answer streaming in by change.
  final class FakeOpenedSession: OpenedSession {
    private let initialFrame: SessionFrame
    private let scenario: FakeConversation.Scenario?
    private let closed = Atomic<Bool>(false)

    init(frame: SessionFrame, scenario: FakeConversation.Scenario?) {
      initialFrame = frame
      self.scenario = scenario
    }

    var isClosed: Bool { closed.load(ordering: .relaxed) }

    func frame() -> AsyncStream<SessionFrame> {
      AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
        continuation.yield(initialFrame)
      }
    }

    func conversation() -> AsyncStream<ConversationChange> {
      let scenario = scenario
      return AsyncStream(bufferingPolicy: .unbounded) { continuation in
        continuation.yield(
          .whole(items: [ConversationItem(id: "loading", kind: .loading(words: "Loading the conversation"))]))
        guard let scenario else { return }
        let task = Task {
          try? await Task.sleep(for: .milliseconds(600))
          let items = FakeConversation.items(scenario)
          continuation.yield(.changed(items: items, order: items.map(\.id)))
          guard scenario == .working else { return }
          for text in FakeConversation.streamed.dropFirst() {
            try? await Task.sleep(for: .milliseconds(700))
            guard !Task.isCancelled else { return }
            continuation.yield(
              .changed(
                items: [
                  ConversationItem(
                    id: "answer", kind: .agentMessage(markdown: text, model: nil, streaming: true))
                ], order: items.map(\.id)))
          }
        }
        continuation.onTermination = { _ in task.cancel() }
      }
    }

    func close() {
      closed.store(true, ordering: .relaxed)
    }
  }
#endif
