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
    case .Failed(let message): message
    }
  }
}
