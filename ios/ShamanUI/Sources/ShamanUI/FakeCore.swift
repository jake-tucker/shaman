#if DEBUG
  import Foundation
  import ShamanCore
  import Synchronization

  /// A stand-in for the core, holding canned values of the types it hands Swift, for previews, UI tests and looking
  /// at a screen without a Server. Debug builds alone carry it; a launch argument chooses it and its scenario.
  public final class FakeCore: Core {
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
    }

    /// The Invite the fake reads as the workstation's. One holding "dead" reads as the workstation's too, and fails to
    /// pair as an expired one does; one holding "unanswered" fails as one no way answered does.
    public static let invite = "shaman-v1-fake-workstation"

    private let device: Mutex<ThisDevice>
    private let held: Mutex<[ServerRow]>
    private let deviceListeners = Listeners<ThisDevice>()
    private let outlookListeners = Listeners<Outlook>()
    private let serversListeners = Listeners<Servers>()
    private let listingListeners = Listeners<Listing>()
    /// Every Session the fake's Remotes hold, the most recently begun first.
    private let sessions = Mutex(FakeSession.canned)
    /// How many of the shelf's rows are on show.
    private let onShow = Mutex(FakeSession.shelfOpening)
    /// The Remotes held that have yet to answer with their Sessions, which they never do.
    private let unanswered: [String]
    /// What the listing is narrowed to and searched for, until the fake goes.
    private let scope = Mutex(FakeScope.everywhere)
    private let query = Mutex("")
    /// The Remotes held that read Unreachable, and why.
    private let unreachable: Mutex<[String: UnreachableHint]>
    /// How long trying a Remote again takes to answer.
    private let tryingTakes = Duration.milliseconds(400)

    /// Each composer's draft, and whether an act is under way.
    private let acts = FakeActs()

    public init(_ scenario: Scenario) {
      device = Mutex(Self.device(in: scenario))
      let remotes: [ServerRow] =
        switch scenario {
        case .fresh, .named, .pairAgain: []
        case .paired, .offline: [Self.workstation(named: "workstation")]
        case .several, .reaching, .unreachable: [Self.workstation(named: "workstation"), Self.laptop, Self.homelab]
        }
      held = Mutex(remotes)
      unanswered = scenario == .reaching ? ["homelab"] : []
      let unreachable: [String: UnreachableHint] =
        switch scenario {
        case .unreachable: ["homelab": .localNetwork]
        case .offline: ["workstation": .noNetwork]
        default: [:]
        }
      self.unreachable = Mutex(unreachable)
      if scenario == .several || scenario == .reaching || scenario == .unreachable {
        turned.withLock { $0 = "workstation" }
      }
    }

    public func listing() -> AsyncStream<Listing> {
      listingListeners.stream(now: currentListing())
    }

    public func settle(_ session: String) async throws(ActError) {
      try await settling(session, to: true)
    }

    public func unsettle(_ session: String) async throws(ActError) {
      try await settling(session, to: false)
    }

    public func showMoreSettled() {
      onShow.withLock { $0 += FakeSession.shelfBatch }
      listingListeners.tell(currentListing())
    }

    /// Every Session the fake has opened, the latest last, for a test to see each closed as its screen goes.
    let opened = Mutex<[FakeOpenedSession]>([])

    public func openSession(_ session: String) -> any OpenedSession {
      let remotes = held.withLock { $0.map(\.name) }
      let found = sessions.withLock { $0.first { $0.id == session && remotes.contains($0.remote) } }
      let cut = found.map { found in unreachable.withLock { $0[found.remote] != nil } } ?? false
      let frame =
        found?.frame(originNamed: remotes.count > 1, unreachable: cut, now: Date())
        ?? SessionFrame(
          title: "", place: "", gone: "This session no longer exists", bar: nil, composer: nil, queued: [],
          intervention: nil)
      if let found {
        turned.withLock { $0 = found.remote }
        // The Workspace remembered for the Remote stays; only where none is does the Session's own become it.
        turnedWorkspace.withLock { $0[found.remote] = $0[found.remote] ?? found.workspace }
        outlookListeners.tell(currentOutlook())
      }
      // A Session on a Remote yet to answer is never read: its loading mark stands.
      let read = found.flatMap { unanswered.contains($0.remote) ? nil : $0.reads }
      let opened = FakeOpenedSession(session: session, frame: frame, scenario: read, unreachable: cut, acts: acts)
      self.opened.withLock { $0.append(opened) }
      return opened
    }

    public func draft(for session: String) -> Draft {
      acts.draft(for: session)
    }

    public func setDraft(_ text: String, version: UInt64, for session: String) {
      acts.setDraft(text, version: version, for: session)
    }

    public func drafts(for session: String) -> AsyncStream<Draft> {
      acts.drafts(for: session)
    }

    public func actsUnderWay() -> AsyncStream<Bool> {
      acts.stream()
    }

    /// Gives `text` back to the composer of `session` as the core does after a failed send, for a test to see what
    /// the field makes of it.
    static func giveBack(_ text: String, in core: FakeCore, to session: String) {
      core.acts.giveBack(text, to: session)
    }

    /// The Workspace last turned toward on each Remote.
    private let turnedWorkspace = Mutex<[String: String]>([:])

    public func narrowListing(_ reference: String) async throws(ActError) {
      let remotes = held.withLock { $0.map(\.name) }
      let chosen = FakeScope(reference).within(remotes, sessions: sessions.withLock { $0 })
      scope.withLock { $0 = chosen }
      if let remote = chosen.remote {
        turned.withLock { $0 = remote }
        if case .workspace(_, let workspace) = chosen {
          turnedWorkspace.withLock { $0[remote] = workspace }
        }
        outlookListeners.tell(currentOutlook())
      }
      listingListeners.tell(currentListing())
    }

    public func searchListing(_ text: String) {
      query.withLock { $0 = text }
      listingListeners.tell(currentListing())
    }

    /// Every Remote read again answers but where the phone has no network, after a moment.
    public func tryAgain(_ remote: String) async throws(ActError) {
      try? await Task.sleep(for: tryingTakes)
      let names = held.withLock { $0.filter { $0.fingerprint == remote }.map(\.name) }
      guard let name = names.first else {
        throw .Gone(message: "This phone is no longer paired with that server")
      }
      let stillUnreachable = unreachable.withLock { unreachable in
        if unreachable[name] == .noNetwork { return true }
        unreachable[name] = nil
        return false
      }
      if stillUnreachable {
        throw .Unreachable(message: "This phone has no network connection")
      }
      listingListeners.tell(currentListing())
    }

    public func cameToFront() {}

    public func leftFront() {}

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
      listingListeners.tell(currentListing())
    }

    /// The listing of every held Remote's Sessions, as the core would make it.
    private func currentListing() -> Listing {
      let remotes = held.withLock { $0.map(\.name) }
      return FakeSession.listing(
        of: sessions.withLock { $0 }, on: remotes, unanswered: unanswered,
        unreachable: unreachable.withLock { $0 }, showing: onShow.withLock { $0 },
        scope: scope.withLock { $0 }, query: query.withLock { $0 }, now: Date())
    }

    public func thisDevice() -> AsyncStream<ThisDevice> {
      deviceListeners.stream(now: device.withLock { $0 })
    }

    public func outlook() -> AsyncStream<Outlook> {
      outlookListeners.stream(now: currentOutlook())
    }

    public func servers() -> AsyncStream<Servers> {
      serversListeners.stream(now: Servers(remotes: held.withLock { $0 }))
    }

    public func renameDevice(_ name: String) async throws(ActError) {
      change { $0.name = name }
    }

    public func dismissPairAgain() async throws(ActError) {
      change { $0.pairAgain = nil }
    }

    public func pairingPrompt() -> String {
      "On the server, run /serve in Shaman or `shaman serve invite`, copy the invite, and paste it here."
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
      return .readable(
        invite: ReadInvite(
          hostname: "workstation", fingerprint: Self.fingerprint,
          ways: [
            OfferedWay(address: "192.168.1.20:7791", note: nil),
            OfferedWay(address: "100.64.0.7:7791", note: nil),
            OfferedWay(address: "[fe80::1c2b:3a4d:5e6f:7a8b%7]:7791", note: "not dialled from a phone"),
          ],
          remoteName: heldAs ?? "workstation",
          held: heldAs.map {
            "This phone is already paired with this server as \($0). Pairing again replaces that pairing."
          },
          deviceName: device.withLock { $0.name }))
    }

    public func confirmPairing(invite: String, remoteName: String, deviceName: String) async -> PairingOutcome {
      try? await Task.sleep(for: .milliseconds(300))
      if invite.contains("dead") {
        return .failed(message: "Invite has expired", again: .scanAgain)
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
      let remotes = held.withLock { held in
        held.removeAll { $0.fingerprint == Self.fingerprint }
        held.append(Self.workstation(named: remoteName))
        return held
      }
      turned.withLock { $0 = remoteName }
      change { $0.name = deviceName }
      serversListeners.tell(Servers(remotes: remotes))
      outlookListeners.tell(currentOutlook())
      listingListeners.tell(currentListing())
      return .paired(name: remoteName)
    }

    /// The Remote last turned toward, which the Outlook stands at.
    private let turned = Mutex<String?>(nil)

    private func currentOutlook() -> Outlook {
      let names = held.withLock { $0.map(\.name) }
      guard let name = turned.withLock({ $0 }).flatMap({ names.contains($0) ? $0 : nil }) ?? names.first else {
        return .nowhere
      }
      let place = turnedWorkspace.withLock { $0[name] }.map { "\(name) · \($0)" } ?? name
      return .remote(
        name: name,
        beginsIn: BeginsIn(lead: "New sessions begin in", place: place, said: "New sessions begin in \(place)"))
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
        name: name, fingerprint: fingerprint,
        ways: [
          WayRow(address: "192.168.1.20:7791", state: .answering, words: "answering"),
          WayRow(address: "100.64.0.7:7791", state: .notAnswering, words: "not answering"),
        ])
    }

    private static let laptop = ServerRow(
      name: "laptop", fingerprint: "5d1e8f0a2c4b6d8e0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9e2b4d6f8a0c1e3b5d",
      ways: [WayRow(address: "100.84.12.7:7791", state: .answering, words: "answering")])

    private static let homelab = ServerRow(
      name: "homelab", fingerprint: "e2b4d6f8a0c1e3b55d1e8f0a2c4b6d8e0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9a",
      ways: [WayRow(address: "192.168.1.40:7791", state: .answering, words: "answering")])

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
