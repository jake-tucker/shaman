import Foundation
import Network

/// Whether the phone has a network path at all, as `NWPathMonitor` says: the core stops dialling without one, and
/// checks every Remote each time the path changes.
enum NetworkPath {
  /// Whether there is a path now, and again each time the path changes, until the stream ends.
  static func changes() -> AsyncStream<Bool> {
    AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
      let monitor = NWPathMonitor()
      monitor.pathUpdateHandler = { path in
        continuation.yield(path.status == .satisfied)
      }
      continuation.onTermination = { _ in monitor.cancel() }
      monitor.start(queue: DispatchQueue(label: "com.jaketucker.shaman.network-path"))
    }
  }
}
