import ShamanCore
import SwiftUI
import Textual

/// A Session pushed over the listing, read live as a conversation: the Title over where it stands, the user's
/// Messages as bubbles at the right, the Agent's as Markdown across the width, and each Turn's work behind one card.
/// It appears at once with what the listing knows, a quiet loading mark standing where the conversation will be. At
/// its foot the bar says what the work is doing, with Stop, above the composer, whose send steers a working Turn and
/// whose held send queues; a Prompt queued waits as a dashed bubble at the foot of the conversation. An act's failure
/// is a banner over the screen, but a Session gone, which pops it.
struct SessionScreen: View {
  let model: SessionModel
  @Environment(\.dismiss) private var dismiss

  var body: some View {
    ScrollView {
      LazyVStack(alignment: .leading, spacing: 12) {
        ForEach(model.conversation.shown, id: \.id) { item in
          ItemView(item: item, route: model.route)
        }
        ForEach(model.frame?.queued ?? [], id: \.prompt) { queued in
          QueuedBubble(queued: queued, model: model)
        }
      }
      .padding(.horizontal, 14)
      .padding(.vertical, 8)
    }
    .defaultScrollAnchor(.bottom)
    .defaultScrollAnchor(.bottom, for: .sizeChanges)
    .scrollDismissesKeyboard(.interactively)
    .accessibilityIdentifier("session.conversation")
    .safeAreaInset(edge: .bottom, spacing: 0) {
      SessionFoot(model: model)
    }
    .safeAreaInset(edge: .top) {
      if let banner = model.banner {
        // On a ground of its own, so the conversation scrolling beneath does not show through.
        Banner(text: banner)
          .padding(.vertical, 4)
          .background(Color(.systemBackground))
          .onTapGesture { model.dismissBanner() }
          .accessibilityAddTraits(.isButton)
          .accessibilityHint("Dismisses this")
      }
    }
    .onChange(of: model.popped) {
      if model.popped {
        dismiss()
      }
    }
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

/// One item of the conversation, drawn as what it is: a Turn's card and a Compaction with more behind it push what
/// they hold over the Session `route` pushed.
private struct ItemView: View {
  let item: ConversationItem
  let route: SessionRoute

  var body: some View {
    switch item.kind {
    case .loading(let words):
      // Centred in what the conversation will fill.
      ProgressView()
        .controlSize(.regular)
        .containerRelativeFrame([.horizontal, .vertical])
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: words))
        .accessibilityIdentifier("session.loading")
    case .userMessage(let text, let note, let sending):
      UserBubble(text: text, note: note, sending: sending)
    case .delegation(let text, let from):
      DelegationBubble(text: text, from: from)
    case .agentMessage(let markdown, let model, _):
      AgentMessage(markdown: markdown, model: model)
    case .turn(let card):
      // The card pushes the Turn's work.
      NavigationLink(value: Pushed.work(route, turn: card.work)) {
        TurnCardView(card: card)
      }
      .buttonStyle(.plain)
      .accessibilityIdentifier("session.turn")
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
    case .compaction(let words, let tone, let detail):
      let compaction = Label {
        Text(verbatim: words)
      } icon: {
        CompactionSymbol(tone: tone)
      }
      .font(.footnote)
      .foregroundStyle(.secondary)
      if let detail {
        // What it was asked to keep, and what it left, stand behind it.
        NavigationLink(value: Pushed.detail(route, activity: detail)) {
          HStack {
            compaction
            Image(systemName: "chevron.right")
              .font(.caption2)
              .foregroundStyle(.tertiary)
              .accessibilityHidden(true)
          }
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("session.compaction")
      } else {
        compaction
          .accessibilityElement(children: .combine)
          .accessibilityIdentifier("session.compaction")
      }
    }
  }
}

/// What the user said, at the right: faded, with a clock beside it, while it is a Prompt this phone is sending.
private struct UserBubble: View {
  let text: String
  let note: String?
  let sending: Bool

  var body: some View {
    VStack(alignment: .trailing, spacing: 3) {
      HStack(alignment: .bottom, spacing: 6) {
        if sending {
          Image(systemName: "clock")
            .font(.caption)
            .foregroundStyle(.secondary)
            .accessibilityHidden(true)
        }
        Text(verbatim: text)
          .padding(.horizontal, 13)
          .padding(.vertical, 9)
          .foregroundStyle(.white)
          .background(Color.accentColor, in: RoundedRectangle(cornerRadius: 18))
          .opacity(sending ? 0.6 : 1)
      }
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
    .accessibilityIdentifier(sending ? "session.sendingMessage" : "session.userMessage")
  }
}

/// A Prompt queued for after the working Turn, waiting at the foot as a dashed bubble, with the acts that deliver it
/// now and withdraw it.
private struct QueuedBubble: View {
  let queued: QueuedPrompt
  let model: SessionModel

  var body: some View {
    let busy = model.acting.contains(queued.prompt)
    VStack(alignment: .trailing, spacing: 4) {
      Text(verbatim: queued.text)
        .padding(.horizontal, 13)
        .padding(.vertical, 9)
        .foregroundStyle(.secondary)
        .overlay(
          RoundedRectangle(cornerRadius: 18)
            .strokeBorder(.tint, style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
        )
        .accessibilityIdentifier("session.queuedText")
      if let note = queued.note {
        Text(verbatim: note)
          .font(.caption)
          .foregroundStyle(.secondary)
      }
      HStack(spacing: 12) {
        Text(verbatim: queued.label)
          .foregroundStyle(.secondary)
        if let steerNow = queued.steerNow {
          Button {
            Task { await model.steerNow(queued.prompt) }
          } label: {
            Text(verbatim: steerNow)
          }
          .accessibilityIdentifier("session.steerNow")
        }
        if let cancel = queued.cancel {
          Button {
            Task { await model.cancelQueued(queued.prompt) }
          } label: {
            Text(verbatim: cancel)
          }
          .accessibilityIdentifier("session.cancelQueued")
        }
      }
      .font(.caption)
      .disabled(busy)
      .opacity(busy ? 0.5 : 1)
    }
    .frame(maxWidth: .infinity, alignment: .trailing)
    .padding(.leading, 44)
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("session.queued")
  }
}

/// The foot of an open Session: the line an Unreachable Origin leaves, the bar while the Session works, and the
/// composer.
private struct SessionFoot: View {
  @Bindable var model: SessionModel

  var body: some View {
    if let frame = model.frame, frame.bar != nil || frame.composer != nil {
      VStack(spacing: 8) {
        if let line = frame.composer?.unreachable {
          Label {
            Text(verbatim: line)
          } icon: {
            Image(systemName: "wifi.slash")
          }
          .font(.caption)
          .foregroundStyle(.secondary)
          .frame(maxWidth: .infinity, alignment: .leading)
          .accessibilityElement(children: .combine)
          .accessibilityIdentifier("session.unreachable")
        }
        if let bar = frame.bar {
          BarView(bar: bar, stopping: model.stopping) {
            Task { await model.stop() }
          }
        }
        if let composer = frame.composer {
          ComposerView(composer: composer, model: model)
        }
      }
      .padding(.horizontal, 12)
      .padding(.vertical, 8)
      .background(.bar)
    }
  }
}

/// The bar above the composer: what the work is doing and for how long, what a Monitoring Session waits on, and Stop,
/// which takes one tap.
private struct BarView: View {
  let bar: Bar
  let stopping: Bool
  let stop: () -> Void

  var body: some View {
    let tint: Color = bar.kind == .monitoring ? .teal : .blue
    HStack(spacing: 8) {
      ProgressView()
        .controlSize(.mini)
        .accessibilityHidden(true)
      Text(verbatim: bar.words)
        .fontWeight(.medium)
        .fixedSize()
        .accessibilityIdentifier("session.barWords")
      TimelineView(.periodic(from: .now, by: 1)) { context in
        Text(verbatim: RunningTime.working(since: bar.since, now: context.date))
          .monospacedDigit()
          .foregroundStyle(.secondary)
          .fixedSize()
      }
      .fixedSize()
      // What a Monitoring Session waits on takes whatever width is left.
      Group {
        if let watching = bar.watching {
          Text(verbatim: watching)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.tail)
        } else {
          Color.clear.frame(height: 0)
        }
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      Button(action: stop) {
        Label {
          Text(verbatim: bar.stop)
        } icon: {
          Image(systemName: "stop.fill")
        }
      }
      .buttonStyle(.bordered)
      .tint(.red)
      .fixedSize()
      .disabled(stopping)
      .accessibilityIdentifier("session.stop")
    }
    .font(.footnote)
    .controlSize(.small)
    .padding(.leading, 12)
    .padding(.trailing, 6)
    .padding(.vertical, 6)
    .background(tint.opacity(0.12), in: Capsule())
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("session.bar")
  }
}

/// The composer: the field, and send, which steers a working Turn and, held, offers to queue for after it. The note a
/// send never answered leaves stands beneath, and so does what the core refused of the input.
private struct ComposerView: View {
  let composer: Composer
  @Bindable var model: SessionModel

  var body: some View {
    let empty = model.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    VStack(alignment: .leading, spacing: 4) {
      HStack(alignment: .bottom, spacing: 8) {
        TextField(text: $model.text, axis: .vertical) {
          Text(verbatim: composer.placeholder)
        }
        .lineLimit(1...6)
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 20))
        .accessibilityIdentifier("session.composer")
        if let queue = composer.queue {
          Menu {
            Button {
              Task { await model.queue() }
            } label: {
              Label {
                Text(verbatim: queue)
              } icon: {
                Image(systemName: "clock")
              }
            }
          } label: {
            SendSymbol()
          } primaryAction: {
            Task { await model.send() }
          }
          .disabled(empty)
          .accessibilityLabel(Text("Send"))
          .accessibilityIdentifier("session.send")
        } else {
          Button {
            Task { await model.send() }
          } label: {
            SendSymbol()
          }
          .disabled(empty)
          .accessibilityLabel(Text("Send"))
          .accessibilityIdentifier("session.send")
        }
      }
      if let note = composer.note {
        Text(verbatim: note)
          .font(.caption)
          .foregroundStyle(.secondary)
          .accessibilityIdentifier("session.note")
      }
      if let invalid = model.invalid {
        Text(verbatim: invalid)
          .font(.caption)
          .foregroundStyle(.red)
          .accessibilityIdentifier("session.invalid")
      }
    }
    .disabled(composer.unreachable != nil)
  }
}

private struct SendSymbol: View {
  var body: some View {
    Image(systemName: "arrow.up.circle.fill")
      .font(.system(size: 32))
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
      Image(systemName: "chevron.right")
        .font(.caption)
        .foregroundStyle(.tertiary)
        .accessibilityHidden(true)
    }
    .contentShape(RoundedRectangle(cornerRadius: 12))
    .padding(.horizontal, 12)
    .padding(.vertical, 10)
    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
    .padding(.trailing, 24)
    .accessibilityElement(children: .combine)
  }
}

/// The symbol a Compaction standing as `tone` is led by.
private struct CompactionSymbol: View {
  let tone: Tone

  var body: some View {
    switch tone {
    case .running: ProgressView().controlSize(.mini)
    case .succeeded, .quiet: Image(systemName: "rectangle.compress.vertical")
    case .waiting: Image(systemName: "hand.raised").foregroundStyle(.orange)
    case .failed: Image(systemName: "exclamationmark.triangle").foregroundStyle(.red)
    case .stopped: Image(systemName: "stop.circle").foregroundStyle(.yellow)
    case .unknown: Image(systemName: "questionmark.circle")
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

/// What a pushed Session holds while it is open, and its acts: what a failure of one says goes in a banner, beside
/// the input where the core refused the input, and where the Session is gone the screen pops.
@MainActor @Observable
final class SessionModel {
  /// The screen it was pushed as, which what it pushes is pushed over.
  let route: SessionRoute
  /// The reference the row it was opened from handed over.
  var session: String { route.session }
  private let core: any Core
  private let opened: any OpenedSession
  private(set) var frame: SessionFrame?
  private(set) var conversation = Conversation()
  /// What the composer's field holds, each change of which the core is handed, written over the version of the
  /// draft it last heard of.
  var text: String {
    didSet {
      guard text != oldValue else { return }
      invalid = nil
      core.setDraft(text, version: draftVersion, for: session)
    }
  }
  /// The version of the draft the core last told of: what the core gave back since stays ahead of what is typed.
  @ObservationIgnored private var draftVersion: UInt64
  /// An act's failure, in the core's words, shown over the screen.
  private(set) var banner: String?
  /// What the core refused of the input, shown beside it.
  private(set) var invalid: String?
  /// Whether the Session turned out gone, which pops its screen.
  private(set) var popped = false
  /// Whether a stop is under way, which dims Stop until it returns.
  private(set) var stopping = false
  /// The queued Prompts an act is under way on.
  private(set) var acting: Set<String> = []
  /// How long a failure's banner stands before it goes by itself.
  @ObservationIgnored private let failureStands: Duration
  @ObservationIgnored private var clearing: Task<Void, Never>?

  convenience init(core: any Core, session: String, failureStands: Duration = .seconds(6)) {
    self.init(core: core, route: SessionRoute(session: session), failureStands: failureStands)
  }

  init(core: any Core, route: SessionRoute, failureStands: Duration = .seconds(6)) {
    self.route = route
    let session = route.session
    self.core = core
    self.failureStands = failureStands
    opened = core.openSession(session)
    let draft = core.draft(for: session)
    text = draft.text
    draftVersion = draft.version
  }

  /// Takes the draft the core holds into the field: what it gave back always wins over what is typed meanwhile.
  private func take(_ draft: Draft) {
    draftVersion = draft.version
    if text != draft.text {
      text = draft.text
    }
  }

  /// Sends what the composer holds, steering the working Turn or beginning one. The field empties at once; where the
  /// send fails, it holds the draft the core gave the text back to.
  func send() async {
    await prompting { text async throws(ActError) in try await opened.send(text) }
  }

  /// Queues what the composer holds for after the working Turn, failing as a send does.
  func queue() async {
    await prompting { text async throws(ActError) in try await opened.queue(text) }
  }

  private func prompting(_ act: (String) async throws(ActError) -> Void) async {
    let sent = text
    guard !sent.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
    text = ""
    do {
      try await act(sent)
    } catch {
      failed(error)
      take(core.draft(for: session))
    }
  }

  /// Stops the Session's work; a Prompt the stop withdrew is back in the composer after.
  func stop() async {
    stopping = true
    defer { stopping = false }
    do {
      try await opened.stop()
      take(core.draft(for: session))
    } catch {
      failed(error)
    }
  }

  func steerNow(_ prompt: String) async {
    await acting(on: prompt) { () async throws(ActError) in try await opened.steerNow(prompt) }
  }

  func cancelQueued(_ prompt: String) async {
    await acting(on: prompt) { () async throws(ActError) in try await opened.cancelQueued(prompt) }
  }

  private func acting(on prompt: String, _ act: () async throws(ActError) -> Void) async {
    acting.insert(prompt)
    defer { acting.remove(prompt) }
    do {
      try await act()
    } catch {
      failed(error)
    }
  }

  /// Takes the banner away.
  func dismissBanner() {
    clearing?.cancel()
    clearing = nil
    banner = nil
  }

  private func failed(_ error: ActError) {
    switch error {
    case .Gone: popped = true
    case .Invalid(let message): invalid = message
    case .Unreachable(let message), .Refused(let message): show(message)
    }
  }

  private func show(_ message: String) {
    clearing?.cancel()
    banner = message
    let stands = failureStands
    clearing = Task { [weak self] in
      try? await Task.sleep(for: stands)
      guard !Task.isCancelled else { return }
      self?.banner = nil
    }
  }

  /// Follows the frame and the conversation until the screen goes.
  func follow() async {
    async let frames: Void = followFrame()
    async let changes: Void = followConversation()
    async let drafts: Void = followDraft()
    _ = await (frames, changes, drafts)
  }

  private func followDraft() async {
    for await draft in core.drafts(for: session) {
      take(draft)
    }
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

  /// The work of the Turn a card handed back, while the screen reading it is up.
  func work(_ turn: String) -> AsyncStream<TurnWork> {
    opened.work(turn)
  }

  /// The whole of the Activity a row handed back, while the screen reading it is up.
  func detail(_ activity: String) -> AsyncStream<ActivityDetail> {
    opened.detail(activity)
  }

  /// Ends the Session in the core, as its screen is popped.
  func close() {
    opened.close()
  }
}

/// The Sessions pushed over the listing, and what is pushed over each, as its navigation stack holds them: each
/// Session opened in the core as its screen is first drawn, and closed there once no screen of it is on the path,
/// however it leaves.
@MainActor @Observable
final class SessionStack {
  /// The screens pushed, the one on top last.
  var path: [Pushed] {
    didSet { closeUnpushed() }
  }
  @ObservationIgnored private let core: any Core
  @ObservationIgnored private var models: [SessionRoute: SessionModel] = [:]

  init(core: any Core, path: [Pushed] = []) {
    self.core = core
    self.path = path
  }

  /// The model of the screen `route` pushed, opened the first time it is asked for.
  func model(for route: SessionRoute) -> SessionModel {
    if let model = models[route] {
      return model
    }
    let model = SessionModel(core: core, route: route)
    models[route] = model
    return model
  }

  /// Closes every Session no screen of which is on the path.
  private func closeUnpushed() {
    let pushed = Set(path.map(\.route))
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

/// A screen pushed over the listing: a Session's, or a Turn's work or an Activity's whole pushed from one, which
/// read through the Session `route` names for as long as they are up.
enum Pushed: Hashable {
  case session(SessionRoute)
  case work(SessionRoute, turn: String)
  case detail(SessionRoute, activity: String)

  /// The Session's screen it is, or was pushed from.
  var route: SessionRoute {
    switch self {
    case .session(let route), .work(let route, _), .detail(let route, _): route
    }
  }
}

#if DEBUG
  #Preview("A working session") {
    NavigationStack {
      SessionScreen(model: SessionModel(core: FakeCore(.paired), session: "workstation-key/1"))
    }
  }

  #Preview("A session on an unreachable server") {
    NavigationStack {
      SessionScreen(model: SessionModel(core: FakeCore(.unreachable), session: "homelab-key/2"))
    }
  }

  #Preview("A settled session") {
    NavigationStack {
      SessionScreen(model: SessionModel(core: FakeCore(.paired), session: "workstation-key/7"))
    }
  }
#endif
