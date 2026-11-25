import ShamanCore
import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// The conversation's collection view in a window of the app's own scene, so that its display link ticks as it does
/// on the phone: what UIKit animates over frames, a scroll toward a foot the rows shrank to or one the app asked for,
/// runs here as it would there. Each test waits on the frames rather than laying the view out by hand.
@MainActor private final class HostedList {
  let window: UIWindow
  let view: FootCollectionView
  let follower: ConversationFollower

  /// `count` rows each first measuring `height`, standing at the foot. What a row measures after that is what the test
  /// reports of it.
  convenience init(rows count: Int, height: CGFloat) async throws {
    try await self.init(rows: count) { _, _ in
      UIHostingConfiguration { Color.clear.frame(height: height) }.margins(.all, 0)
    }
  }

  /// `count` rows of words of uneven length, each saying its height whenever it changes, as the conversation's rows
  /// do, standing at the foot.
  convenience init(wordRows count: Int) async throws {
    try await self.init(rows: count) { row, follower in
      UIHostingConfiguration {
        Text(String(repeating: "\(row.id) words ", count: (row.id.count * 7 + row.id.hashValue & 7) % 20 + 2))
          .fixedSize(horizontal: false, vertical: true)
          .onGeometryChange(for: CGFloat.self) {
            ceil($0.size.height)
          } action: { [weak follower] height in
            follower?.report(row.id, height)
          }
          .frame(maxHeight: .infinity, alignment: .top)
      }
      .margins(.horizontal, 14)
    }
  }

  private init(
    rows count: Int, configuration: @escaping (ConversationRow, ConversationFollower) -> any UIContentConfiguration
  ) async throws {
    let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    window = UIWindow(windowScene: scene)
    let follower = ConversationFollower()
    self.follower = follower
    follower.configuration = { [unowned follower] row in configuration(row, follower) }
    view = follower.makeView()
    let controller = UIViewController()
    controller.view = view
    window.rootViewController = controller
    window.windowLevel = .alert + 1
    window.makeKeyAndVisible()
    follower.take((0..<count).map { .item(ConversationItem(id: "\($0)", kind: .error(text: "row \($0)"))) })
    try await frames(30)
  }

  /// Lets `count` frames go by, or about as long.
  func frames(_ count: Int) async throws {
    try await Task.sleep(for: .milliseconds(count * 17))
  }

  /// How far above its foot the list stood at each frame over `duration`.
  func distances(over duration: Duration) async -> [CGFloat] {
    var seen: [CGFloat] = []
    let end = ContinuousClock.now + duration
    while ContinuousClock.now < end {
      seen.append(view.distanceToFoot)
      try? await Task.sleep(for: .milliseconds(8))
    }
    return seen
  }

  func close() {
    window.isHidden = true
  }
}

@Suite(.serialized) @MainActor struct ConversationListHostedTests {
  /// Rows above the foot measuring far shorter all at once, as a long Session's answers do when the text grows
  /// smaller or Textual first draws a run of short ones: the system scrolls toward the new foot over a few frames,
  /// and the list must neither ride that scroll up nor take it for the reader leaving.
  @Test func rowsShrinkingUnderTheFootLeaveItAtTheFootStillFollowing() async throws {
    let list = try await HostedList(rows: 80, height: 90)
    defer { list.close() }
    #expect(list.follower.following)
    #expect(list.view.distanceToFoot < 1)
    for index in 0..<80 {
      list.follower.report("item:\(index)", 20)
    }
    let seen = await list.distances(over: .seconds(1.5))
    #expect(list.follower.following, "still following the foot")
    #expect(list.view.distanceToFoot < 1, "at the foot of what is now 5,600 points shorter")
    #expect((seen.max() ?? 0) < 1, "never above it on any frame: \(seen.map { Int($0) })")
  }

  /// The text made far larger and then as it was: the rows on screen grow, then measure thousands of points shorter
  /// together, and the system scrolls toward the new foot over a few frames, as on the phone when its text size is
  /// changed with a long Session open.
  @Test func theTextMadeLargerAndBackLeavesTheListAtItsFootStillFollowing() async throws {
    let list = try await HostedList(wordRows: 120)
    defer { list.close() }
    #expect(list.view.distanceToFoot < 1)
    list.window.traitOverrides.preferredContentSizeCategory = .accessibilityExtraExtraExtraLarge
    let larger = await list.distances(over: .seconds(1.5))
    #expect(list.follower.following, "following as the text grew")
    #expect(list.view.distanceToFoot < 1, "at the foot of the larger text: \(larger.map { Int($0) })")
    list.window.traitOverrides.preferredContentSizeCategory = .large
    let back = await list.distances(over: .seconds(1.5))
    #expect(list.follower.following, "still following once the text came back")
    #expect(list.view.distanceToFoot < 1, "at the foot again: \(back.map { Int($0) })")
  }

  @Test func rowsGrowingUnderTheFootKeepItThere() async throws {
    let list = try await HostedList(rows: 40, height: 40)
    defer { list.close() }
    for index in 0..<40 {
      list.follower.report("item:\(index)", 120)
    }
    try await list.frames(30)
    #expect(list.follower.following)
    #expect(list.view.distanceToFoot < 1)
  }

  @Test(arguments: [false, true])
  func aScrollToAnOffsetAboveTheFootLeavesItAndStays(animated: Bool) async throws {
    let list = try await HostedList(rows: 60, height: 80)
    defer { list.close() }
    let view = list.view
    view.setContentOffset(CGPoint(x: 0, y: view.contentOffset.y - 500), animated: animated)
    try await list.frames(60)
    #expect(!list.follower.following, "the app or the system scrolled 500 points up")
    #expect(abs(view.distanceToFoot - 500) < 2, "and the list stands where it was taken: \(view.distanceToFoot)")
  }

  @Test(arguments: [false, true])
  func aRectAboveBroughtIntoSightLeavesTheFootAndStays(animated: Bool) async throws {
    let list = try await HostedList(rows: 60, height: 80)
    defer { list.close() }
    let view = list.view
    let top = view.contentOffset.y + view.adjustedContentInset.top
    view.scrollRectToVisible(CGRect(x: 0, y: top - 300, width: 10, height: 10), animated: animated)
    try await list.frames(60)
    #expect(!list.follower.following, "the keyboard's or the focus's rect, 300 points above")
    #expect(view.distanceToFoot > 250, "and the list stands where it was taken: \(view.distanceToFoot)")
  }

  @Test(arguments: [false, true])
  func aRowAboveScrolledToLeavesTheFootAndStays(animated: Bool) async throws {
    let list = try await HostedList(rows: 60, height: 80)
    defer { list.close() }
    list.view.scrollToItem(at: IndexPath(item: 10, section: 0), at: .top, animated: animated)
    try await list.frames(60)
    #expect(!list.follower.following)
    #expect(list.view.distanceToFoot > 2000, "\(list.view.distanceToFoot)")
  }

  /// A scroll toward a point near the foot, under way as the last row grows: the list goes to the new foot itself,
  /// which stops that scroll, and UIKit says the scroll ended before the list stands there. That is no reader coming to
  /// rest above the foot.
  @Test func aScrollNearTheFootStoppedByARowGrowingKeepsFollowing() async throws {
    let list = try await HostedList(rows: 60, height: 80)
    defer { list.close() }
    let view = list.view
    view.setContentOffset(CGPoint(x: 0, y: view.contentOffset.y - 30), animated: true)
    list.follower.report("item:59", 80 + 310)
    try await list.frames(60)
    #expect(list.follower.following, "still following the foot")
    #expect(view.distanceToFoot < 1, "at the foot of the grown row: \(view.distanceToFoot)")
  }

  @Test func aScrollToTheLastRowWhileItGrowsKeepsFollowing() async throws {
    let list = try await HostedList(rows: 60, height: 80)
    defer { list.close() }
    let view = list.view
    view.setContentOffset(CGPoint(x: 0, y: view.contentOffset.y - 20), animated: false)
    view.scrollToItem(at: IndexPath(item: 59, section: 0), at: .bottom, animated: true)
    list.follower.report("item:59", 80 + 310)
    try await list.frames(60)
    #expect(list.follower.following)
    #expect(view.distanceToFoot < 1, "\(view.distanceToFoot)")
  }

  /// The room the list has changing, as the composer growing or the keyboard rising changes it, with nothing else
  /// laying the list out: it follows its foot there.
  @Test func anInsetChangingAloneIsFollowed() async throws {
    let list = try await HostedList(rows: 60, height: 80)
    defer { list.close() }
    let view = list.view
    view.contentInset.bottom += 200
    try await list.frames(30)
    #expect(list.follower.following)
    #expect(view.distanceToFoot < 1, "at the foot of the smaller room: \(view.distanceToFoot)")
    view.contentInset.bottom -= 200
    try await list.frames(30)
    #expect(view.distanceToFoot < 1, "and back: \(view.distanceToFoot)")
  }

  @Test func aScrollToTheFootItselfKeepsFollowing() async throws {
    let list = try await HostedList(rows: 60, height: 80)
    defer { list.close() }
    list.view.scrollToItem(at: IndexPath(item: 59, section: 0), at: .bottom, animated: true)
    try await list.frames(60)
    #expect(list.follower.following)
    #expect(list.view.distanceToFoot < 1)
  }

  /// A change of width, animated as a column coming or going beside the list animates it, in which the last row leaves
  /// the screen and comes back. UIKit passes that one over from then on, where it is asked to reconfigure it.
  @Test(arguments: [false, true])
  func aRowThatChangesAfterAChangeOfWidthIsDrawnAgain(animated: Bool) async throws {
    let list = try await HostedList(wordRows: 13)
    defer { list.close() }
    let view = list.view
    UIView.animate(withDuration: animated ? 0.35 : 0) {
      view.frame.size.width -= 60
      view.layoutIfNeeded()
    }
    try await list.frames(60)
    let configuration = list.follower.configuration
    var drawn: [String] = []
    list.follower.configuration = { row in
      drawn.append(row.id)
      return configuration?(row) ?? UIListContentConfiguration.cell()
    }
    list.follower.take(
      (0..<13).map { .item(ConversationItem(id: "\($0)", kind: .error(text: "row \($0 == 12 ? "changed" : "\($0)")"))) }
    )
    try await list.frames(10)
    #expect(drawn == ["item:12"], "the one that changed, on screen at the foot")
  }
}
