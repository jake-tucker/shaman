#if DEBUG
  import Foundation
  import ShamanCore
  import Synchronization

  /// What the fake's acts share with the fake core: each composer's draft, and whether any act is under way.
  final class FakeActs: Sendable {
    /// One composer's draft, versioned as the core versions it.
    private struct Held {
      var text = ""
      var version: UInt64 = 0
      /// The text given back at each version Swift may not have heard of yet.
      var given: [(UInt64, String)] = []
    }

    private let drafts = Mutex<[String: Held]>([:])
    private let draftListeners = Mutex<[String: Listeners<Draft>]>([:])
    private let underWay = Mutex(0)
    private let listeners = Listeners<Bool>()

    func stream() -> AsyncStream<Bool> {
      listeners.stream(now: underWay.withLock { $0 > 0 })
    }

    func draft(for session: String) -> Draft {
      drafts.withLock { drafts in
        let held = drafts[session] ?? Held()
        return Draft(text: held.text, version: held.version)
      }
    }

    func drafts(for session: String) -> AsyncStream<Draft> {
      draftListening(session).stream(now: draft(for: session))
    }

    /// Holds `text` written over `version`, keeping what was given back since ahead of it, as the core does.
    func setDraft(_ text: String, version: UInt64, for session: String) {
      let merged = drafts.withLock { drafts -> Draft? in
        var held = drafts[session] ?? Held()
        let unseen = held.given.filter { $0.0 > version }.map(\.1)
        if unseen.isEmpty {
          held.given = []
          held.text = text
          drafts[session] = held
          return nil
        }
        held.text = unseen.reduce(text) { written, given in Self.ahead(given, of: written) }
        drafts[session] = held
        return Draft(text: held.text, version: held.version)
      }
      if let merged {
        draftListening(session).tell(merged)
      }
    }

    /// Gives `text` back to the composer of `session`, ahead of anything written there, as the core does.
    func giveBack(_ text: String, to session: String) {
      let draft = drafts.withLock { drafts in
        var held = drafts[session] ?? Held()
        held.version += 1
        held.given.append((held.version, text))
        held.text = Self.ahead(text, of: held.text)
        drafts[session] = held
        return Draft(text: held.text, version: held.version)
      }
      draftListening(session).tell(draft)
    }

    private static func ahead(_ given: String, of written: String) -> String {
      written.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? given : "\(given)\n\(written)"
    }

    private func draftListening(_ session: String) -> Listeners<Draft> {
      draftListeners.withLock { listeners in
        if let held = listeners[session] {
          return held
        }
        let made = Listeners<Draft>()
        listeners[session] = made
        return made
      }
    }

    /// Runs `act`, marked under way while it runs.
    func during(_ act: () async throws(ActError) -> Void) async throws(ActError) {
      count(by: 1)
      defer { count(by: -1) }
      try await act()
    }

    private func count(by change: Int) {
      let busy = underWay.withLock { underWay in
        underWay += change
        return underWay > 0
      }
      listeners.tell(busy)
    }
  }

  /// A Session the fake holds open: framed at once, its conversation landing a moment later, a working Turn's answer
  /// streaming in by change, and its acts answered as the core would answer them. A Prompt holding "fail" is refused,
  /// one holding "gone" finds the Session gone, and one holding "unanswered" is never answered and may have been sent,
  /// so a screen can be seen failing each way. The Interventions its Turn waits on present themselves once it lands,
  /// oldest first, and wait behind "Review" once dismissed, as the core's do. Answers holding "fail" are refused.
  final class FakeOpenedSession: OpenedSession {
    private struct State {
      /// The frame of the Session's work, before its Interventions are shown in it.
      var frame: SessionFrame
      var items: [ConversationItem]
      var sent = 0
      /// The Interventions the Session waits on, oldest first.
      var pending: [Intervention] = []
      /// Those this screen dismissed.
      var dismissed: Set<String> = []

      /// The frame as the core shows it: the oldest Intervention not dismissed presented while the Origin answers,
      /// and otherwise the orange bar naming the oldest, with "Review" where there is anything to bring back.
      func shown(unreachable: Bool) -> SessionFrame {
        var shown = frame
        guard let oldest = pending.first else { return shown }
        let presenting = unreachable ? nil : pending.first { !dismissed.contains($0.reference) }
        shown.intervention = presenting
        if presenting == nil {
          let words =
            switch oldest {
            case .approval: "Approval waiting"
            case .questionnaire: "Questionnaire waiting"
            }
          shown.bar = Bar(
            kind: .waiting, words: words, since: nil, watching: nil, stop: "Stop",
            review: unreachable ? nil : "Review")
        }
        return shown
      }
    }

    private let session: String
    private let scenario: FakeConversation.Scenario?
    /// The tree the Session belongs to, where it is one of `FakeTree`'s.
    private let family: FakeTree?
    private let unreachable: Bool
    private let acts: FakeActs
    private let state: Mutex<State>
    private let frames = Listeners<SessionFrame>()
    private let changes = Mutex<[Int: AsyncStream<ConversationChange>.Continuation]>([:])
    private let nextListener = Atomic<Int>(0)
    private let closed = Atomic<Bool>(false)
    private let landing = Mutex<Task<Void, Never>?>(nil)
    private let following = Mutex<Task<Void, Never>?>(nil)

    init(
      session: String, frame: SessionFrame, scenario: FakeConversation.Scenario?, unreachable: Bool = false,
      acts: FakeActs, tree: FakeTree? = nil
    ) {
      self.session = session
      self.scenario = scenario
      family = tree
      self.unreachable = unreachable
      self.acts = acts
      state = Mutex(
        State(frame: frame, items: [ConversationItem(id: "loading", kind: .loading(words: "Loading the conversation"))])
      )
      guard let scenario else { return }
      if let tree {
        // A stop anywhere in the tree reaches this screen too.
        let follow = Task { [weak self] in
          for await _ in tree.changed() {
            guard let self, !Task.isCancelled else { return }
            changeFrame { $0 = tree.frame(of: session, top: $0) }
            // Once the conversation has landed, it moves with the tree.
            if state.withLock({ $0.items.first?.id != "loading" }) {
              change { $0 = tree.items(of: session) }
            }
          }
        }
        following.withLock { $0 = follow }
      }
      let task = Task { [weak self] in
        try? await Task.sleep(for: .milliseconds(600))
        self?.change { $0 = tree?.items(of: session) ?? FakeConversation.items(scenario) }
        self?.changeState { $0.pending = FakeConversation.interventions(scenario) }
        guard scenario == .working else { return }
        for text in FakeConversation.streamed.dropFirst() {
          try? await Task.sleep(for: .milliseconds(700))
          guard !Task.isCancelled else { return }
          self?.change { items in
            items = items.map {
              $0.id == "answer"
                ? ConversationItem(id: "answer", kind: .agentMessage(markdown: text, model: nil, streaming: true)) : $0
            }
          }
        }
      }
      landing.withLock { $0 = task }
    }

    var isClosed: Bool { closed.load(ordering: .relaxed) }

    func frame() -> AsyncStream<SessionFrame> {
      frames.stream(now: state.withLock { $0.shown(unreachable: unreachable) })
    }

    func conversation() -> AsyncStream<ConversationChange> {
      let id = nextListener.add(1, ordering: .relaxed).oldValue
      return AsyncStream(bufferingPolicy: .unbounded) { continuation in
        continuation.yield(.whole(items: state.withLock { $0.items }))
        changes.withLock { $0[id] = continuation }
        continuation.onTermination = { [weak self] _ in
          _ = self?.changes.withLock { $0.removeValue(forKey: id) }
        }
      }
    }

    func work(_ turn: String) -> AsyncStream<TurnWork> {
      guard let tree = family else { return FakeWork.work(of: turn, in: scenario) }
      let session = session
      return AsyncStream { continuation in
        continuation.yield(tree.work(of: session, turn: turn))
        let follow = Task {
          for await _ in tree.changed() {
            continuation.yield(tree.work(of: session, turn: turn))
          }
        }
        continuation.onTermination = { _ in follow.cancel() }
      }
    }

    func tree() -> AsyncStream<SubagentTree> {
      guard let tree = family else {
        return AsyncStream { continuation in
          continuation.yield(SubagentTree(title: "Subagents", count: nil, loading: nil, failed: nil, rows: []))
        }
      }
      return tree.stream(for: session)
    }

    func stopSubagent(_ session: String) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        guard let tree = family else { throw .Refused(message: "That subagent is not working") }
        try await tree.stop(session)
      }
    }

    func detail(_ activity: String) -> AsyncStream<ActivityDetail> {
      FakeWork.detail(of: activity, in: scenario)
    }

    func close() {
      closed.store(true, ordering: .relaxed)
      landing.withLock { $0?.cancel() }
      following.withLock { $0?.cancel() }
    }

    func send(_ text: String) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        let text = try prompt(text)
        let id = state.withLock { state in
          state.sent += 1
          return "sent-\(state.sent)"
        }
        changeFrame { $0.composer?.note = nil }
        change { $0.append(FakeConversation.user(id, text, sending: true)) }
        try? await Task.sleep(for: .milliseconds(2_500))
        let lowered = text.lowercased()
        if lowered.contains("unanswered") {
          // Its answer never came: it may have been sent, and is never sent again.
          change { $0.removeAll { $0.id == id } }
          acts.giveBack(text, to: session)
          changeFrame { $0.composer?.note = "The server did not answer, so this prompt may have been sent" }
          throw .Unreachable(message: "The server did not answer")
        }
        if lowered.contains("fail") || lowered.contains("gone") {
          change { $0.removeAll { $0.id == id } }
          acts.giveBack(text, to: session)
          throw lowered.contains("gone")
            ? .Gone(message: "This session no longer exists")
            : .Refused(message: "The server refused the prompt: its agent is not signed in")
        }
        change { items in
          items = items.map { $0.id == id ? FakeConversation.user(id, text) : $0 }
        }
        // A Prompt to a Session at rest begins a Turn.
        if state.withLock({ $0.frame.bar == nil }) {
          change { $0.append(FakeConversation.card("turn-\(id)", .working, "Working", nil)) }
          changeFrame { frame in
            frame.bar = Bar(kind: .working, words: "Working", since: Date(), watching: nil, stop: "Stop", review: nil)
            frame.composer = Self.composer(working: true, unreachable: nil)
          }
        }
      }
    }

    func queue(_ text: String) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        let text = try prompt(text)
        let id = state.withLock { state in
          state.sent += 1
          return "queued-\(state.sent)"
        }
        changeFrame {
          $0.queued.append(
            QueuedPrompt(prompt: id, text: text, note: nil, label: "Sending", steerNow: nil, cancel: nil))
        }
        try? await Task.sleep(for: .milliseconds(500))
        changeFrame { frame in
          frame.queued = frame.queued.map {
            $0.prompt == id
              ? QueuedPrompt(
                prompt: id, text: text, note: nil, label: "Queued", steerNow: "Steer now", cancel: "Cancel")
              : $0
          }
        }
      }
    }

    func steerNow(_ prompt: String) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        try? await Task.sleep(for: .milliseconds(300))
        let queued = try withdraw(prompt)
        change { $0.append(FakeConversation.user(queued.prompt, queued.text)) }
      }
    }

    func cancelQueued(_ prompt: String) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        try? await Task.sleep(for: .milliseconds(300))
        _ = try withdraw(prompt)
      }
    }

    func stop() async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        try reachable()
        if let tree = family {
          // A Subagent stops alone, with whatever it delegated; the Session heading the tree stops all of it.
          try await tree.stop(FakeTree.member(session) == nil ? nil : session)
          return
        }
        try? await Task.sleep(for: .milliseconds(400))
        landing.withLock { $0?.cancel() }
        if scenario == .undelivered {
          // Nothing works but the Prompt not yet delivered, which is withdrawn and given back.
          change { $0.removeAll { $0.id == "open" } }
          acts.giveBack(FakeConversation.undeliveredPrompt, to: session)
        } else {
          settleCard(.stopped, "Stopped after 4m 12s")
        }
        changeState { state in
          state.pending = []
          state.frame.bar = nil
          state.frame.composer = Self.composer(working: false, unreachable: nil)
        }
      }
    }

    func decide(_ approval: String, _ decision: DecisionKind) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        try reachable()
        try? await Task.sleep(for: .milliseconds(2_500))
        guard state.withLock({ $0.pending.contains { $0.reference == approval } }) else {
          throw .Refused(message: "That approval is no longer waiting")
        }
        if decision == .declineAndInterrupt {
          // Declined, and the Turn interrupted with it: nothing else it asked waits any more.
          settleCard(.stopped, "Stopped after 6m 03s")
          changeState { state in
            state.pending = []
            state.frame.bar = nil
            state.frame.composer = Self.composer(working: false, unreachable: nil)
          }
          return
        }
        answered(approval)
      }
    }

    func answer(_ questionnaire: String, _ answers: [GivenAnswer]) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        try await submitted(questionnaire)
        if answers.contains(where: { $0.text.lowercased().contains("fail") }) {
          throw .Refused(message: "The server refused the answers: the agent stopped listening for them")
        }
        answered(questionnaire)
      }
    }

    func decline(_ questionnaire: String) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        try await submitted(questionnaire)
        answered(questionnaire)
      }
    }

    /// Waits as a submission to the Questionnaire `questionnaire` does, refused where it no longer waits.
    private func submitted(_ questionnaire: String) async throws(ActError) {
      try reachable()
      try? await Task.sleep(for: .milliseconds(1_500))
      guard state.withLock({ $0.pending.contains { $0.reference == questionnaire } }) else {
        throw .Refused(message: "That questionnaire is no longer waiting")
      }
    }

    /// Takes the Intervention `reference` names away, answered: the Turn works again once nothing else waits.
    private func answered(_ reference: String) {
      let answeredAll = state.withLock { state in
        state.pending.removeAll { $0.reference == reference }
        return state.pending.isEmpty
      }
      changeState { _ in }
      if answeredAll {
        change { items in
          items = items.map { item in
            guard case .turn(let card) = item.kind, card.outcome == .waiting else { return item }
            return FakeConversation.card(item.id, .working, "Working", card.count)
          }
        }
      }
    }

    func dismiss() {
      changeState { state in
        state.dismissed.formUnion(state.pending.map(\.reference))
      }
    }

    func review() {
      changeState { $0.dismissed = [] }
    }

    /// Settles the card of the Turn that works, or waits on the reader, as `outcome` with `line`.
    private func settleCard(_ outcome: TurnOutcome, _ line: String) {
      change { items in
        items = items.map { item in
          guard case .turn(let card) = item.kind, card.outcome == .working || card.outcome == .waiting else {
            return item
          }
          return FakeConversation.card(item.id, outcome, line, card.count)
        }
      }
    }

    /// The composer of a Session over the fake, steering where a Turn works and taking nothing where its Origin is
    /// Unreachable, which `unreachable` says.
    static func composer(working: Bool, unreachable: String?) -> Composer {
      Composer(
        placeholder: working ? "Steer, or hold send to queue" : "Message",
        queue: working ? "Queue for after this turn" : nil, unreachable: unreachable, note: nil)
    }

    /// `text` as a Prompt is sent, or why it is not.
    private func prompt(_ text: String) throws(ActError) -> String {
      let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
      guard !text.isEmpty else { throw .Invalid(message: "Write something to send") }
      try reachable()
      return text
    }

    private func reachable() throws(ActError) {
      if unreachable {
        throw .Unreachable(message: "The server did not answer in time")
      }
    }

    /// Takes the queued Prompt `prompt` from the foot.
    private func withdraw(_ prompt: String) throws(ActError) -> QueuedPrompt {
      let queued = state.withLock { $0.frame.queued.first { $0.prompt == prompt } }
      guard let queued else { throw .Refused(message: "That prompt is no longer queued") }
      changeFrame { $0.queued.removeAll { $0.prompt == prompt } }
      return queued
    }

    /// Changes the conversation by `body`, telling each listener the items that changed and the order.
    private func change(_ body: (inout [ConversationItem]) -> Void) {
      let (changed, order) = state.withLock { state in
        let before = Dictionary(state.items.map { ($0.id, $0) }, uniquingKeysWith: { _, last in last })
        body(&state.items)
        return (state.items.filter { before[$0.id] != $0 }, state.items.map(\.id))
      }
      for continuation in changes.withLock({ Array($0.values) }) {
        continuation.yield(.changed(items: changed, order: order))
      }
    }

    private func changeFrame(_ body: (inout SessionFrame) -> Void) {
      changeState { body(&$0.frame) }
    }

    /// Changes what the fake holds by `body`, telling each listener the frame as the core would show it.
    private func changeState(_ body: (inout State) -> Void) {
      let unreachable = unreachable
      let frame = state.withLock { state in
        body(&state)
        return state.shown(unreachable: unreachable)
      }
      frames.tell(frame)
    }
  }
#endif
