#if DEBUG
  import Foundation
  import ShamanCore
  import Synchronization

  /// The tree of a fake Session whose Agent delegates, as the core would make it: the workstation's Session finding
  /// what holds the session lock, and the Subagents beneath it, working, settled, failed and monitoring, one of them
  /// spawning one of its own that cannot be stopped alone. Every Session of it reads its frame, conversation and work
  /// from here, and a stop from its sheet or a Subagent's bar reaches every screen of it. Where its Subagents wait on
  /// Interventions, as `Waits` says, the Session above marks its bar and rows with them, and each presents itself only
  /// in its own Session, an answer there reaching every screen of the tree.
  final class FakeTree: Sendable {
    /// One Session of the tree.
    struct Member: Sendable {
      let session: String
      let depth: UInt32
      let name: String
      let title: String
      let model: String
      /// How long before the fake opened it began working, or none where it is settled.
      let working: TimeInterval?
      /// How it settled and how long it worked, where it has.
      let settled: (Tone, String)?
      let monitoring: Bool
      /// Whether its Provider can stop it alone.
      let stoppable: Bool
      /// Who delegated its work, and what they asked.
      let delegatedBy: String
      let asked: String
      /// The Subagents it spawned, in the order they stand beneath it.
      let spawned: [String]
    }

    /// The top-level Session heading the tree.
    static let top = "workstation-key/12"
    static let title = "Find what holds the session lock"
    static let prompt = "The session lock was held all night. Find out what held it, and fix it."

    /// Every Subagent of the tree, in the Subagents Section's order: working branches first, newest spawn first.
    static let members: [Member] = [
      Member(
        session: "workstation-key/12.6", depth: 1, name: "Tester", title: "Run the lock's tests", model: "haiku",
        working: 64, settled: nil, monitoring: false, stoppable: true, delegatedBy: "The delegating agent",
        asked: "Run the session store's lock tests and say which fail.", spawned: []),
      Member(
        session: "workstation-key/12.1", depth: 1, name: "Explorer", title: "Find what held the lock", model: "haiku",
        working: 95, settled: nil, monitoring: false, stoppable: true, delegatedBy: "The delegating agent",
        asked: "Find what held the session lock last night. The logs are in ~/.local/state/shaman.",
        spawned: ["workstation-key/12.4"]),
      Member(
        session: "workstation-key/12.4", depth: 2, name: "Grepper", title: "Grep the logs for the lock",
        model: "haiku", working: 30, settled: nil, monitoring: false, stoppable: false, delegatedBy: "Explorer",
        asked: "Grep last night's logs for every line naming the session lock.", spawned: []),
      Member(
        session: "workstation-key/12.5", depth: 1, name: "Watcher", title: "Watch the nightly backup",
        model: "haiku", working: nil, settled: (.succeeded, "2m 5s"), monitoring: true, stoppable: true,
        delegatedBy: "The delegating agent", asked: "Watch tonight's backup and say if it takes the lock.",
        spawned: []),
      Member(
        session: "workstation-key/12.3", depth: 1, name: "Writer", title: "Draft the fix", model: "sonnet",
        working: nil, settled: (.failed, "12s"), monitoring: false, stoppable: true,
        delegatedBy: "The delegating agent", asked: "Draft a fix that releases the lock on shutdown.", spawned: []),
      Member(
        session: "workstation-key/12.2", depth: 1, name: "Reviewer", title: "Review the lock's tests",
        model: "sonnet", working: nil, settled: (.succeeded, "41s"), monitoring: false, stoppable: true,
        delegatedBy: "The delegating agent", asked: "Review the lock's tests for what they miss.", spawned: []),
    ]

    /// Which of the tree's Sessions wait on Interventions.
    enum Waits: Sendable {
      /// None.
      case none
      /// The top-level Session waits on an Approval of its own, and Explorer on one.
      case one
      /// Explorer waits on an Approval, and Grepper, beneath it, on a Questionnaire.
      case several
      /// Explorer, named at a length no bar has room for, waits on an Approval.
      case longNamed
    }

    /// The name Explorer goes by where its tree `Waits.longNamed`.
    static let longName = "security-code-reviewer-for-the-session-store-and-the-lock-it-holds"

    /// An Approval one of the tree's Sessions waits on.
    static let explorerApproval = PendingApproval(
      reference: "approval-explorer", title: "Run this command?",
      facts: [
        Fact(label: "Command", value: "rg session.lock ~/.local/state/shaman", style: .monospace),
        Fact(label: "Reason", value: "Reads the logs outside the workspace.", style: .prose),
      ], sections: [], note: nil,
      large: [
        DecisionOffer(decision: .decline, label: "Decline"),
        DecisionOffer(decision: .acceptOnce, label: "Accept once"),
      ],
      beneath: [
        DecisionOffer(decision: .declineAndInterrupt, label: "Decline and interrupt"),
        DecisionOffer(decision: .acceptForSession, label: "Accept for session"),
      ])

    /// The top-level Session's own Approval.
    static let topApproval = PendingApproval(
      reference: "approval-top", title: "Run this command?",
      facts: [Fact(label: "Command", value: "git stash list", style: .monospace)], sections: [], note: nil,
      large: explorerApproval.large, beneath: explorerApproval.beneath)

    /// Grepper's Questionnaire.
    static let grepperQuestionnaire = PendingQuestionnaire(
      reference: "questionnaire-grepper",
      questions: [
        PendingQuestion(
          position: "Question 1 of 1", heading: "Logs", text: "Which night's logs should I grep?", hint: nil,
          choices: [
            ChoiceCard(choice: "last", label: "Last night", description: "When the lock was held.", badge: nil),
            ChoiceCard(choice: "week", label: "The whole week", description: "Slower, and finds more.", badge: nil),
          ], multiple: false, field: nil, skip: nil, secret: nil)
      ], note: nil, decline: "Decline", next: "Next", send: "Send answers", back: "Back")

    private struct State {
      /// The Interventions answered, by reference.
      var answered: Set<String> = []
      /// The Subagents stopped, and how long each had worked.
      var stopped: [String: String] = [:]
      /// Whether the top-level Session was stopped.
      var topStopped = false
      var listeners: [Int: (open: String, continuation: AsyncStream<SubagentTree>.Continuation)] = [:]
      var next = 0
      /// Whether the tree has been read, which it is once, a while after the first Session of it opens.
      var landed = false
      var landing: Task<Void, Never>?
    }

    /// How the tree's stream goes.
    enum Reading: Sendable {
      /// It is read a while after the first Session of it opens.
      case lands
      /// It is never read.
      case never
      /// It cannot be read.
      case fails
    }

    private let opened: Date
    private let reading: Reading
    private let waits: Waits
    private let state = Mutex(State())
    private let changes = Listeners<Void>()

    init(opened: Date = Date(), reading: Reading = .lands, waits: Waits = .none) {
      self.opened = opened
      self.reading = reading
      self.waits = waits
    }

    /// The name `member` goes by in this tree.
    private func name(_ member: Member) -> String {
      waits == .longNamed && member.session == "workstation-key/12.1" ? Self.longName : member.name
    }

    /// Every Intervention the Session `session` of the tree was asked, oldest first, answered or not.
    private func asked(of session: String) -> [Intervention] {
      switch (waits, session) {
      case (.one, Self.top): [.approval(approval: Self.topApproval)]
      case (.one, "workstation-key/12.1"), (.several, "workstation-key/12.1"), (.longNamed, "workstation-key/12.1"):
        [.approval(approval: Self.explorerApproval)]
      case (.several, "workstation-key/12.4"): [.questionnaire(questionnaire: Self.grepperQuestionnaire)]
      default: []
      }
    }

    /// The Interventions the Session `session` of the tree waits on, oldest first.
    func interventions(of session: String) -> [Intervention] {
      let answered = state.withLock { $0.answered }
      return asked(of: session).filter { !answered.contains($0.reference) }
    }

    /// Takes the Intervention `reference` names away, answered, on every screen of the tree.
    func answer(_ reference: String) {
      let listeners = state.withLock { state in
        state.answered.insert(reference)
        return Array(state.listeners.values)
      }
      for (open, continuation) in listeners {
        continuation.yield(sheet(for: open))
      }
      changes.tell(())
    }

    /// What the Session `session` of the tree waits on, as a row says it: "Needs approval", "Needs an answer", "Needs
    /// you".
    private func needs(_ session: String) -> String? {
      Self.needs(interventions(of: session))
    }

    private static func needs(_ waiting: [Intervention]) -> String? {
      switch waiting.count {
      case 0: return nil
      case 1:
        if case .approval = waiting[0] { return "approval" }
        return "an answer"
      default: return "you"
      }
    }

    /// The Subagents beneath the Session `session`, at any depth, in the Subagents Section's order.
    private static func beneath(_ session: String) -> [Member] {
      let spawned: [Member] =
        if let above = member(session) { above.spawned.compactMap { member($0) } } else {
          members.filter { $0.depth == 1 }
        }
      return spawned.flatMap { (spawned: Member) -> [Member] in [spawned] + beneath(spawned.session) }
    }

    /// What waits in the Subagent `member` or beneath it.
    private func waiting(through member: Member) -> [Intervention] {
      ([member] + Self.beneath(member.session)).flatMap { interventions(of: $0.session) }
    }

    /// The orange bar of the Session `session` while its Subagents' Interventions wait, as the core makes it: the one
    /// by its name, its "Review" pushing its Session, or several by how many, their "Review" opening the tree.
    func waitingBeneath(_ session: String) -> Bar? {
      let waiting = Self.beneath(session).filter { !interventions(of: $0.session).isEmpty }
      let (words, opens): (String, ReviewOpens)
      switch waiting.count {
      case 0: return nil
      case 1:
        let needs = Self.needs(interventions(of: waiting[0].session)) ?? "you"
        (words, opens) = ("\(name(waiting[0])) needs \(needs)", .session(session: waiting[0].session))
      default: (words, opens) = ("\(waiting.count) subagents need you", .tree)
      }
      let stops = Self.member(session)?.stoppable ?? true
      return Bar(
        kind: .waiting, words: words, since: nil, watching: nil, stop: stops ? "Stop" : nil,
        review: Review(label: "Review", opens: opens))
    }

    /// The sheet while the tree is not in hand: "Loading", or why it cannot be read.
    private var unread: SubagentTree {
      SubagentTree(
        title: "Subagents", count: nil, loading: reading == .fails ? nil : "Loading",
        failed: reading == .fails ? "Could not load subagents: This session's tree is not on this server" : nil,
        rows: [])
    }

    /// Whether `session` is a Session of the tree.
    static func holds(_ session: String) -> Bool {
      session == top || members.contains { $0.session == session }
    }

    static func member(_ session: String) -> Member? {
      members.first { $0.session == session }
    }

    /// Each change to the tree, as a stop moves it.
    func changed() -> AsyncStream<Void> {
      changes.stream(now: ())
    }

    /// How long the tree takes to be read once the first Session of it opens, as a Server's stream takes a moment.
    private static let readTakes = Duration.seconds(2)

    /// Notes that a Session of the tree opened: the tree is read once, as the core reads it for as long as any of its
    /// Sessions is open, and every Session of it opened after joins what was read.
    func opening() {
      guard reading == .lands else { return }
      state.withLock { held in
        guard held.landing == nil else { return }
        held.landing = Task { [weak self] in
          try? await Task.sleep(for: Self.readTakes)
          guard let self else { return }
          let listeners = state.withLock { state in
            state.landed = true
            return Array(state.listeners.values)
          }
          for (open, continuation) in listeners {
            continuation.yield(sheet(for: open))
          }
          changes.tell(())
        }
      }
    }

    /// The tree as the sheet over `open` shows it, now and on every change: "Loading" until it has been read.
    func stream(for open: String) -> AsyncStream<SubagentTree> {
      AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
        let landed = state.withLock { state in
          state.next += 1
          state.listeners[state.next] = (open, continuation)
          return (state.next, state.landed)
        }
        continuation.yield(landed.1 ? sheet(for: open) : unread)
        continuation.onTermination = { [weak self] _ in
          _ = self?.state.withLock { $0.listeners.removeValue(forKey: landed.0) }
        }
      }
    }

    /// Stops the working Subagent `session`, and whatever it delegated, or, where it names none, everything the
    /// tree's top-level Session is doing.
    func stop(_ session: String?) async throws(ActError) {
      try? await Task.sleep(for: .milliseconds(300))
      let stopping: [Member]
      if let session {
        guard let member = Self.member(session), working(member) else {
          throw .Refused(message: "That subagent is not working")
        }
        stopping = [member] + member.spawned.compactMap(Self.member)
      } else {
        stopping = Self.members.filter(working)
      }
      let listeners = state.withLock { state in
        for stopped in stopping where stopped.working != nil {
          state.stopped[stopped.session] = "1m 35s"
        }
        state.topStopped = state.topStopped || session == nil
        return Array(state.listeners.values)
      }
      for (open, continuation) in listeners {
        continuation.yield(sheet(for: open))
      }
      changes.tell(())
    }

    private var topStopped: Bool {
      state.withLock { $0.topStopped }
    }

    private func stopped(_ member: Member) -> String? {
      state.withLock { $0.stopped[member.session] }
    }

    private func working(_ member: Member) -> Bool {
      member.working != nil && stopped(member) == nil
    }

    private var workingCount: Int {
      Self.members.filter(working).count
    }

    /// The button the tree is opened by.
    var button: TreeButton {
      // Before the tree is read the Session shows it has Subagents, but not how many work.
      guard state.withLock({ $0.landed }) else { return TreeButton(working: nil, label: "Subagents") }
      let working = workingCount
      return TreeButton(
        working: working > 0 ? "\(working)" : nil,
        label: working > 0 ? "\(Self.members.count) subagents, \(working) working" : "\(Self.members.count) subagents")
    }

    /// The sheet over `open`.
    func sheet(for open: String) -> SubagentTree {
      let topStopped = topStopped
      let topNeeds = needs(Self.top).map { "Needs \($0)" }
      let top = TreeRow(
        id: Self.top, session: Self.top, depth: 0, marker: topStopped ? .stopped : .running, title: Self.title,
        line: topStopped ? "Claude · opus · Stopped" : "Claude · opus",
        trailing: topNeeds ?? (topStopped ? "3m 10s" : nil), needsYou: topNeeds != nil,
        since: topStopped || topNeeds != nil ? nil : opened.addingTimeInterval(-190), open: open == Self.top, stop: nil)
      let rows = Self.members.map { member in
        let stoppedAfter = stopped(member)
        let working = member.working != nil && stoppedAfter == nil
        let (marker, worked): (Tone, String?) =
          if let stoppedAfter { (.stopped, stoppedAfter) } else if let settled = member.settled {
            settled
          } else { (.running, nil) }
        let outcome = marker == .failed ? "Failed" : marker == .stopped ? "Stopped" : nil
        // One waiting on the reader says what it waits on in place of its time.
        let needs = needs(member.session).map { "Needs \($0)" }
        return TreeRow(
          id: member.session, session: member.session, depth: member.depth, marker: marker, title: member.title,
          line: ([name(member), member.model] + [outcome].compactMap { $0 }).joined(separator: " · "),
          trailing: needs ?? (working ? nil : member.monitoring ? "monitoring" : worked), needsYou: needs != nil,
          since: member.working.flatMap { working && needs == nil ? opened.addingTimeInterval(-$0) : nil },
          open: open == member.session, stop: working && member.stoppable ? "Stop" : nil)
      }
      let working = workingCount
      return SubagentTree(
        title: "Subagents", count: working > 0 ? "\(Self.members.count) · \(working) working" : "\(Self.members.count)",
        loading: nil, failed: nil, rows: [top] + rows)
    }

    /// What frames the Session `session` of the tree now, the top-level Session's own framed as `top` is.
    func frame(of session: String, top: SessionFrame) -> SessionFrame {
      guard let member = Self.member(session) else {
        var frame = top
        // While the Agent only waits on its Subagents, the bar names which.
        let waiting = Self.members.filter { $0.depth == 1 && working($0) }
        frame.bar =
          waiting.isEmpty
          ? nil
          : Bar(
            kind: .working,
            words: waiting.count == 1 ? "Waiting on \(name(waiting[0]))" : "Waiting on \(waiting.count) subagents",
            since: opened.addingTimeInterval(-190), watching: nil, stop: "Stop", review: nil)
        frame.tree = button
        return frame
      }
      let bar: Bar? =
        if working(member), let began = member.working {
          Bar(
            kind: .working, words: "Working", since: opened.addingTimeInterval(-began), watching: nil,
            stop: member.stoppable ? "Stop" : nil, review: nil)
        } else if member.monitoring {
          Bar(
            kind: .monitoring, words: "Monitoring", since: opened.addingTimeInterval(-60),
            watching: "the nightly backup", stop: "Stop", review: nil)
        } else { nil }
      // A Subagent's Session takes no Prompt, so it has no composer at all.
      return SessionFrame(
        title: member.title, place: "\(name(member)) · \(member.model)", gone: nil, bar: bar, composer: nil, queued: [],
        intervention: nil, tree: button, waiting: [])
    }

    /// The conversation of the Session `session` of the tree.
    func items(of session: String) -> [ConversationItem] {
      let ownWaits = !interventions(of: session).isEmpty
      guard let member = Self.member(session) else {
        return [
          FakeConversation.user("open", Self.prompt),
          topStopped
            ? FakeConversation.card("work", .stopped, "Stopped after 3m 10s", "2 tools")
            : ownWaits
              ? FakeConversation.card("work", .waiting, "Waiting on you", "2 tools")
              : FakeConversation.card("work", .working, "Explorer: find what held the lock", "2 tools"),
        ]
      }
      let delegation = ConversationItem(
        id: "delegation", kind: .delegation(text: member.asked, from: member.delegatedBy))
      if working(member) {
        return [
          delegation,
          ownWaits
            ? FakeConversation.card("work", .waiting, "Waiting on you", "1 tool")
            : FakeConversation.card("work", .working, "rg session.lock", "1 tool"),
        ]
      }
      let line =
        if let stopped = stopped(member) { "Stopped after \(stopped)" } else if member.settled?.0 == .failed {
          "Failed after 12s"
        } else { "Worked for \(member.settled?.1 ?? "41s")" }
      let outcome: TurnOutcome =
        stopped(member) != nil ? .stopped : member.settled?.0 == .failed ? .failed : .worked
      return [
        delegation, FakeConversation.card("work", outcome, line, "2 tools"),
        ConversationItem(
          id: "answer",
          kind: .agentMessage(
            markdown: "The backup's `pg_dump` held the lock from 02:00 until it was killed at 06:10.", model: nil,
            streaming: false)),
      ]
    }

    /// The work of the Turn `turn` of the Session `session` of the tree.
    func work(of session: String, turn: String) -> TurnWork {
      let spawned = (Self.member(session)?.spawned ?? Self.members.filter { $0.depth == 1 }.map(\.session))
        .compactMap(Self.member)
      let rows =
        [
          WorkItem(
            id: "activity:read-store",
            kind: .row(
              row: WorkRow(
                glyph: "\u{eb6d}", title: "Read src/sessions/store.rs", titleStyle: .monospace, line: nil,
                trailing: nil, needsYou: false, tone: .succeeded, tap: .nothing)))
        ]
        + spawned.map { member in
          let stoppedAfter = stopped(member)
          let tone: Tone =
            if stoppedAfter != nil { .stopped } else if let settled = member.settled { settled.0 } else { .running }
          let worked = stoppedAfter ?? member.settled?.1
          let outcome = tone == .failed ? "Failed" : tone == .stopped ? "Stopped" : nil
          let trailing = ([outcome] + [worked]).compactMap { $0 }.joined(separator: " · ")
          // Where an Intervention waits in it or beneath it, it says so in orange.
          let needs = Self.needs(waiting(through: member)).map { "Needs \($0)" }
          return WorkItem(
            id: "activity:\(member.session)",
            kind: .row(
              row: WorkRow(
                glyph: "\u{f167a}", title: name(member), titleStyle: .prose, line: member.title,
                trailing: needs ?? (trailing.isEmpty ? nil : trailing), needsYou: needs != nil, tone: tone,
                tap: .session(session: member.session))))
        }
      let working = Self.member(session).map(working) ?? true
      return TurnWork(title: working ? "Working" : "Worked for 41s", items: rows, empty: nil)
    }
  }
#endif
