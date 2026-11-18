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

    public init(_ scenario: Scenario) {
      device = Mutex(Self.device(in: scenario))
      let remotes: [ServerRow] =
        switch scenario {
        case .fresh, .named, .pairAgain: []
        case .paired: [Self.workstation(named: "workstation")]
        case .several: [Self.workstation(named: "workstation"), Self.laptop, Self.homelab]
        }
      held = Mutex(remotes)
      if scenario == .several {
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
        of: sessions.withLock { $0 }, on: remotes, showing: onShow.withLock { $0 }, now: Date())
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

    /// The Remote last paired, which the Outlook stands at.
    private let turned = Mutex<String?>(nil)

    private func currentOutlook() -> Outlook {
      let names = held.withLock { $0.map(\.name) }
      guard let name = turned.withLock({ $0 }).flatMap({ names.contains($0) ? $0 : nil }) ?? names.first else {
        return .nowhere
      }
      // With several Remotes the fake remembers a Workspace for the workstation, as the core will once one is chosen.
      let place = names.count > 1 && name == "workstation" ? "\(name) · shaman" : name
      return .remote(name: name, beginsIn: "New sessions begin in \(place)")
    }

    private func change(_ change: (inout ThisDevice) -> Void) {
      let changed = device.withLock { device in
        change(&device)
        return device
      }
      deviceListeners.tell(changed)
    }

    private static let fingerprint = "9c4e2a7b1d0f3865e2a9c17b4d6f0e3a8b5c2d9f1e7a4b6c3d0e9f8a7b6c5d4e"

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
  private final class Listeners<Model: Sendable>: Sendable {
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
