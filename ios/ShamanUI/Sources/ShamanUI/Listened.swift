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

/// Hands each model the core tells a listener of to `send`.
final class Forwarder<Model: Sendable>: Sendable {
  private let send: @Sendable (Model) -> Void

  init(_ send: @escaping @Sendable (Model) -> Void) {
    self.send = send
  }
}

extension Forwarder: DeviceListener where Model == ThisDevice {
  func changed(device: ThisDevice) {
    send(device)
  }
}

extension Forwarder: ServersListener where Model == Servers {
  func changed(servers: Servers) {
    send(servers)
  }
}

extension Forwarder: OutlookListener where Model == Outlook {
  func changed(outlook: Outlook) {
    send(outlook)
  }
}
