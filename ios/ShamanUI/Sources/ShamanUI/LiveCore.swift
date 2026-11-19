import Foundation
import ShamanCore

/// The thin Client's core itself, kept in a directory of the app's container.
public final class LiveCore: Core {
  private let client: ThinClient

  /// The core kept in `directory`, which `CoreDirectory.prepare` makes, proving this phone by the key `signer` signs
  /// with. Whatever the directory holds of another key is discarded as it opens, all but the name.
  public init(directory: URL, signer: any KeySigner) throws(OpenError) {
    do {
      client = try ThinClient.open(directory: directory.path(percentEncoded: false), signer: signer)
    } catch let error as OpenError {
      throw error
    } catch {
      throw .Failed(message: "This phone's records could not be read: \(error)")
    }
  }

  public func thisDevice() -> AsyncStream<ThisDevice> {
    listened { send in client.watchDevice(listener: Forwarder(send)) }
  }

  public func renameDevice(_ name: String) async throws(ActError) {
    try await acted { try await client.renameDevice(name: name) }
  }

  public func dismissPairAgain() async throws(ActError) {
    try await acted { try await client.dismissPairAgain() }
  }

  public func outlook() -> AsyncStream<Outlook> {
    listened { send in client.watchOutlook(listener: Forwarder(send)) }
  }

  public func servers() -> AsyncStream<Servers> {
    listened { send in client.watchServers(listener: Forwarder(send)) }
  }

  public func listing() -> AsyncStream<Listing> {
    listened { send in client.watchListing(listener: Forwarder(send)) }
  }

  public func settle(_ session: String) async throws(ActError) {
    try await acted { try await client.settle(session: session) }
  }

  public func unsettle(_ session: String) async throws(ActError) {
    try await acted { try await client.unsettle(session: session) }
  }

  public func showMoreSettled() {
    client.showMoreSettled()
  }

  public func openSession(_ session: String) -> any OpenedSession {
    LiveOpenedSession(client.openSession(session: session))
  }

  public func landing() -> AsyncStream<Landing> {
    listened { send in client.watchLanding(listener: Forwarder(send)) }
  }

  public func beginSession(_ text: String, newWorktree: Bool) throws(ActError) -> any OpenedSession {
    do {
      return LiveOpenedSession(try client.beginSession(text: text, newWorktree: newWorktree))
    } catch let error as ActError {
      throw error
    } catch {
      throw .Refused(message: "This phone could not do that: \(error)")
    }
  }

  public func workspaces() -> AsyncStream<WorkspacePicker> {
    listened { send in client.watchWorkspaces(listener: Forwarder(send)) }
  }

  public func searchWorkspaces(_ text: String) {
    client.searchWorkspaces(text: text)
  }

  public func chooseWorkspace(_ workspace: String) async throws(ActError) {
    try await acted { try await client.chooseWorkspace(workspace: workspace) }
  }

  public func narrowListing(_ scope: String) async throws(ActError) {
    try await acted { try await client.narrowListing(scope: scope) }
  }

  public func searchListing(_ text: String) {
    client.searchListing(text: text)
  }

  public func draft(for session: String) -> Draft {
    client.draft(session: session)
  }

  public func setDraft(_ text: String, version: UInt64, for session: String) {
    client.setDraft(session: session, text: text, version: version)
  }

  public func drafts(for session: String) -> AsyncStream<Draft> {
    listened { send in client.watchDraft(session: session, listener: Forwarder(send)) }
  }

  public func actsUnderWay() -> AsyncStream<Bool> {
    listened { send in client.watchActs(listener: Forwarder(send)) }
  }

  public func tryAgain(_ remote: String) async throws(ActError) {
    try await acted { try await client.tryAgain(remote: remote) }
  }

  public func cameToFront() {
    client.cameToFront()
  }

  public func leftFront() {
    client.leftFront()
  }

  public func pathChanged(hasPath: Bool) {
    client.pathChanged(hasPath: hasPath)
  }

  public func pairingPrompt() -> String {
    client.pairingPrompt()
  }

  public func readInvite(_ invite: String) -> InviteReading {
    client.readInvite(invite: invite)
  }

  public func confirmPairing(invite: String, remoteName: String, deviceName: String) async -> PairingOutcome {
    await client.confirmPairing(invite: invite, remoteName: remoteName, deviceName: deviceName)
  }
}

/// A Session the core holds open.
final class LiveOpenedSession: OpenedSession {
  private let session: OpenSession

  init(_ session: OpenSession) {
    self.session = session
  }

  var reference: String { session.reference() }

  func tryAgain() async throws(ActError) {
    try await acted { try await session.tryAgain() }
  }

  func frame() -> AsyncStream<SessionFrame> {
    listened { send in session.watchFrame(listener: Forwarder(send)) }
  }

  func conversation() -> AsyncStream<ConversationChange> {
    listened(everyChange: true) { send in session.watchConversation(listener: Forwarder(send)) }
  }

  func work(_ turn: String) -> AsyncStream<TurnWork> {
    listened { send in session.watchWork(turn: turn, listener: Forwarder(send)) }
  }

  func detail(_ activity: String) -> AsyncStream<ActivityDetail> {
    listened { send in session.watchDetail(activity: activity, listener: Forwarder(send)) }
  }

  func tree() -> AsyncStream<SubagentTree> {
    listened { send in session.watchTree(listener: Forwarder(send)) }
  }

  func stopSubagent(_ session: String) async throws(ActError) {
    try await acted { try await self.session.stopSubagent(subagent: session) }
  }

  func close() {
    session.close()
  }

  func send(_ text: String) async throws(ActError) {
    try await acted { try await session.send(text: text) }
  }

  func queue(_ text: String) async throws(ActError) {
    try await acted { try await session.queue(text: text) }
  }

  func steerNow(_ prompt: String) async throws(ActError) {
    try await acted { try await session.steerNow(prompt: prompt) }
  }

  func cancelQueued(_ prompt: String) async throws(ActError) {
    try await acted { try await session.cancelQueued(prompt: prompt) }
  }

  func stop() async throws(ActError) {
    try await acted { try await session.stop() }
  }

  func decide(_ approval: String, _ decision: DecisionKind) async throws(ActError) {
    try await acted { try await session.decide(approval: approval, decision: decision) }
  }

  func answer(_ questionnaire: String, _ answers: [GivenAnswer]) async throws(ActError) {
    try await acted { try await session.answer(questionnaire: questionnaire, answers: answers) }
  }

  func decline(_ questionnaire: String) async throws(ActError) {
    try await acted { try await session.decline(questionnaire: questionnaire) }
  }

  func dismiss() {
    session.dismiss()
  }

  func review() {
    session.review()
  }
}

/// Runs `act`, which fails as an `ActError` alone, so that is what it is said to throw.
private func acted(_ act: () async throws -> Void) async throws(ActError) {
  do {
    try await act()
  } catch let error as ActError {
    throw error
  } catch {
    throw .Refused(message: "This phone could not do that: \(error)")
  }
}

/// The live core over the key `IdentityKey` found, or why there is none.
public enum Opening {
  case core(LiveCore)
  /// The records could not be read or kept, or there is no key to open them under.
  case failed(String)
  /// The key cannot prove this phone: the chip will not take its blob, or it will not sign. Starting again under a
  /// new key is the user's to choose.
  case keyRefused(String)

  /// The core kept in `directory`, over the key `found`.
  public static func of(_ found: IdentityKey.Found, in directory: URL) -> Opening {
    switch found {
    case .ready(let signer): over(signer, in: directory)
    case .failed(let message): .failed(message)
    case .refused(let message): .keyRefused(message)
    }
  }

  /// The core kept in `directory`, over the key `signer` signs with.
  static func over(_ signer: any KeySigner, in directory: URL) -> Opening {
    do {
      return .core(try LiveCore(directory: directory, signer: signer))
    } catch .KeyRefused(let message) {
      return .keyRefused(message)
    } catch {
      return .failed(error.message)
    }
  }
}
