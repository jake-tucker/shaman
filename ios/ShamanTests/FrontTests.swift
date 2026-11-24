import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// What the core is told of the front as UIKit says the phases of the app's own scene, on a notification center of the
/// test's own, so the scene itself goes nowhere.
@Suite @MainActor struct FrontTests {
  @Test func whatUIKitSaysOfTheAppsSceneIsToldAsItSaysIt() throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    let center = NotificationCenter()
    var told: [Front.Change] = []
    let front = Front { told.append($0) }
    ScenePhases.follow(center) { front.scene($0, became: $1) }

    center.post(name: UIScene.willEnterForegroundNotification, object: scene)
    #expect(told == [])
    center.post(name: UIScene.didActivateNotification, object: scene)
    #expect(told == [.came])
    center.post(name: UIScene.willDeactivateNotification, object: scene)
    #expect(told == [.came])
    center.post(name: UIScene.didEnterBackgroundNotification, object: scene)
    #expect(told == [.came, .left], "before UIKit goes on")

    center.post(name: UIScene.willEnterForegroundNotification, object: scene)
    center.post(name: UIScene.didActivateNotification, object: scene)
    center.post(name: UIScene.didDisconnectNotification, object: scene)
    #expect(told == [.came, .left, .came, .left], "a scene that goes holds the front no longer")
  }

  @Test func theFollowersCoreIsToldTheAppCameToTheFrontAndLeftIt() throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    let center = NotificationCenter()
    let core = FakeCore(.paired)
    let follower = AppFollower(scenes: center)
    withExtendedLifetime(follower) {
      follower.follow(core)
      center.post(name: UIScene.didActivateNotification, object: scene)
      #expect(core.toldOfFront.withLock { $0 } == [true])
      center.post(name: UIScene.didEnterBackgroundNotification, object: scene)
      #expect(core.toldOfFront.withLock { $0 } == [true, false], "before UIKit goes on")
    }
  }

  @Test func aNotificationOfNoWindowSceneSaysNothing() {
    let center = NotificationCenter()
    var heard = 0
    ScenePhases.follow(center) { _, _ in heard += 1 }
    center.post(name: UIScene.didActivateNotification, object: NSObject())
    #expect(heard == 0)
  }
}
