import ShamanCore
import UIKit

/// What holds iOS background time: begun as an act goes under way, ended once none is, so an act asked just before the
/// app leaves the front is answered before iOS suspends it. `UIApplication` is the live one; a test hands in its own.
@MainActor
protocol BackgroundTasks: AnyObject {
  /// Begins holding background time, calling `expired` should iOS take it back first.
  func begin(expired: @escaping @MainActor @Sendable () -> Void) -> UIBackgroundTaskIdentifier

  func end(_ task: UIBackgroundTaskIdentifier)
}

extension UIApplication: BackgroundTasks {
  func begin(expired: @escaping @MainActor @Sendable () -> Void) -> UIBackgroundTaskIdentifier {
    beginBackgroundTask(withName: "An act under way") {
      MainActor.assumeIsolated { expired() }
    }
  }

  func end(_ task: UIBackgroundTaskIdentifier) {
    endBackgroundTask(task)
  }
}

/// Holds iOS background time only while the core says an act is under way, and never twice.
@MainActor
final class BackgroundTime {
  private let tasks: any BackgroundTasks
  private var held: UIBackgroundTaskIdentifier?

  init(tasks: any BackgroundTasks) {
    self.tasks = tasks
  }

  /// Follows whether an act is under way until `underWay` ends, letting go of any time still held then.
  func follow(_ underWay: AsyncStream<Bool>) async {
    for await busy in underWay {
      busy ? hold() : release()
    }
    release()
  }

  private func hold() {
    guard held == nil else { return }
    held = tasks.begin { [weak self] in self?.release() }
  }

  private func release() {
    guard let task = held else { return }
    held = nil
    tasks.end(task)
  }
}
