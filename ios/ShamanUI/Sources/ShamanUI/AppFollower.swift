import ShamanCore
import UIKit

/// What is followed once for the app, and not once for each of its windows: whether it is in front, the iOS background
/// time held while an act is under way, and the network path. Made as the app launches, before it has a window, so it
/// hears of every scene from its first, and kept for as long as the app is, since all of it ends with the follower.
@MainActor
public final class AppFollower {
  private var core: (any DeviceCore)?
  private var following: [Task<Void, Never>] = []

  public convenience init() {
    self.init(scenes: .default)
  }

  init(scenes center: NotificationCenter) {
    let front = Front { [weak self] change in
      // The core is told when the app comes to the front, so it checks every Remote, and when it leaves, when it does
      // nothing to its connections.
      switch change {
      case .came: self?.core?.cameToFront()
      case .left: self?.core?.leftFront()
      }
    }
    ScenePhases.follow(center) { scene, phase in
      front.scene(scene, became: phase)
    }
  }

  deinit {
    for task in following {
      task.cancel()
    }
  }

  /// Follows for `core`, the app's one, from once it is open.
  public func follow(_ core: any DeviceCore) {
    // The application is there only once the app has launched, which is after this is asked and before a task runs.
    follow(core, tasks: UIApplication.shared)
  }

  func follow(_ core: any DeviceCore, tasks: @autoclosure @escaping @MainActor () -> any BackgroundTasks) {
    self.core = core
    following = [
      Task {
        // Held while an act is under way, whatever screen of whichever window asked it. Named, as the optimiser takes
        // what is not named for freed before iOS could take its time back, though it is not, and warns of it.
        let time = BackgroundTime(tasks: tasks())
        await time.follow(core.actsUnderWay())
      },
      Task {
        for await hasPath in NetworkPath.changes() {
          core.pathChanged(hasPath: hasPath)
        }
      },
    ]
  }
}
