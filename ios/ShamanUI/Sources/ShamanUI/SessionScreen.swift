import ShamanCore
import SwiftUI
import Textual

/// A Session pushed over the listing, read live as a conversation: the Title over where it stands, the user's
/// Messages as bubbles at the right, the Agent's as Markdown across the width, and each Turn's work behind one card.
/// It appears at once with what the listing knows, a quiet loading mark standing where the conversation will be.
struct SessionScreen: View {
  let model: SessionModel

  var body: some View {
    ScrollView {
      LazyVStack(alignment: .leading, spacing: 12) {
        ForEach(model.conversation.shown, id: \.id) { item in
          ItemView(item: item)
        }
      }
      .padding(.horizontal, 14)
      .padding(.vertical, 8)
    }
    .defaultScrollAnchor(.bottom)
    .defaultScrollAnchor(.bottom, for: .sizeChanges)
    .accessibilityIdentifier("session.conversation")
    .overlay {
      if let gone = model.frame?.gone {
        ContentUnavailableView {
          Label {
            Text(verbatim: gone)
              .accessibilityIdentifier("session.gone")
          } icon: {
            Image(systemName: "questionmark.bubble")
          }
        }
        .background(Color(.systemBackground))
      }
    }
    .navigationBarTitleDisplayMode(.inline)
    .toolbar {
      ToolbarItem(placement: .principal) {
        SessionHeader(frame: model.frame)
      }
    }
    .task { await model.follow() }
  }
}

/// The Title over "origin · workspace · branch".
private struct SessionHeader: View {
  let frame: SessionFrame?

  var body: some View {
    VStack(spacing: 1) {
      Text(verbatim: frame?.title ?? "")
        .font(.headline)
        .lineLimit(1)
        .accessibilityIdentifier("session.title")
      if let place = frame?.place, !place.isEmpty {
        Text(verbatim: place)
          .font(.caption)
          .foregroundStyle(.secondary)
          .lineLimit(1)
          .truncationMode(.middle)
          .accessibilityIdentifier("session.place")
      }
    }
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("session.header")
  }
}

/// One item of the conversation, drawn as what it is.
private struct ItemView: View {
  let item: ConversationItem

  var body: some View {
    switch item.kind {
    case .loading(let words):
      ProgressView()
        .controlSize(.regular)
        .frame(maxWidth: .infinity)
        .padding(.top, 40)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: words))
        .accessibilityIdentifier("session.loading")
    case .userMessage(let text, let note):
      UserBubble(text: text, note: note)
    case .delegation(let text, let from):
      DelegationBubble(text: text, from: from)
    case .agentMessage(let markdown, let model, _):
      AgentMessage(markdown: markdown, model: model)
    case .turn(let card):
      TurnCardView(card: card)
    case .error(let text):
      Label {
        Text(verbatim: text)
      } icon: {
        Image(systemName: "exclamationmark.triangle.fill")
      }
      .font(.footnote)
      .foregroundStyle(.red)
      .padding(10)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(Color.red.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
      .accessibilityElement(children: .combine)
      .accessibilityIdentifier("session.error")
    case .compaction(let words, let tone):
      Label {
        Text(verbatim: words)
      } icon: {
        tone.symbol
      }
      .font(.footnote)
      .foregroundStyle(.secondary)
      .accessibilityElement(children: .combine)
      .accessibilityIdentifier("session.compaction")
    }
  }
}

/// What the user said, at the right.
private struct UserBubble: View {
  let text: String
  let note: String?

  var body: some View {
    VStack(alignment: .trailing, spacing: 3) {
      Text(verbatim: text)
        .padding(.horizontal, 13)
        .padding(.vertical, 9)
        .foregroundStyle(.white)
        .background(Color.accentColor, in: RoundedRectangle(cornerRadius: 18))
      if let note {
        Label {
          Text(verbatim: note)
        } icon: {
          Image(systemName: "paperclip")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
      }
    }
    .frame(maxWidth: .infinity, alignment: .trailing)
    .padding(.leading, 44)
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier("session.userMessage")
  }
}

/// What a delegating Agent asked of a Subagent, at the left, naming who sent it.
private struct DelegationBubble: View {
  let text: String
  let from: String

  var body: some View {
    VStack(alignment: .leading, spacing: 3) {
      Text(verbatim: from)
        .font(.caption)
        .foregroundStyle(.secondary)
      Text(verbatim: text)
        .padding(.horizontal, 13)
        .padding(.vertical, 9)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18))
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(.trailing, 44)
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier("session.delegation")
  }
}

/// What the Agent said, as Markdown across the width: code blocks and tables scroll sideways rather than wrap. Where
/// another Model answered than the one selected, a subdued line above names both.
private struct AgentMessage: View {
  let markdown: String
  let model: String?

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      if let model {
        Text(verbatim: model)
          .font(.caption)
          .foregroundStyle(.tertiary)
          .accessibilityIdentifier("session.model")
      }
      StructuredText(markdown: markdown)
        .textual.overflowMode(.scroll)
        .textual.tableStyle(.overflow)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier("session.agentMessage")
  }
}

/// The card a Turn's work stands behind: how it settled and how long it took, or what runs now, over its count.
private struct TurnCardView: View {
  let card: TurnCard

  var body: some View {
    HStack(spacing: 10) {
      Group {
        switch card.outcome {
        case .working:
          ProgressView().controlSize(.small)
        case .waiting:
          Image(systemName: "hand.raised.fill").foregroundStyle(.orange)
        case .worked:
          Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
        case .stopped:
          Image(systemName: "stop.circle.fill").foregroundStyle(.yellow)
        case .failed:
          Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
        }
      }
      .frame(width: 22)
      .accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 2) {
        Text(verbatim: card.line)
          .font(.subheadline.weight(.medium))
          .lineLimit(1)
        if let count = card.count {
          Text(verbatim: count)
            .font(.caption)
            .foregroundStyle(.secondary)
        }
      }
      Spacer(minLength: 0)
    }
    .padding(.horizontal, 12)
    .padding(.vertical, 10)
    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
    .padding(.trailing, 24)
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier("session.turn")
  }
}

extension Tone {
  /// The symbol a piece of work standing as this is led by.
  @ViewBuilder var symbol: some View {
    switch self {
    case .running: ProgressView().controlSize(.mini)
    case .succeeded: Image(systemName: "rectangle.compress.vertical")
    case .failed: Image(systemName: "exclamationmark.triangle").foregroundStyle(.red)
    case .stopped: Image(systemName: "stop.circle").foregroundStyle(.yellow)
    }
  }
}

/// An open Session's conversation as Swift holds it: each item by its id, in the order the core last said. Swift's
/// part is no more than replacing an item by its id; the core folds everything else.
struct Conversation: Equatable {
  private var items: [String: ConversationItem] = [:]
  private var order: [String] = []

  /// The items in the order they stand.
  var shown: [ConversationItem] {
    order.compactMap { items[$0] }
  }

  /// Takes `change` in: the conversation whole, or the items that changed and the order every item stands in now.
  mutating func take(_ change: ConversationChange) {
    switch change {
    case .whole(let whole):
      items = Dictionary(whole.map { ($0.id, $0) }, uniquingKeysWith: { _, last in last })
      order = whole.map(\.id)
    case .changed(let changed, let newOrder):
      for item in changed {
        items[item.id] = item
      }
      order = newOrder
      let standing = Set(newOrder)
      items = items.filter { standing.contains($0.key) }
    }
  }
}

/// What a pushed Session holds while it is open.
@MainActor @Observable
final class SessionModel {
  /// The reference the row it was opened from handed over.
  let session: String
  private let opened: any OpenedSession
  private(set) var frame: SessionFrame?
  private(set) var conversation = Conversation()

  init(core: any Core, session: String) {
    self.session = session
    opened = core.openSession(session)
  }

  /// Follows the frame and the conversation until the screen goes.
  func follow() async {
    async let frames: Void = followFrame()
    async let changes: Void = followConversation()
    _ = await (frames, changes)
  }

  private func followFrame() async {
    for await frame in opened.frame() {
      self.frame = frame
    }
  }

  private func followConversation() async {
    for await change in opened.conversation() {
      conversation.take(change)
    }
  }

  /// Ends the Session in the core, as its screen is popped.
  func close() {
    opened.close()
  }
}

/// The Sessions open on the screens pushed over the listing, each closed in the core as its screen is popped.
@MainActor
final class OpenedSessions {
  private var models: [SessionRoute: SessionModel] = [:]

  /// The model of the screen `route` pushed, opened the first time it is asked for.
  func model(for route: SessionRoute, core: any Core) -> SessionModel {
    if let model = models[route] {
      return model
    }
    let model = SessionModel(core: core, session: route.session)
    models[route] = model
    return model
  }

  /// Closes every Session whose screen is no longer on `path`.
  func keep(only path: [SessionRoute]) {
    let pushed = Set(path)
    for (route, model) in models where !pushed.contains(route) {
      model.close()
      models[route] = nil
    }
  }
}

/// A Session's screen pushed over the listing: each push its own, so the same Session pushed twice is two screens.
struct SessionRoute: Hashable {
  let session: String
  let push = UUID()
}

#if DEBUG
  #Preview("A working session") {
    NavigationStack {
      SessionScreen(model: SessionModel(core: FakeCore(.paired), session: "workstation-key/1"))
    }
  }

  #Preview("A settled session") {
    NavigationStack {
      SessionScreen(model: SessionModel(core: FakeCore(.paired), session: "workstation-key/7"))
    }
  }
#endif
