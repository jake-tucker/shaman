import SwiftUI
import UIKit

/// Whether the app is in front, which is the app's and no window's own: it leaves the front as the last of its scenes
/// on screen goes to the background, and comes to it as one becomes active while none was.
@MainActor
final class Front {
  enum Change {
    /// A scene became active while none was: the first back, or one alone returning from beneath what the system
    /// showed over it.
    case came
    /// No scene is in the foreground any longer.
    case left
  }

  /// The phase each scene was last in, of those that have had one and have not gone.
  private var phases: [ObjectIdentifier: ScenePhase] = [:]
  private let tell: (Change) -> Void

  init(tell: @escaping (Change) -> Void) {
    self.tell = tell
  }

  /// Hears the phase `scene` is now in, or that it has gone where there is none, and tells what that changes.
  func scene(_ scene: ObjectIdentifier, became phase: ScenePhase?) {
    let wasActive = isActive
    let wasInForeground = isInForeground
    phases[scene] = phase
    if isActive, !wasActive {
      tell(.came)
    }
    if wasInForeground, !isInForeground {
      tell(.left)
    }
  }

  private var isActive: Bool {
    phases.values.contains(.active)
  }

  private var isInForeground: Bool {
    phases.values.contains { $0 != .background }
  }
}

/// The phases of the app's scenes, as UIKit says them. A view's own `scenePhase` says nothing as its window goes, so
/// the app hears them here and in no window.
@MainActor
enum ScenePhases {
  /// Tells `heard` each phase a window of the app enters, and nil as it goes, for as long as `center` is.
  static func follow(
    _ center: NotificationCenter, _ heard: @escaping @MainActor (ObjectIdentifier, ScenePhase?) -> Void
  ) {
    let said: [(Notification.Name, ScenePhase?)] = [
      (UIScene.willEnterForegroundNotification, .inactive),
      (UIScene.didActivateNotification, .active),
      (UIScene.willDeactivateNotification, .inactive),
      (UIScene.didEnterBackgroundNotification, .background),
      (UIScene.didDisconnectNotification, nil),
    ]
    for (name, phase) in said {
      // On no queue, so it is heard before UIKit goes on, and what the core does as the app leaves the front is done.
      _ = center.addObserver(forName: name, object: nil, queue: nil) { notification in
        guard let scene = notification.object as? UIWindowScene else { return }
        MainActor.assumeIsolated {
          guard scene.session.role == .windowApplication else { return }
          heard(ObjectIdentifier(scene), phase)
        }
      }
    }
  }
}
