import ShamanCore

/// Everything the app asks of the thin Client's core: the one way it reaches it. `LiveCore` is the core itself; in
/// debug builds `FakeCore` stands in with canned values, chosen by a launch argument.
///
/// What a screen shows arrives as a finished model, whole on every change, beginning with what it is now. An act
/// returns once it is done, with nothing but success; what it changed arrives by the model.
public protocol Core: AnyObject, Sendable {
  /// What "This device" shows, now and on every change.
  func thisDevice() -> AsyncStream<ThisDevice>

  /// Renames this device, or fails with why, in the core's words.
  func renameDevice(_ name: String) async throws(ActError)

  /// Says no more that this phone must pair again.
  func dismissPairAgain() async throws(ActError)

  /// Where the Outlook stands, now and on every change: nowhere while the phone holds no Remote.
  func outlook() -> AsyncStream<Outlook>

  /// The Servers section of the sheet of Servers, Relays and This device, now and on every change.
  func servers() -> AsyncStream<Servers>

  /// Reads `invite` for the confirmation screen, dialling nothing.
  func readInvite(_ invite: String) -> InviteReading

  /// Pairs with the Server `invite` is from, naming it `remoteName` and this phone `deviceName`, replacing any Pairing
  /// with that Server, and turns the Outlook toward it. It keeps dialling for a few seconds while no way answers, and
  /// keeps nothing where it fails.
  func confirmPairing(invite: String, remoteName: String, deviceName: String) async -> PairingOutcome
}

extension ActError {
  /// The words the core wrote for the failure.
  public var message: String {
    switch self {
    case .Unreachable(let message), .Refused(let message), .Gone(let message), .Invalid(let message):
      message
    }
  }
}

extension OpenError {
  /// The words the core wrote for the failure.
  public var message: String {
    switch self {
    case .Failed(let message), .KeyRefused(let message): message
    }
  }
}
