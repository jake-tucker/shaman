#if DEBUG
  import Foundation
  import ShamanCore
  import Synchronization

  /// A stand-in for the device's half of the core, holding canned values of the types it hands Swift, for previews, UI
  /// tests and looking at a screen without a Server, and handing out a `FakeWindow` for each window. Debug builds alone
  /// carry it; a launch argument chooses it and its scenario.
  public final class FakeCore: DeviceCore {
    /// What the fake begins holding.
    public enum Scenario: String, CaseIterable, Sendable {
      /// A phone just installed, which has never been named and holds no Remote.
      case fresh
      /// A phone that has been given a name of its own, and holds no Remote.
      case named
      /// A named phone whose records were of a key it no longer has, so it must pair again.
      case pairAgain
      /// A named phone paired with the workstation alone, which has Sessions in every Standing and a long shelf.
      case paired
      /// A named phone paired with the workstation, a laptop and a home lab, whose Sessions stand together.
      case several
      /// The `several` phone while the home lab has yet to answer with its Sessions.
      case reaching
      /// The `several` phone while the home lab, whose every way is on the local network, reads Unreachable: its
      /// Sessions open and take nothing sent, and trying it again brings it back.
      case unreachable
      /// The `paired` phone with no network at all, so its one Remote reads Unreachable until a path is back.
      case offline
      /// The `paired` phone whose Session trees are never read, so a tree's sheet stands at "Loading".
      case unreadTree
      /// The `paired` phone whose Session trees cannot be read, so a tree's sheet says why.
      case unreadableTree
      /// The `paired` phone whose Subagents' stops are all refused, so the tree's sheet says why.
      case subagentStopRefused
      /// The `paired` phone where the Session heading the tree waits on an Approval of its own, and one of its
      /// Subagents, Explorer, on another.
      case subagentWaits
      /// The `paired` phone where two of the tree's Subagents wait: Explorer on an Approval, and Grepper, beneath it,
      /// on a Questionnaire.
      case subagentsWait
      /// The `subagentWaits` phone, but that `/12` waits on nothing of its own and Explorer goes by a name far longer
      /// than any bar has room for.
      case longSubagentName
      /// The `paired` phone whose Outlook remembers the workstation's `shaman` Workspace, so the Landing offers send
      /// there. A Prompt holding "refuse" is refused the first time it is asked.
      case composing
      /// The `several` phone, which also lists a Sidekick's Session on the workstation, `workstation-key/20`, whose tree
      /// stands its Subsessions and the Sessions it acted on across Servers, the phone not paired with some of them;
      /// the home lab reads Unreachable here, though the workstation still reaches it.
      case sidekick
      /// A named phone paired with a scratch Server alone, which holds no Sessions and lists no Workspaces.
      case emptyServer
      /// The `paired` phone whose workstation is also reached through `relay.example.com`, where the phone holds no
      /// login, so that way reads "login needed" and offers "Log in".
      case relayLogin
      /// The `relayLogin` phone logged in at the relay as octocat, which the Relays section lists.
      case relayed
      /// The `relayLogin` phone where the workstation answers by no direct way, so it reads Unreachable and its sheet
      /// leads to the login, after which it answers through the relay.
      case loginNeeded
      /// The `relayed` phone logged in at the relay as hubot, under another account than the workstation's, so the
      /// workstation reads Unreachable and the login is offered again.
      case otherAccount
      /// The `paired` phone whose workstation turns on `relay.example.com` a moment after the app opens, so that way
      /// appears beside its direct ways reading "login needed".
      case relayTurnedOn
      /// The `relayed` phone whose workstation turns the relay off a moment after the app opens, so that way goes,
      /// while the phone's login there stays listed.
      case relayTurnedOff
      /// The `several` phone across releases: the laptop no longer works with an app as old as this one and the home
      /// lab's Shaman is too old for the app, so each stands where an Unreachable Remote's row would, naming the side to
      /// update, and trying either again is refused so; and the workstation is newer, which the core says once.
      case releases
      /// The `several` phone whose workstation, where the Outlook stands, removes this phone a moment after the app
      /// opens, so the Outlook turns to the laptop, turned toward before it, saying why.
      case removedThere
      /// The `paired` phone whose one server, the workstation, removes this phone a moment after the app opens, so it
      /// offers pairing alone, saying why.
      case lastRemovedThere
      /// The `paired` phone whose listing also carries two long Sessions, `FakeLong`'s, of hundreds of items of very
      /// uneven height: `workstation-key/30`, settled, and `/31`, whose last answer still streams; and `/32`, whose
      /// answer is written again and again, long and short in turn; and `/33`, `/30` ending in eight one-line
      /// answers.
      case longSession
    }

    /// The relay the fake's workstation is reached through in the relay scenarios, and its Invite offers.
    public static let relay = "https://relay.example.com"

    /// The Invite the fake reads as the workstation's. One holding "dead" reads as the workstation's too, and fails to
    /// pair as an expired one does; one holding "unanswered" fails as one no way answered does; one holding "relay"
    /// offers `relay` after the direct ways, and pairs by them with no login; and one holding "relayonly" offers the
    /// relay alone, and pairs only once the phone is logged in there.
    public static let invite = "shaman-v1-fake-workstation"

    private let device: Mutex<ThisDevice>
    private let held: Mutex<[ServerRow]>
    private let deviceListeners = Listeners<ThisDevice>()
    private let serversListeners = Listeners<Servers>()
    /// Every window handed out, while it lives.
    private let windows = Mutex<[Held]>([])
    /// Every Session the fake's Remotes hold, the most recently begun first.
    private let sessions = Mutex(FakeSession.canned)
    /// The Remotes held that have yet to answer with their Sessions, which they never do.
    private let unanswered: [String]
    /// What it was told of the front, in order: true as the app came to it and false as it left.
    let toldOfFront = Mutex<[Bool]>([])
    /// The Remotes held that read Unreachable, and why.
    private let unreachable: Mutex<[String: UnreachableHint]>
    /// The Remotes held that answer but that the phone and they no longer work together across, and which side is to
    /// be updated: `.updateApp` or `.updateServer`.
    private let behind: [String: UnreachableHint]
    /// That a Remote is newer than the app, which every window says, a window made later included.
    let newer: Notice?
    /// How long trying a Remote again takes to answer.
    private let tryingTakes = Duration.milliseconds(400)

    /// Each composer's draft, and whether an act is under way.
    let acts: FakeActs
    /// The tree of the Session whose Agent delegates, which every Session of it reads.
    private let tree: FakeTree
    /// Whether the phone holds the Sidekick's Session `FakeSidekick` makes the tree of.
    private let sidekick: Bool

    /// How this phone stands at each relay it is logged in at, by its address.
    private let relayStandings: Mutex<[String: FakeRelayStanding]>
    /// The Remotes, by name, also reached through `relay`.
    private let relayed: Mutex<Set<String>>
    /// How long after the fake opens its workstation turns the relay on or off, in `relayTurnedOn` and
    /// `relayTurnedOff`: `--relay-told-after <seconds>` makes it as long as a UI test needs.
    private let relayToldAfter: Duration
    /// The logins under way, each given up as the app leaves the front.
    private let loggingIn = Mutex<[Int: FakeLogin]>([:])
    private let nextLogin = Atomic<Int>(0)
    /// How long a login waits before the fake's relay confirms it: `--login-confirms-after <seconds>` makes it as long
    /// as a UI test needs.
    private let loginConfirms: Duration

    /// The fake beginning as `scenario` says, its relay confirming a login after `loginConfirmsAfter` where given, and
    /// keeping its drafts in the file `keepingDraftsIn` names from one run to the next, where one is named.
    public init(_ scenario: Scenario, loginConfirmsAfter: Duration? = nil, keepingDraftsIn drafts: URL? = nil) {
      acts = FakeActs(keepingDraftsIn: drafts)
      device = Mutex(Self.device(in: scenario))
      let relayScenarios: [Scenario] = [.relayLogin, .relayed, .loginNeeded, .otherAccount, .relayTurnedOff]
      relayed = Mutex(relayScenarios.contains(scenario) ? ["workstation"] : [])
      let standings: [String: FakeRelayStanding] =
        switch scenario {
        case .relayed, .relayTurnedOff: [Self.relay: .loggedIn("octocat (github)")]
        case .otherAccount: [Self.relay: .otherAccount("hubot (github)")]
        default: [:]
        }
      relayStandings = Mutex(standings)
      let arguments = ProcessInfo.processInfo.arguments
      if let loginConfirmsAfter {
        loginConfirms = loginConfirmsAfter
      } else if let flag = arguments.firstIndex(of: "--login-confirms-after"), arguments.indices.contains(flag + 1),
        let seconds = Double(arguments[flag + 1])
      {
        loginConfirms = .seconds(seconds)
      } else {
        loginConfirms = .seconds(3)
      }
      if let flag = arguments.firstIndex(of: "--relay-told-after"), arguments.indices.contains(flag + 1),
        let seconds = Double(arguments[flag + 1])
      {
        relayToldAfter = .seconds(seconds)
      } else {
        relayToldAfter = .seconds(3)
      }
      let waits: FakeTree.Waits =
        switch scenario {
        case .subagentWaits: .one
        case .subagentsWait: .several
        case .longSubagentName: .longNamed
        default: .none
        }
      tree = FakeTree(
        reading: scenario == .unreadTree ? .never : scenario == .unreadableTree ? .fails : .lands, waits: waits,
        refusesStops: scenario == .subagentStopRefused)
      let remotes: [ServerRow] =
        switch scenario {
        case .fresh, .named, .pairAgain: []
        case .paired, .offline, .unreadTree, .unreadableTree, .subagentStopRefused, .subagentWaits, .subagentsWait,
          .longSubagentName, .composing, .relayLogin, .relayed, .loginNeeded, .otherAccount, .relayTurnedOn,
          .relayTurnedOff, .lastRemovedThere, .longSession:
          [Self.workstation(named: "workstation")]
        case .several, .reaching, .unreachable, .sidekick, .releases, .removedThere:
          [Self.workstation(named: "workstation"), Self.laptop, Self.homelab]
        case .emptyServer: [Self.scratch]
        }
      sidekick = scenario == .sidekick
      held = Mutex(remotes)
      unanswered = scenario == .reaching ? ["homelab"] : []
      let unreachable: [String: UnreachableHint] =
        switch scenario {
        case .unreachable, .sidekick: ["homelab": .localNetwork]
        case .offline: ["workstation": .noNetwork]
        case .loginNeeded: ["workstation": .loginNeeded]
        case .otherAccount: ["workstation": .otherAccount]
        default: [:]
        }
      self.unreachable = Mutex(unreachable)
      behind = scenario == .releases ? ["laptop": .updateApp, "homelab": .updateServer] : [:]
      newer =
        scenario == .releases
        ? Notice(id: 1, words: "workstation runs a newer Shaman; some things may not show") : nil
      if scenario == .several || scenario == .reaching || scenario == .unreachable || scenario == .sidekick
        || scenario == .releases
      {
        turn("workstation")
      }
      if scenario == .removedThere {
        turn("laptop")
        turn("workstation")
      }
      if scenario == .removedThere || scenario == .lastRemovedThere {
        // `--removed-after <seconds>` makes the moment as long as a UI test needs.
        var after = Duration.seconds(3)
        if let flag = arguments.firstIndex(of: "--removed-after"), arguments.indices.contains(flag + 1),
          let seconds = Double(arguments[flag + 1])
        {
          after = .seconds(seconds)
        }
        Task { [weak self] in
          try? await Task.sleep(for: after)
          guard let self, let row = self.held.withLock({ $0.first { $0.name == "workstation" } }) else { return }
          self.forget(row, removedThere: true)
        }
      }
      if scenario == .composing {
        turnedWorkspace.withLock { $0["workstation"] = "shaman" }
      }
      if sidekick {
        sessions.withLock { $0 = FakeSidekick.listed + $0 }
      }
      if scenario == .longSession {
        sessions.withLock { $0 = FakeLong.listed + $0 }
      }
      if scenario == .relayTurnedOn || scenario == .relayTurnedOff {
        let after = relayToldAfter
        let on = scenario == .relayTurnedOn
        Task { [weak self] in
          try? await Task.sleep(for: after)
          self?.workstationServesThroughRelay(on)
        }
      }
    }

    /// The workstation tells the phone it Serves through the relay, or no longer does, as the core follows what a
    /// Remote tells.
    private func workstationServesThroughRelay(_ serves: Bool) {
      relayed.withLock { relayed in
        if serves {
          relayed.insert("workstation")
        } else {
          relayed.remove("workstation")
        }
      }
      tellServers()
      tellWindows { $0.tellListing() }
    }

    public func window(kept: String?) -> any WindowCore {
      let window = FakeWindow(over: self)
      if let kept {
        window.restoreOutlook(kept)
      }
      return window
    }

    private let windowsMade = Atomic<Int>(0)

    /// The number of the next window made over this fake.
    func nextWindow() -> Int {
      windowsMade.add(1, ordering: .relaxed).newValue
    }

    /// How many windows have been made over this fake, for a test to see none is made that never stands.
    var windowCount: Int { windowsMade.load(ordering: .relaxed) }

    /// What was last half given each Questionnaire, by its Session and its reference, for the fake's life.
    private let halfAnswers = Mutex<[String: HalfAnswered]>([:])

    public func halfAnswered(_ questionnaire: String, of session: String) -> HalfAnswered? {
      halfAnswers.withLock { $0["\(session) \(questionnaire)"] }
    }

    public func keepHalfAnswered(_ answers: HalfAnswered, to questionnaire: String, of session: String) {
      halfAnswers.withLock { $0["\(session) \(questionnaire)"] = answers }
    }

    public func forgetHalfAnswered(_ questionnaire: String, of session: String) {
      _ = halfAnswers.withLock { $0.removeValue(forKey: "\(session) \(questionnaire)") }
    }

    /// What was said while no window lived, which the first window to come says, and no other.
    private let unclaimed = Mutex<[String]>([])

    /// What was said while no window lived, taken by the window asking, the first to come.
    func takeUnclaimed() -> [String] {
      unclaimed.withLock { unclaimed in
        defer { unclaimed.removeAll() }
        return unclaimed
      }
    }

    /// Tells `window`, made over this fake, of what changes, while it lives.
    func hold(_ window: FakeWindow) {
      windows.withLock { windows in
        windows.removeAll { $0.window == nil }
        windows.append(Held(window: window))
      }
    }

    /// Has every window that lives do `tell`.
    private func tellWindows(_ tell: (FakeWindow) -> Void) {
      for window in windows.withLock({ $0.compactMap(\.window) }) {
        tell(window)
      }
    }

    public func settle(_ session: String) async throws(ActError) {
      try await settling(session, to: true)
    }

    public func unsettle(_ session: String) async throws(ActError) {
      try await settling(session, to: false)
    }

    /// The Prompt each Session begun from the Landing was begun with, by the Session.
    private let begun = Mutex<[String: String]>([:])
    /// How many Sessions have been begun from the Landing.
    private let begunCount = Atomic<Int>(0)

    /// The number of the next Session begun from a Landing, in any window.
    func nextBegun() -> Int {
      begunCount.add(1, ordering: .relaxed).newValue
    }

    /// Lists the Session `session` the fake's Server made from the Prompt `text`, on `remote` in `workspace`, first.
    func list(_ session: String, begunWith text: String, on remote: String, in workspace: String) {
      begun.withLock { $0[session] = text }
      sessions.withLock {
        $0.insert(
          FakeSession(
            id: session, remote: remote, workspace: workspace, title: text, glyph: nil, checkout: nil,
            standing: .working, running: 0, moved: 0, settled: false, reads: .working), at: 0)
      }
      tellWindows { $0.tellListing() }
    }

    /// The Landing of a window whose Outlook stands at `remote`, in `workspace` where it remembers one, saying `note`,
    /// as the core would show it: naming the Remote only beside others.
    func landing(on remote: String, in workspace: String?, note: String?) -> Landing {
      let named = held.withLock { $0.count > 1 }
      let listed = workspace.flatMap { FakeWorkspace.find(remote, $0) }
      let cut = unreachable.withLock { $0[remote] != nil }
      let workingCopy = workspace != nil && listed.map({ if case .bare = $0.source { true } else { false } }) == true
      return Landing(
        title: "New session",
        place: ((named ? [remote] : []) + [workspace].compactMap { $0 }).joined(separator: " · "),
        workspace: workspace.map {
          LandingWorkspace(
            id: FakeWorkspace.reference(remote, $0), glyph: listed?.glyph, name: $0, origin: remote,
            description: listed?.description)
        },
        asks: workspace == nil ? "Choose a workspace\(named ? " on \(remote)" : "") to begin in." : nil,
        elsewhere: "Begin somewhere else",
        worktree: workspace == nil ? nil : listed?.worktree,
        workingCopy: workingCopy ? FakeWorkspace.workingCopy : nil,
        agent: Self.agents[remote],
        unreachable: cut
          ? "\(named ? remote : "The server") is unreachable. Nothing begins there until it answers."
          : behind[remote].map { apart in
            let named = named ? remote : "The server"
            return apart == .updateApp
              ? "\(named) no longer works with an app as old as this one: update the app. Nothing begins there until "
                + "then."
              : "\(named) runs a Shaman too old to work with this app: update Shaman there. Nothing begins there "
                + "until then."
          },
        note: note, placeholder: "Message",
        canSend: workspace != nil && !cut && !workingCopy && behind[remote] == nil,
        draft: FakeOpenedSession.landingDraft)
    }

    /// The Workspace Picker as the core would show it in a window whose Outlook stands at `chosen`, searched for
    /// `query`.
    func picker(chosen: (String, String)?, query: String) -> WorkspacePicker {
      FakeWorkspace.picker(
        on: held.withLock { $0.map(\.name) }, unanswered: unanswered,
        unreachable: Set(unreachable.withLock { $0.keys }), chosen: chosen, query: query)
    }

    /// The Agent each of the fake's Remotes says a new Session would get: the home lab's has no Provider able to
    /// serve, and the laptop is too old to say.
    private static let agents = ["workstation": "Claude · Fable 5.2 · high", "homelab": "Agent unavailable"]

    /// The Session `session` names opened for a window's screen, and what the listing says of it, where it lists it,
    /// which the window turns its Outlook toward.
    func open(_ session: String) -> (FakeOpenedSession, FakeSession?) {
      if let text = begun.withLock({ $0[session] }), let found = sessions.withLock({ $0.first { $0.id == session } }) {
        let place = ((held.withLock { $0.count > 1 } ? [found.remote] : []) + [found.workspace])
          .joined(separator: " · ")
        return (FakeOpenedSession(begun: session, text: text, place: place, acts: acts), nil)
      }
      let remotes = held.withLock { $0.map(\.name) }
      let found = sessions.withLock { $0.first { $0.id == session && remotes.contains($0.remote) } }
      let cut = found.map { found in unreachable.withLock { $0[found.remote] != nil } } ?? false
      // A Subagent's Session is listed nowhere, and is read from its tree.
      let delegated = FakeTree.member(session) != nil && remotes.contains("workstation")
      let gone = SessionFrame(
        title: "", place: "", gone: "This session no longer exists", bar: nil, composer: nil, queued: [],
        intervention: nil, tree: nil, waiting: [])
      var listed = found?.frame(originNamed: remotes.count > 1, unreachable: cut, now: Date())
      // A Session the Sidekick has a hand in that no listing carries is read from its Remote all the same.
      let remote = String(session.prefix { $0 != "-" })
      let unlistedCut = sidekick && unreachable.withLock { $0[remote] != nil }
      let handIn =
        sidekick
        ? FakeSidekick.frame(
          of: session,
          unreachable: unlistedCut ? "\(remote) is unreachable. Nothing sent goes anywhere until it answers." : nil)
        : nil
      if sidekick && session == FakeSidekick.top {
        listed?.tree = FakeSidekick.button
      }
      let frame =
        FakeTree.holds(session) && (found != nil || delegated)
        ? tree.frame(of: session, top: listed ?? gone) : listed ?? handIn ?? gone
      // A Session on a Remote yet to answer is never read: its loading mark stands.
      let read =
        found.flatMap { unanswered.contains($0.remote) ? nil : $0.reads }
        ?? (delegated ? .delegating : handIn != nil ? .settled : nil)
      if FakeTree.holds(session) && read != nil {
        tree.opening()
      }
      // A Session on a Remote across the gap is never read: why stands where its conversation would, and its
      // composer takes nothing.
      if let found, let apart = behind[found.remote] {
        let named = remotes.count > 1 ? found.remote : "The server"
        let why =
          apart == .updateApp
          ? "\(named) no longer works with an app as old as this one: update the app"
          : "\(named) runs a Shaman too old to work with this app: update Shaman there"
        var cut = frame
        cut.composer?.unreachable = "\(why). Nothing sent goes anywhere until then."
        cut.composer?.canSend = false
        cut.bar = nil
        let opened = FakeOpenedSession(
          session: session, frame: cut, scenario: nil, acts: acts,
          unread: "This session cannot be read yet. \(why), and it is read as soon as it can be.")
        return (opened, found)
      }
      let opened = FakeOpenedSession(
        session: session, frame: frame, scenario: read, unreachable: cut || (handIn != nil && unlistedCut),
        acts: acts, tree: FakeTree.holds(session) && read != nil ? tree : nil,
        sidekick: sidekick && FakeSidekick.holds(session))
      return (opened, found)
    }

    public func actsUnderWay() -> AsyncStream<Bool> {
      acts.stream()
    }

    /// Gives `text` back to the composer of `session` as the core does after a failed send, for a test to see what
    /// the field makes of it.
    static func giveBack(_ text: String, in window: FakeWindow, to session: String) {
      window.fake.acts.giveBack(text, to: session)
    }

    /// The Workspace last turned toward on each Remote, by any window.
    private let turnedWorkspace = Mutex<[String: String]>([:])

    /// Every Remote read again answers but where the phone has no network, after a moment.
    public func tryAgain(_ remote: String) async throws(ActError) {
      try? await Task.sleep(for: tryingTakes)
      let names = held.withLock { $0.filter { $0.fingerprint == remote }.map(\.name) }
      guard let name = names.first else {
        throw .Gone(message: "This phone is no longer paired with that server")
      }
      switch behind[name] {
      case .updateApp:
        throw .Refused(
          message: "That server no longer works with an app as old as this one: update the app, and the pairing works "
            + "again")
      case .updateServer:
        throw .Refused(
          message: "That server's Shaman is too old to work with this app: update Shaman there, and the pairing works "
            + "again")
      default: break
      }
      let loggedIn = relayStandings.withLock { $0[Self.relay] }.map { if case .loggedIn = $0 { true } else { false } }
      let stillUnreachable = unreachable.withLock { unreachable -> String? in
        switch unreachable[name] {
        case .noNetwork: return "This phone has no network connection"
        case .loginNeeded, .otherAccount: if loggedIn != true { return "No way to the server answered" }
        default: break
        }
        unreachable[name] = nil
        return nil
      }
      if let stillUnreachable {
        throw .Unreachable(message: stillUnreachable)
      }
      tellWindows { window in
        window.tellListing()
        window.tellLanding()
        window.tellWorkspaces()
      }
    }

    public func cameToFront() {
      toldOfFront.withLock { $0.append(true) }
    }

    /// Every draft is kept, and every login under way given up, as the core does both.
    public func leftFront() {
      toldOfFront.withLock { $0.append(false) }
      acts.keepDrafts()
      let given = loggingIn.withLock { logins in
        defer { logins.removeAll() }
        return Array(logins.values)
      }
      for login in given {
        login.task.cancel()
        login.continuation.yield(
          .failed(
            title: login.title,
            message: "The login was given up as Shaman left the screen. Log in again for a new code.",
            again: "Try again"))
      }
      if !given.isEmpty {
        tellServers()
      }
    }

    /// The fake's network never changes: its scenario says which Remotes read Unreachable.
    public func pathChanged(hasPath: Bool) {}

    private func settling(_ session: String, to settled: Bool) async throws(ActError) {
      try? await Task.sleep(for: .milliseconds(150))
      let found = sessions.withLock { sessions in
        guard let index = sessions.firstIndex(where: { $0.id == session }) else { return false }
        sessions[index].settled = settled
        if !settled {
          sessions[index].moved = 0
        }
        return true
      }
      guard found else {
        throw .Gone(message: "That session is no longer listed")
      }
      tellWindows { $0.tellListing() }
    }

    /// The names of the Remotes held, in the order paired.
    func heldNames() -> [String] {
      held.withLock { $0.map(\.name) }
    }

    /// Every Session the fake's Remotes hold.
    func listed() -> [FakeSession] {
      sessions.withLock { $0 }
    }

    /// The listing of every held Remote's Sessions, as the core would make it for a window showing `showing` of the
    /// shelf's rows, narrowed to `scope` and searched for `query`.
    func listing(showing: Int, scope: FakeScope, query: String) -> Listing {
      let remotes = held.withLock { $0.map(\.name) }
      var listing = FakeSession.listing(
        of: sessions.withLock { $0 }, on: remotes, unanswered: unanswered,
        unreachable: unreachable.withLock { $0 }.merging(behind) { held, _ in held }, showing: showing,
        scope: scope, query: query, now: Date())
      // A Remote renamed is named so where a row names its Origin.
      for (was, now) in renamed.withLock({ $0 }) {
        for index in listing.active.indices where listing.active[index].place.hasPrefix("\(was) · ") {
          listing.active[index].place = now + listing.active[index].place.dropFirst(was.count)
        }
      }
      return listing
    }

    public func thisDevice() -> AsyncStream<ThisDevice> {
      deviceListeners.stream(now: device.withLock { $0 })
    }

    public func servers() -> AsyncStream<Servers> {
      serversListeners.stream(now: currentServers())
    }

    /// The Servers and Relays sections as the core would show them: a Remote reached through the relay with that way
    /// as this phone stands there.
    private func currentServers() -> Servers {
      let standings = relayStandings.withLock { $0 }
      let relayed = relayed.withLock { $0 }
      let remotes = held.withLock { $0 }.map { held in
        var row = held
        row.name = shown(held.name)
        row.removal = Self.removal(of: row.name)
        if relayed.contains(held.name) {
          row.ways.append(Self.relayWay(standings[Self.relay]))
        }
        return row
      }
      let underWay = !loggingIn.withLock { $0.isEmpty }
      var relays = standings.sorted { $0.key < $1.key }.map { relay, standing in
        Self.relayRow(relay, standing: underWay ? .loggingIn(standing.account) : standing)
      }
      if underWay && standings[Self.relay] == nil {
        relays.append(Self.relayRow(Self.relay, standing: .loggingIn(nil)))
      }
      return Servers(remotes: remotes, relays: relays)
    }

    private func tellServers() {
      serversListeners.tell(currentServers())
    }

    private static func relayWay(_ standing: FakeRelayStanding?) -> WayRow {
      let address = "Relay \(relay)"
      return switch standing {
      case nil, .loggingIn:
        WayRow(
          address: address, state: .loginNeeded, words: "login needed",
          login: LoginOffer(relay: relay, words: "Log in"))
      case .loggedIn: WayRow(address: address, state: .answering, words: "answering", login: nil)
      case .otherAccount:
        WayRow(
          address: address, state: .otherAccount, words: "under another account",
          login: LoginOffer(relay: relay, words: "Log in again"))
      }
    }

    private static func relayRow(_ relay: String, standing: FakeRelayStanding) -> RelayRow {
      let shown = relay.replacingOccurrences(of: "https://", with: "")
      let (state, words, login): (RelayRowState, String, LoginOffer?) =
        switch standing {
        case .loggedIn: (.loggedIn, "logged in", nil)
        case .loggingIn: (.loggingIn, "logging in", nil)
        case .otherAccount:
          (.otherAccount, "under another account than a server", LoginOffer(relay: relay, words: "Log in again"))
        }
      return RelayRow(
        relay: relay, address: shown, account: standing.account, state: state, words: words, login: login,
        removal:
          "This phone gives up its login at \(shown). A server it reaches only through this relay reads unreachable "
          + "until this phone logs in there again.")
    }

    public func logIn(at relay: String) -> AsyncStream<DeviceLogin> {
      let shown = relay.replacingOccurrences(of: "https://", with: "")
      let title = "Log in at \(shown)"
      let id = nextLogin.add(1, ordering: .relaxed).oldValue
      return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
        continuation.yield(.starting(title: title, words: "Asking the relay at \(shown) for a code…"))
        let confirms = loginConfirms
        let task = Task { [weak self] in
          try? await Task.sleep(for: .milliseconds(400))
          guard !Task.isCancelled else { return }
          let address = "https://github.com/login/device"
          continuation.yield(
            .waiting(
              title: title, code: "WDJB-MJHT", address: address,
              words:
                "Paste this code at \(address), which opens here, or enter it there from another device. This "
                + "phone logs in at \(shown) as soon as the code is approved.",
              copy: "Copy code", open: "Open github.com"))
          try? await Task.sleep(for: confirms)
          guard !Task.isCancelled, let self else { return }
          self.loggingIn.withLock { _ = $0.removeValue(forKey: id) }
          self.loggedIn(at: relay)
          continuation.yield(
            .done(title: title, words: "This phone is logged in at \(shown) as octocat (github)."))
        }
        loggingIn.withLock { $0[id] = FakeLogin(title: title, continuation: continuation, task: task) }
        tellServers()
        continuation.onTermination = { [weak self] _ in
          task.cancel()
          guard let self else { return }
          if self.loggingIn.withLock({ $0.removeValue(forKey: id) }) != nil {
            self.tellServers()
          }
        }
      }
    }

    /// The fake's relay confirmed a login at `relay` as octocat: every Remote reached through it that was out of reach
    /// for want of it answers again.
    private func loggedIn(at relay: String) {
      relayStandings.withLock { $0[relay] = .loggedIn("octocat (github)") }
      let relayed = relayed.withLock { $0 }
      unreachable.withLock { unreachable in
        for name in relayed where unreachable[name] == .loginNeeded || unreachable[name] == .otherAccount {
          unreachable[name] = nil
        }
      }
      tellServers()
      tellWindows { window in
        window.tellListing()
        window.tellLanding()
      }
    }

    public func removeRelay(_ relay: String) async throws(ActError) {
      try? await Task.sleep(for: .milliseconds(200))
      guard relayStandings.withLock({ $0.removeValue(forKey: relay) }) != nil else {
        throw .Gone(message: "This phone no longer holds that relay")
      }
      tellServers()
    }

    public func renameDevice(_ name: String) async throws(ActError) {
      try? await Task.sleep(for: .milliseconds(200))
      let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
      if name.isEmpty {
        throw .Invalid(message: "A name needs something other than spaces")
      }
      if name.lowercased() == "everywhere" {
        throw .Invalid(message: "\"\(name)\" names every server at once, so nothing may be named so")
      }
      change { $0.name = name }
    }

    public func dismissPairAgain() async throws(ActError) {
      change { $0.pairAgain = nil }
    }

    public func pairingPrompt() -> String {
      "On the server, run /serve in Shaman and press q, or run `shaman serve invite --qr`, then scan the code it "
        + "draws or paste the invite here."
    }

    public func scanWords() -> ScanWords {
      ScanWords(
        looking: "Looking for a Shaman invite", passedOver: "That code is not a Shaman invite",
        cameraRefused: "Shaman may not use the camera. Allow it in the Settings app to scan, or paste the invite.",
        noCamera: "This phone has no camera that can scan. Paste the invite instead.",
        cameraFailed: "The camera stopped scanning. Paste the invite instead.")
    }

    public func isInvite(_ text: String) -> Bool {
      text.trimmingCharacters(in: .whitespacesAndNewlines).wholeMatch(of: /shaman-v[0-9]+-.*/) != nil
    }

    public func readInvite(_ invite: String) -> InviteReading {
      let invite = invite.trimmingCharacters(in: .whitespacesAndNewlines)
      guard invite.hasPrefix("shaman-") else {
        return .unreadable(message: "Invite must start with `shaman-`; was all of it copied?")
      }
      guard invite.hasPrefix("shaman-v1-") else {
        return .unreadable(message: "Invite is malformed")
      }
      let heldAs = held.withLock { $0.first { $0.fingerprint == Self.fingerprint }?.name }
      let loggedIn = relayStandings.withLock { $0[Self.relay] }.map { if case .loggedIn = $0 { true } else { false } }
      let relayWay = OfferedWay(
        address: "Relay \(Self.relay)", note: loggedIn == true ? nil : "login needed",
        login: loggedIn == true ? nil : LoginOffer(relay: Self.relay, words: "Log in"))
      let direct = [
        OfferedWay(address: "192.168.1.20:7791", note: nil, login: nil),
        OfferedWay(address: "100.64.0.7:7791", note: nil, login: nil),
        OfferedWay(address: "[fe80::1c2b:3a4d:5e6f:7a8b%7]:7791", note: "not dialled from a phone", login: nil),
      ]
      return .readable(
        invite: ReadInvite(
          hostname: "workstation", fingerprint: Self.fingerprint,
          ways: invite.contains("relayonly")
            ? [relayWay] : invite.contains("relay") ? direct + [relayWay] : direct,
          remoteName: heldAs ?? "workstation",
          held: heldAs.map {
            "This phone is already paired with this server as \($0). Pairing again replaces that pairing."
          },
          deviceName: device.withLock { $0.name }))
    }

    /// Pairs with the Server `invite` is from, as a window's `confirmPairing` asks, which turns that window toward it.
    func pair(invite: String, remoteName: String, deviceName: String) async -> PairingOutcome {
      try? await Task.sleep(for: .milliseconds(300))
      if invite.contains("dead") {
        return .failed(message: "Invite has expired", again: .scanAgain)
      }
      let loggedIn = relayStandings.withLock { $0[Self.relay] }.map { if case .loggedIn = $0 { true } else { false } }
      if invite.contains("relayonly") && loggedIn != true {
        return .loginNeeded(
          message:
            "No way this invite offers answered. It offers the relay at relay.example.com, where this phone needs to "
            + "log in: log in there and pairing goes on by itself, or try again once this phone can reach the server.",
          login: LoginOffer(relay: Self.relay, words: "Log in"), tryAgain: "Try again")
      }
      if invite.contains("unanswered") {
        return .failed(
          message:
            "No way this invite offers answered. Check that the server is serving and that this phone can reach it.",
          again: .tryAgain)
      }
      let deviceName = deviceName.trimmingCharacters(in: .whitespaces)
      if deviceName.isEmpty || deviceName.lowercased() == "everywhere" {
        return .deviceNameRefused(message: "\"\(deviceName)\" is not a name a server keeps")
      }
      let remoteName = remoteName.trimmingCharacters(in: .whitespaces)
      if remoteName.isEmpty {
        return .remoteNameRefused(
          message: "A remote's name must contain non-whitespace characters without outer whitespace")
      }
      held.withLock { held in
        held.removeAll { $0.fingerprint == Self.fingerprint }
        held.append(Self.workstation(named: remoteName))
      }
      relayed.withLock { $0 = invite.contains("relay") ? [remoteName] : [] }
      turn(remoteName)
      change { $0.name = deviceName }
      return .paired(name: remoteName)
    }

    /// Shows what a pairing changed in every window, once the window that paired has turned toward it.
    func pairedTurned() {
      tellServers()
      tellWindows { window in
        window.turnedOutlook()
        window.tellListing()
      }
    }

    /// The Remotes in the order last turned toward by any window, the latest first: a new window's Outlook stands at
    /// the first held.
    private let turned = Mutex<[String]>([])

    /// Turns the phone toward the Remote `remote`, first among those turned toward.
    private func turn(_ remote: String) {
      turned.withLock { turned in
        turned.removeAll { $0 == remote }
        turned.insert(remote, at: 0)
      }
    }

    /// Keeps a window's turn toward the Remote `remote`, at `workspace` where it stands in one, as the phone's latest.
    func keepTurn(_ remote: String, workspace: String?) {
      turn(remote)
      if let workspace {
        turnedWorkspace.withLock { $0[remote] = workspace }
      }
    }

    /// Where the phone was last turned, by any window: the Remotes in order and the Workspace on each.
    func lastTurned() -> ([String], [String: String]) {
      (turned.withLock { $0 }, turnedWorkspace.withLock { $0 })
    }

    /// What each Remote renamed is called now, by the name the fake holds it under.
    private let renamed = Mutex<[String: String]>([:])

    /// What the Remote the fake holds as `name` is called now.
    func shown(_ name: String) -> String {
      renamed.withLock { $0[name] } ?? name
    }

    public func renameRemote(_ remote: String, to name: String) async throws(ActError) {
      try? await Task.sleep(for: .milliseconds(200))
      let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
      let rows = held.withLock { $0 }
      guard let row = rows.first(where: { $0.remote == remote }) else {
        throw .Gone(message: "This phone is no longer paired with that server")
      }
      if name.isEmpty {
        throw .Invalid(message: "A remote's name must contain non-whitespace characters without outer whitespace")
      }
      if name.lowercased() == "everywhere" {
        throw .Invalid(
          message: "`everywhere` names every server at once, so no remote may be named so; choose another name")
      }
      if rows.contains(where: { $0.remote != remote && shown($0.name) == name }) {
        throw .Invalid(message: "This phone holds another server as \(name); choose another name")
      }
      renamed.withLock { $0[row.name] = name }
      tellServers()
      tellWindows { window in
        window.turnedOutlook()
        window.tellListing()
      }
    }

    public func removeRemote(_ remote: String) async throws(ActError) {
      try? await Task.sleep(for: .milliseconds(400))
      guard let row = held.withLock({ $0.first { $0.remote == remote } }) else {
        throw .Gone(message: "This phone is no longer paired with that server")
      }
      forget(row, removedThere: false)
    }

    /// Forgets the Remote `row`, as its Pairing ended, by this phone's user removing it or, where `removedThere`, by
    /// its own user removing this phone: each window's Outlook that stood there turns to the Remote the phone was most
    /// recently turned toward that is still held, saying so in that window alone; where no window's stood there, the
    /// first window says it was removed there, and where no window lives, the first to come says that alone.
    private func forget(_ row: ServerRow, removedThere: Bool) {
      let windows = windows.withLock { $0.compactMap(\.window) }
      let stoodThere = windows.map { $0.currentRemote() == row.name }
      let name = shown(row.name)
      held.withLock { $0.removeAll { $0.remote == row.remote } }
      turned.withLock { $0.removeAll { $0 == row.name } }
      _ = turnedWorkspace.withLock { $0.removeValue(forKey: row.name) }
      acts.forgetDrafts(on: "\(row.name)-key/")
      tellServers()
      let latest = turned.withLock { $0 }.first(where: heldNames().contains) ?? heldNames().first
      // Where no window lives, the first to come says why, and of no turn, which was not its own.
      if windows.isEmpty, removedThere {
        unclaimed.withLock { $0.append("\(name) removed this phone; pair with it again to reach it") }
      }
      for (window, stoodThere) in zip(windows, stoodThere) {
        window.forget(row.name, turningTo: stoodThere ? latest : nil)
        // A Session open on it is read no more, as the core's is.
        window.endSessions(on: row.name)
        window.turnedOutlook()
        window.tellListing()
        // A window whose Outlook stood elsewhere says nothing.
        guard stoodThere else { continue }
        var turnedTo: String?
        if case .remote(_, let beginsIn) = window.outlookNow() {
          turnedTo = beginsIn.said
        }
        let why: String? =
          removedThere
          ? "\(name) removed this phone; pair with it again to reach it" : turnedTo.map { _ in "\(name) was removed" }
        if let why {
          window.say(turnedTo.map { "\(why). \($0)" } ?? why)
        }
      }
      // Where no window's stood there, the first window says once that its Remote removed this phone.
      if removedThere, !stoodThere.contains(true), let first = windows.first {
        first.say("\(name) removed this phone; pair with it again to reach it")
      }
    }

    private func change(_ change: (inout ThisDevice) -> Void) {
      let changed = device.withLock { device in
        change(&device)
        return device
      }
      deviceListeners.tell(changed)
    }

    private static let fingerprint = "9c4e2a7b1d0f3865e2a9c17b4d6f0e3a8b5c2d9f1e7a4b6c3d0e9f8a7b6c5d4e"

    /// The fingerprint of the fake's Remote `name`.
    static func fingerprint(of name: String) -> String {
      switch name {
      case laptop.name: laptop.fingerprint
      case homelab.name: homelab.fingerprint
      default: fingerprint
      }
    }

    private static func workstation(named name: String) -> ServerRow {
      ServerRow(
        remote: fingerprint, name: name, fingerprint: fingerprint,
        ways: [
          WayRow(address: "192.168.1.20:7791", state: .answering, words: "answering", login: nil),
          WayRow(address: "100.64.0.7:7791", state: .notAnswering, words: "not answering", login: nil),
        ], removal: removal(of: name))
    }

    /// What removing the Remote `name` asks, as the core says it.
    private static func removal(of name: String) -> String {
      "This phone removes \(name), with what it was writing there, and asks \(name) to remove this phone too. Seeing "
        + "it again takes a new invite."
    }

    /// The scratch Server of `emptyServer`, which holds nothing.
    private static let scratch = ServerRow(
      remote: "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9", name: "scratch",
      fingerprint: "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9",
      ways: [WayRow(address: "192.168.1.50:7791", state: .answering, words: "answering", login: nil)],
      removal: removal(of: "scratch"))

    private static let laptop = ServerRow(
      remote: "5d1e8f0a2c4b6d8e0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9e2b4d6f8a0c1e3b5d", name: "laptop",
      fingerprint: "5d1e8f0a2c4b6d8e0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9e2b4d6f8a0c1e3b5d",
      ways: [WayRow(address: "100.84.12.7:7791", state: .answering, words: "answering", login: nil)],
      removal: removal(of: "laptop"))

    private static let homelab = ServerRow(
      remote: "e2b4d6f8a0c1e3b55d1e8f0a2c4b6d8e0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9a", name: "homelab",
      fingerprint: "e2b4d6f8a0c1e3b55d1e8f0a2c4b6d8e0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9a",
      ways: [WayRow(address: "192.168.1.40:7791", state: .answering, words: "answering", login: nil)],
      removal: removal(of: "homelab"))

    private static func device(in scenario: Scenario) -> ThisDevice {
      let note =
        "Servers paired from now on know this phone by this name. Those already paired keep the name they were given."
      let fingerprint = "3f9a1c07d24e8b56a0c3e19f7d5b2a48c6e0f1d39b7a25c84e6d0f2a1b3c5d7e"
      let pairAgain =
        "This phone has a new identity key, so no server it was paired with knows it any more. Pair with each of them again."
      let name = scenario == .fresh ? "iPhone" : "Pocket"
      return ThisDevice(
        name: name, nameNote: note, fingerprint: fingerprint, pairAgain: scenario == .pairAgain ? pairAgain : nil,
        protocolVersion: 116)
    }
  }

  /// A window the fake handed out, held without holding it up.
  private struct Held: Sendable {
    weak var window: FakeWindow?
  }

  /// How the fake's phone stands at a relay.
  enum FakeRelayStanding: Sendable {
    case loggedIn(String)
    case otherAccount(String)
    /// A login is under way there, where the phone stood logged in under the account given, if any, before.
    case loggingIn(String?)

    var account: String? {
      switch self {
      case .loggedIn(let account), .otherAccount(let account): account
      case .loggingIn(let account): account
      }
    }
  }

  /// A login the fake has under way.
  struct FakeLogin: Sendable {
    let title: String
    let continuation: AsyncStream<DeviceLogin>.Continuation
    let task: Task<Void, Never>
  }

  /// The streams a fake model is followed by, each told the model whole on every change.
  final class Listeners<Model: Sendable>: Sendable {
    private let continuations = Mutex<[Int: AsyncStream<Model>.Continuation]>([:])
    private let next = Atomic<Int>(0)

    /// A stream beginning with `now`.
    func stream(now: Model) -> AsyncStream<Model> {
      let id = next.add(1, ordering: .relaxed).oldValue
      return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
        continuation.yield(now)
        continuations.withLock { $0[id] = continuation }
        continuation.onTermination = { [weak self] _ in
          _ = self?.continuations.withLock { $0.removeValue(forKey: id) }
        }
      }
    }

    func tell(_ model: Model) {
      for continuation in continuations.withLock({ Array($0.values) }) {
        continuation.yield(model)
      }
    }
  }
#endif
