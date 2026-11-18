import ShamanCore

/// A listener the core tells of each change, as an `AsyncStream`: `subscribe` hands the core a listener that yields
/// each value it hears, and the stream ending, however it ends, cancels the subscription the core gave back.
/// Cancelling a Swift task does not reach the core; the cancellation is what does.
///
/// Each value is a whole model, so only the newest unread one is kept.
func listened<Value: Sendable>(
  _ subscribe: (_ send: @escaping @Sendable (Value) -> Void) -> any SubscriptionProtocol
) -> AsyncStream<Value> {
  AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
    let subscription = subscribe { value in continuation.yield(value) }
    continuation.onTermination = { _ in subscription.cancel() }
  }
}

/// Hands what the core says of "This device" to `send`.
final class DeviceForwarder: DeviceListener {
  private let send: @Sendable (ThisDevice) -> Void

  init(_ send: @escaping @Sendable (ThisDevice) -> Void) {
    self.send = send
  }

  func changed(device: ThisDevice) {
    send(device)
  }
}
