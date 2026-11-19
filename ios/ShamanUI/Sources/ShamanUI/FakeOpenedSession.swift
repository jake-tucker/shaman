#if DEBUG
  import Foundation
  import ShamanCore
  import Synchronization

  /// What the fake's acts share with the fake core: each composer's draft, and whether any act is under way. Where it
  /// is given a file, its drafts are kept there as text alone as the app leaves the front, and read back as it is
  /// made, as the core keeps them from one run to the next; otherwise they last for its life.
  final class FakeActs: Sendable {
    /// One composer's draft, versioned as the core versions it.
    private struct Held {
      var text = ""
      var version: UInt64 = 0
      /// The text given back at each version Swift may not have heard of yet.
      var given: [(UInt64, String)] = []
    }

    private let drafts: Mutex<[String: Held]>
    /// Where the drafts are kept, if anywhere.
    private let kept: URL?
    private let draftListeners = Mutex<[String: Listeners<Draft>]>([:])
    private let underWay = Mutex(0)
    private let listeners = Listeners<Bool>()

    /// The fake's acts, their drafts kept in `kept` where it names a file, and read back from it.
    init(keepingDraftsIn kept: URL? = nil) {
      self.kept = kept
      let read =
        kept.flatMap { try? Data(contentsOf: $0) }.flatMap {
          try? JSONDecoder().decode([String: String].self, from: $0)
        }
        ?? [:]
      drafts = Mutex(read.mapValues { Held(text: $0) })
    }

    /// Keeps every draft with anything in it in the file given, as the core does as the app leaves the front.
    func keepDrafts() {
      guard let kept else { return }
      let texts = drafts.withLock { drafts in
        drafts.mapValues(\.text).filter { !$0.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
      }
      do {
        try JSONEncoder().encode(texts).write(to: kept, options: .atomic)
      } catch {
        assertionFailure("The fake's drafts could not be kept: \(error)")
      }
    }

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

    /// Drops the draft of every Session whose reference begins with `prefix`, as a Remote removed does.
    func forgetDrafts(on prefix: String) {
      drafts.withLock { drafts in
        for session in drafts.keys where session.hasPrefix(prefix) {
          drafts[session] = nil
        }
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
    /// A Session being begun from the Landing: its Prompt, and how the fake's Server answered, where it has and not
    /// with the Session.
    private struct Provisional {
      var text: String
      var outcome: Outcome?
    }

    /// How a beginning stands where the fake's Server did not answer with the Session.
    private enum Outcome {
      case refused
      /// Its answer never came, so it may have begun.
      case unanswered
    }

    /// What a Session being begun tells the fake core.
    enum Told: Sendable {
      /// The fake's Server made it, with the Prompt it holds, so it is listed.
      case began(String)
      /// Its screen went while it may have begun, which the Landing says.
      case leftUnsure
    }

    private struct State {
      /// The frame of the Session's work, before its Interventions are shown in it.
      var frame: SessionFrame
      var items: [ConversationItem]
      var sent = 0
      /// The Interventions the Session waits on, oldest first.
      var pending: [Intervention] = []
      /// Those this screen dismissed.
      var dismissed: Set<String> = []
      /// The orange bar the Interventions of the Subagents beneath it make, where any wait.
      var beneath: Bar?
      /// The Session being begun, until the fake's Server answers with it.
      var provisional: Provisional?

      /// The frame as the core shows it: the oldest Intervention not dismissed presented while the Origin answers;
      /// and otherwise, once that is answered or dismissed, the orange bar of a Subagent's waiting, or of the oldest of
      /// its own, with "Review" where there is anything to lead to. Nothing is reviewed on an Unreachable Origin.
      func shown(unreachable: Bool) -> SessionFrame {
        var shown = frame
        shown.waiting = pending.map(\.reference)
        var beneath = beneath
        if unreachable {
          beneath?.review = nil
        }
        guard let oldest = pending.first else {
          shown.bar = beneath ?? shown.bar
          return shown
        }
        let presenting = unreachable ? nil : pending.first { !dismissed.contains($0.reference) }
        shown.intervention = presenting
        if presenting == nil {
          let words =
            switch oldest {
            case .approval: "Approval waiting"
            case .questionnaire: "Questionnaire waiting"
            }
          let own = Bar(
            kind: .waiting, words: words, since: nil, watching: nil, stop: "Stop",
            review: unreachable ? nil : Review(label: "Review", opens: .interventions))
          shown.bar = unreachable ? own : beneath ?? own
        }
        return shown
      }
    }

    private let session: String
    private let scenario: FakeConversation.Scenario?
    /// The tree the Session belongs to, where it is one of `FakeTree`'s.
    private let family: FakeTree?
    /// Whether the tree it belongs to is the Sidekick's `FakeSidekick` makes.
    private let sidekick: Bool
    private let unreachable: Bool
    private let acts: FakeActs
    private let state: Mutex<State>
    private let frames = Listeners<SessionFrame>()
    private let changes = Mutex<[Int: AsyncStream<ConversationChange>.Continuation]>([:])
    private let nextListener = Atomic<Int>(0)
    private let closed = Atomic<Bool>(false)
    private let landing = Mutex<Task<Void, Never>?>(nil)
    private let following = Mutex<Task<Void, Never>?>(nil)
    /// Where a Session being begun stands, beneath its Title.
    private let place: String
    /// Told what becomes of a Session being begun that the fake core keeps.
    private let told: (@Sendable (Told) -> Void)?
    /// Whether a Session being begun waits on a new Worktree its Server has yet to make.
    private let preparing = Atomic<Bool>(false)

    /// The key the Landing's draft is kept under, as the core names it.
    static let landingDraft = "new-session"

    /// A Session being begun as `session` with the Prompt `text` in the Workspace `place` names: the Provisional
    /// Session at once, and a moment later the Session the fake's Server made, which `began` is told of so it is
    /// listed. In a `newWorktree` the bar reads "Creating worktree" a moment first. A Prompt holding "refuse" is
    /// refused the first time it is asked, and one holding "unanswered" is never answered, so it may have begun; each
    /// begins when asked again.
    init(
      beginning session: String, text: String, place: String, newWorktree: Bool = false, acts: FakeActs,
      told: @escaping @Sendable (Told) -> Void
    ) {
      self.session = session
      scenario = nil
      family = nil
      sidekick = false
      unreachable = false
      self.acts = acts
      self.place = place
      self.told = told
      preparing.store(newWorktree, ordering: .relaxed)
      var beginning = State(
        frame: Self.provisionalFrame(text, place: place, answered: false, preparing: newWorktree),
        items: Self.provisionalItems(Provisional(text: text)))
      beginning.provisional = Provisional(text: text)
      state = Mutex(beginning)
      ask(text, again: false)
    }

    /// A Session the fake's Server made from the Landing, opened again from the listing.
    init(begun session: String, text: String, place: String, acts: FakeActs) {
      self.session = session
      scenario = nil
      family = nil
      sidekick = false
      unreachable = false
      self.acts = acts
      self.place = place
      told = nil
      state = Mutex(State(frame: Self.begunFrame(text, place: place), items: Self.begunItems(text)))
    }

    init(
      session: String, frame: SessionFrame, scenario: FakeConversation.Scenario?, unreachable: Bool = false,
      acts: FakeActs, tree: FakeTree? = nil, sidekick: Bool = false, unread: String? = nil
    ) {
      self.session = session
      self.scenario = scenario
      family = tree
      self.sidekick = sidekick
      self.unreachable = unreachable
      self.acts = acts
      place = ""
      told = nil
      state = Mutex(
        State(
          frame: frame,
          items: [
            ConversationItem(
              id: "loading", kind: unread.map { .unread(words: $0) } ?? .loading(words: "Loading the conversation"))
          ])
      )
      guard let scenario else { return }
      if let tree {
        // A stop anywhere in the tree reaches this screen too.
        let follow = Task { [weak self] in
          for await _ in tree.changed() {
            guard let self, !Task.isCancelled else { return }
            // Once the conversation has landed, it moves with the tree, and so do the Interventions it waits on.
            let landed = state.withLock { $0.items.first?.id != "loading" }
            changeState { state in
              state.frame = tree.frame(of: session, top: state.frame)
              state.beneath = tree.waitingBeneath(session)
              if landed {
                state.pending = tree.interventions(of: session)
              }
            }
            if landed {
              change { $0 = tree.items(of: session) }
            }
          }
        }
        following.withLock { $0 = follow }
      }
      let task = Task { [weak self] in
        try? await Task.sleep(for: .milliseconds(600))
        self?.change {
          $0 =
            tree?.items(of: session)
            ?? (sidekick && session == FakeSidekick.top ? FakeSidekick.items : FakeConversation.items(scenario))
        }
        self?.changeState { state in
          state.pending = tree?.interventions(of: session) ?? FakeConversation.interventions(scenario)
          state.beneath = tree?.waitingBeneath(session)
        }
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

    var reference: String { session }

    func tryAgain() async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        guard let text = state.withLock({ $0.provisional?.outcome != nil ? $0.provisional?.text : nil }) else {
          throw .Invalid(message: "Nothing is sent until the session has begun")
        }
        await asking(text, again: true)
      }
    }

    /// Asks the fake's Server to begin the Session with the Prompt `text`, `again` where it was refused before: in the
    /// background, so leaving the screen stops nothing.
    private func ask(_ text: String, again: Bool) {
      let acts = acts
      Task {
        try? await acts.during { () async throws(ActError) in await self.asking(text, again: again) }
      }
    }

    /// Asks the fake's Server to begin the Session with the Prompt `text`, `again` where it was refused or unanswered
    /// before, answering once it has.
    private func asking(_ text: String, again: Bool) async {
      let preparing = preparing.load(ordering: .relaxed)
      changeState { state in
        state.provisional = Provisional(text: text)
        state.frame = Self.provisionalFrame(text, place: place, answered: false, preparing: preparing)
      }
      change { $0 = Self.provisionalItems(Provisional(text: text)) }
      if preparing {
        // The new Worktree is made first, which takes a moment of its own.
        try? await Task.sleep(for: .milliseconds(2_000))
        self.preparing.store(false, ordering: .relaxed)
        changeFrame { $0 = Self.provisionalFrame(text, place: place, answered: false, preparing: false) }
      }
      // As long as a send to a fake Session takes, so a test sees it starting.
      try? await Task.sleep(for: .milliseconds(2_500))
      let lowered = text.lowercased()
      let outcome: Outcome? =
        again ? nil : lowered.contains("refuse") ? .refused : lowered.contains("unanswered") ? .unanswered : nil
      if let outcome {
        let left = state.withLock { state -> Bool in
          guard !isClosed else {
            state.provisional = nil
            return true
          }
          state.provisional?.outcome = outcome
          return false
        }
        if left {
          // The screen went: the Prompt is the Landing's draft again, the Landing saying where it may have begun.
          self.left(text, outcome)
          return
        }
        changeFrame { $0 = Self.provisionalFrame(text, place: place, answered: true, preparing: false) }
        change { $0 = Self.provisionalItems(Provisional(text: text, outcome: outcome)) }
        return
      }
      told?(.began(text))
      changeState { state in
        state.provisional = nil
        state.frame = Self.begunFrame(text, place: place)
      }
      change { $0 = Self.begunItems(text) }
    }

    /// Keeps the Prompt `text` of a beginning whose screen went as the Landing's draft, the Landing saying so where
    /// its `outcome` leaves it maybe begun.
    private func left(_ text: String, _ outcome: Outcome) {
      acts.giveBack(text, to: Self.landingDraft)
      if outcome == .unanswered {
        told?(.leftUnsure)
      }
    }

    /// What frames a Session being begun with the Prompt `text`: its first line as the Title, and "Creating worktree"
    /// while `preparing` one and "Starting" after, with no time and no Stop, until it is begun or, `answered`
    /// otherwise, a composer whose Prompt is written instead.
    private static func provisionalFrame(_ text: String, place: String, answered: Bool, preparing: Bool)
      -> SessionFrame
    {
      let title =
        text.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? ""
      return SessionFrame(
        title: title, place: place, gone: nil,
        bar: answered
          ? nil
          : Bar(
            kind: .working, words: preparing ? "Creating worktree" : "Starting", since: nil, watching: nil, stop: nil,
            review: nil),
        composer: Composer(
          placeholder: answered ? "Write a new prompt instead" : "Message", queue: nil, unreachable: nil, note: nil,
          canSend: answered),
        queued: [], intervention: nil, tree: nil, waiting: [])
    }

    private static func provisionalItems(_ provisional: Provisional) -> [ConversationItem] {
      let row: ConversationItem? =
        switch provisional.outcome {
        case nil: nil
        case .refused:
          ConversationItem(
            id: "refused",
            kind: .refused(
              words: "Could not create session: the workspace's directory could not be read", mayHaveBegun: false,
              tryAgain: "Try again"))
        case .unanswered:
          ConversationItem(
            id: "refused",
            kind: .refused(
              words: "The server did not answer, so the session may have begun", mayHaveBegun: true,
              tryAgain: "Try again"))
        }
      return [FakeConversation.user("prompt", provisional.text, sending: provisional.outcome == nil)]
        + (row.map { [$0] } ?? [])
    }

    /// What frames the Session the fake's Server made from the Prompt `text`: Working, with Stop.
    private static func begunFrame(_ text: String, place: String) -> SessionFrame {
      SessionFrame(
        title: text, place: place, gone: nil,
        bar: Bar(kind: .working, words: "Working", since: Date(), watching: nil, stop: "Stop", review: nil),
        composer: composer(working: true, unreachable: nil), queued: [], intervention: nil, tree: nil,
        waiting: [])
    }

    private static func begunItems(_ text: String) -> [ConversationItem] {
      [
        FakeConversation.user("prompt", text),
        FakeConversation.card("turn-begun", .working, "Reading the workspace", nil),
      ]
    }

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
      if sidekick && session == FakeSidekick.top {
        return AsyncStream { $0.yield(FakeSidekick.work()) }
      }
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
      if sidekick {
        let session = session
        return AsyncStream { $0.yield(FakeSidekick.sheet(for: session)) }
      }
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

    /// Ends the Session's reading as the core does where it is gone, saying `why`: what was read stays beneath it,
    /// and nothing is offered on it any more.
    func end(_ why: String) {
      changeState { state in
        state.pending = []
        state.beneath = nil
        state.frame.gone = why
        state.frame.bar = nil
        state.frame.composer = nil
        state.frame.queued = []
        state.frame.intervention = nil
        state.frame.tree = nil
      }
    }

    func close() {
      // A beginning refused or unanswered left is discarded, its Prompt the Landing's draft again.
      let discarded = state.withLock { state -> (String, Outcome)? in
        guard let provisional = state.provisional, let outcome = provisional.outcome else { return nil }
        state.provisional = nil
        return (provisional.text, outcome)
      }
      if let (text, outcome) = discarded {
        left(text, outcome)
      }
      closed.store(true, ordering: .relaxed)
      landing.withLock { $0?.cancel() }
      following.withLock { $0?.cancel() }
    }

    func send(_ text: String) async throws(ActError) {
      try await acts.during { () async throws(ActError) in
        let text = try prompt(text)
        if let provisional = state.withLock({ $0.provisional }) {
          // A Prompt written on a beginning refused or unanswered replaces its own; until then nothing is sent.
          guard provisional.outcome != nil else {
            acts.giveBack(text, to: session)
            throw .Invalid(message: "Nothing is sent until the session has begun")
          }
          await asking(text, again: false)
          return
        }
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
          // Its Agent took none of them: it asks again, saying so.
          changeState { state in
            state.pending = state.pending.map { intervention in
              guard case .questionnaire(var asked) = intervention, asked.reference == questionnaire else {
                return intervention
              }
              asked.note = "Your answers were not delivered. Look them over and send them again."
              return .questionnaire(questionnaire: asked)
            }
          }
          throw .Refused(message: "The agent did not take the answers.")
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

    /// Takes the Intervention `reference` names away, answered: the Turn works again once nothing else waits. One of a
    /// tree's is answered on every screen of it.
    private func answered(_ reference: String) {
      if let family {
        family.answer(reference)
        return
      }
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
        queue: working ? "Queue for after this turn" : nil, unreachable: unreachable, note: nil, canSend: true)
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
