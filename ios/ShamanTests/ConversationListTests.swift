import ShamanCore
import SwiftUI
import Testing
import UIKit

@testable import ShamanUI

/// A row, and how far above the top of what the reader sees its head stands.
private struct Standing: Equatable {
  let row: String
  let above: CGFloat

  /// To what arithmetic makes of a point.
  static func == (one: Standing, other: Standing) -> Bool {
    one.row == other.row && abs(one.above - other.above) < 0.001
  }
}

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
        Text(Self.words(of: row))
          .fixedSize(horizontal: false, vertical: true)
          .onGeometryChange(for: CGFloat.self) {
            ceil($0.size.height)
          } action: { [weak follower] height in
            follower?.report(row.id, height)
          }
          .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
          .padding(.horizontal, 6)
      }
      .margins(.all, 0)
    }
  }

  /// From two words to thirty-odd, and twice as many where the row says it has grown.
  private static func words(of row: ConversationRow) -> String {
    guard case .item(let item) = row, case .error(let text) = item.kind else { return "" }
    let count = (Int(item.id) ?? 0) * 7 % 17 + 1
    return String(repeating: "\(row.id) words ", count: text.hasSuffix("grown") ? count * 2 + 10 : count)
  }

  init(
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
    follower.take(rows(count))
    try await frames(30)
  }

  /// The first `count` rows, with the one `grown` names grown.
  func rows(_ count: Int, grown: String? = nil) -> [ConversationRow] {
    (0..<count).map {
      .item(ConversationItem(id: "\($0)", kind: .error(text: "row \($0)" + (grown == "item:\($0)" ? " grown" : ""))))
    }
  }

  /// The reader drags to `offset`, up or down, and lets go there, and what came into sight settles.
  func drag(to offset: CGFloat) async throws {
    follower.scrollViewWillBeginDragging(view)
    view.contentOffset.y = offset
    follower.scrollViewDidEndDragging(view, willDecelerate: false)
    try await frames(60)
  }

  /// Stands the list across `width`, as a turn or a column coming or going does.
  func stand(across width: CGFloat, height: CGFloat? = nil, animated: Bool = false) async throws {
    let view = view
    UIView.animate(withDuration: animated ? 0.2 : 0) {
      view.frame.size = CGSize(width: width, height: height ?? view.frame.height)
      view.layoutIfNeeded()
    }
    try await frames(30)
  }

  /// Where the row at the top of what the reader sees stands on the screen.
  var onScreen: Standing? {
    atTheTop.flatMap { place in
      follower.geometry.depth(of: ConversationPlace(row: place.row, share: 0, above: 0)).map {
        Standing(row: place.row, above: view.visibleHead - $0)
      }
    }
  }

  /// What stands at the top of what the reader sees.
  var atTheTop: ConversationPlace? {
    follower.geometry.place(at: view.visibleHead)
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

  /// At the top of what the reader sees stands what did, to the nearest line.
  private func stands(
    _ list: HostedList, at place: ConversationPlace?, _ said: Comment, sourceLocation: SourceLocation = #_sourceLocation
  ) throws {
    let place = try #require(place, sourceLocation: sourceLocation)
    let now = try #require(list.atTheTop, sourceLocation: sourceLocation)
    let line = UIFont.preferredFont(forTextStyle: .body).lineHeight
    #expect(now.row == place.row, said, sourceLocation: sourceLocation)
    let height = list.follower.geometry.height(of: now.row)
    #expect(abs(now.share - place.share) * height < line, said, sourceLocation: sourceLocation)
  }

  @Test(arguments: [false, true])
  func theWordsAtTheTopStayThereThroughEveryChangeOfWidthAndARoundTripReturnsExactly(animated: Bool) async throws {
    let list = try await HostedList(wordRows: 120)
    defer { list.close() }
    let view = list.view
    let (width, height) = (view.frame.width, view.frame.height)
    for share in [0.15, 0.5, 0.8] {
      try await list.drag(to: view.contentSize.height * share)
      let place = list.atTheTop
      let began = list.onScreen
      let offset = view.contentOffset.y
      for _ in 0..<3 {
        // As a fold does, through widths never come back to.
        for through in [width - 55, width - 31, width - 120] {
          try await list.stand(across: through, height: height / 2, animated: animated)
          try stands(list, at: place, "across \(through) points")
        }
        try await list.stand(across: width, height: height, animated: animated)
        #expect(list.onScreen == began, "exactly where it began")
        #expect(view.contentOffset.y == offset, "the rows above standing as they did")
        #expect(!list.follower.following)
      }
    }
  }

  @Test func atItsFootItStaysAtItsFootThroughAChangeOfWidth() async throws {
    let list = try await HostedList(wordRows: 80)
    defer { list.close() }
    let width = list.view.frame.width
    for across in [width - 90, width - 30, width, width - 90] {
      try await list.stand(across: across, animated: across != width)
      #expect(list.follower.following)
      #expect(list.view.distanceToFoot < 1, "at the foot across \(across) points")
    }
  }

  /// Near the foot, where a narrower list has the place and a wider one ends before it comes to the top.
  @Test func aPlaceNearTheFootIsComeBackToThoughTheListEndsBeforeItElsewhere() async throws {
    let list = try await HostedList(wordRows: 60)
    defer { list.close() }
    let view = list.view
    try await list.stand(across: 220)
    try await list.drag(to: view.contentOffset.y - 300)
    let began = list.onScreen
    try await list.stand(across: 400, height: 900)
    // As a fling the change of width came in the middle of is told of as ended.
    list.follower.scrollViewDidEndDecelerating(view)
    #expect(!list.follower.following, "at the foot only for want of room, and not following it")
    try await list.stand(across: 220, height: 874)
    #expect(list.onScreen == began)
  }

  @Test func whatComesInAndMeasuresAnewMeanwhileMovesNothingTheReaderSees() async throws {
    let list = try await HostedList(wordRows: 80)
    defer { list.close() }
    let view = list.view
    let width = view.frame.width
    try await list.drag(to: view.contentSize.height / 2)
    let place = list.atTheTop
    let began = list.onScreen
    try await list.stand(across: width - 90)
    let turned = view.contentOffset.y
    list.follower.take(list.rows(90))
    try await list.frames(10)
    #expect(view.contentOffset.y == turned, "rows that came in at the foot")
    list.follower.report("item:0", list.follower.geometry.height(of: "item:0") + 400)
    #expect(view.contentOffset.y == turned, "not scrolled for it")
    try await list.frames(10)
    #expect(view.contentOffset.y == turned + 400, "a row above, grown")
    try stands(list, at: place, "the same words at the top")
    try await list.stand(across: width)
    #expect(list.onScreen == began, "and at the width it began at, where none of that stands above it")
  }

  /// While a row in sight grows, with no change of width since the reader scrolled, the list keeps what stands at the
  /// foot of the screen. What has come to stand at the top is their place.
  @Test func aRowInSightGrowingBeforeTheChangeOfWidthLeavesThePlaceAtWhatStandsAtTheTop() async throws {
    let list = try await HostedList(wordRows: 80)
    defer { list.close() }
    let view = list.view
    let width = view.frame.width
    try await list.drag(to: view.contentSize.height / 2)
    let seen = try #require(list.atTheTop).row
    let stood = view.contentOffset.y
    list.follower.take(list.rows(80, grown: seen))
    try await list.frames(40)
    #expect(view.contentOffset.y > stood + 100, "as much as it grew")
    let place = list.atTheTop
    let began = list.onScreen
    try await list.stand(across: width - 90)
    try stands(list, at: place, "what came to the top")
    try await list.stand(across: width)
    #expect(list.onScreen == began)
  }

  /// What the reader next scrolls to is their place, by whatever hand.
  @Test(arguments: ["drag", "offset", "animated offset", "rect", "animated rect", "row", "page", "top"])
  func theReadersNextScrollEndsIt(_ hand: String) async throws {
    let list = try await HostedList(wordRows: 120)
    defer { list.close() }
    let view = list.view
    let width = view.frame.width
    try await list.drag(to: view.contentSize.height / 2)
    try await list.stand(across: width - 90)
    #expect(list.follower.holding)
    let turned = view.contentOffset.y
    let up = CGPoint(x: 0, y: turned - 700)
    let rect = CGRect(x: 0, y: view.visibleHead - 700, width: 10, height: 10)
    switch hand {
    case "drag": try await list.drag(to: up.y)
    case "offset": view.setContentOffset(up, animated: false)
    case "animated offset": view.setContentOffset(up, animated: true)
    case "rect": view.scrollRectToVisible(rect, animated: false)
    case "animated rect": view.scrollRectToVisible(rect, animated: true)
    case "row": view.scrollToItem(at: IndexPath(item: 30, section: 0), at: .top, animated: false)
    case "page": #expect(view.accessibilityScroll(.down))
    default:
      #expect(list.follower.scrollViewShouldScrollToTop(view))
      view.contentOffset.y = -view.adjustedContentInset.top
      list.follower.scrollViewDidScrollToTop(view)
    }
    try await list.frames(60)
    #expect(!list.follower.holding)
    #expect(turned - view.contentOffset.y > 300, "where it was taken it stays: \(turned - view.contentOffset.y)")
    let place = list.atTheTop
    let scrolled = list.onScreen
    try await list.stand(across: width)
    try stands(list, at: place, "what was scrolled to")
    try await list.stand(across: width - 90)
    #expect(list.onScreen == scrolled, "and come back to exactly")
  }

  /// Asked to bring into sight what is in sight, as the system asks for a field's, nothing scrolled.
  @Test(arguments: [false, true])
  func askedForWhatIsInSightItStandsAtThePlaceStill(animated: Bool) async throws {
    let list = try await HostedList(wordRows: 80)
    defer { list.close() }
    let view = list.view
    let width = view.frame.width
    try await list.drag(to: view.contentSize.height / 2)
    let began = list.onScreen
    try await list.stand(across: width - 90)
    view.scrollRectToVisible(CGRect(x: 0, y: view.visibleHead + 100, width: 10, height: 10), animated: animated)
    view.setContentOffset(view.contentOffset, animated: animated)
    try await list.frames(30)
    #expect(list.follower.holding)
    try await list.stand(across: width)
    #expect(list.onScreen == began)
  }

  @Test func sendingGoesToTheFootFromThePlace() async throws {
    let list = try await HostedList(wordRows: 80)
    defer { list.close() }
    try await list.drag(to: list.view.contentSize.height / 2)
    try await list.stand(across: list.view.frame.width - 90)
    list.follower.following = true
    try await list.frames(30)
    #expect(list.view.distanceToFoot < 1)
    try await list.stand(across: list.view.frame.width + 90)
    #expect(list.follower.following)
    #expect(list.view.distanceToFoot < 1)
  }

  /// Held at a place part-way through a row that then grows at its end, as a block streaming in does: the words at the
  /// top stay where they are, though they are a smaller share of the row.
  @Test func aRowAtThePlaceGrowingWhileHeldMovesNothingTheReaderSees() async throws {
    let list = try await HostedList(wordRows: 80)
    defer { list.close() }
    let view = list.view
    try await list.drag(to: view.contentSize.height / 2)
    let ids = list.follower.geometry.ids
    let top = try #require(list.atTheTop)
    let first = try #require(ids.firstIndex(of: top.row))
    let tall = try #require(ids[first..<(first + 12)].first { list.follower.geometry.height(of: $0) > 100 })
    let depth = try #require(list.follower.geometry.depth(of: ConversationPlace(row: tall, share: 0.5, above: 0)))
    try await list.drag(to: depth - view.adjustedContentInset.top)
    try await list.stand(across: view.frame.width - 90)
    #expect(list.follower.holding)
    #expect(list.atTheTop?.row == tall)
    let before = list.onScreen
    let was = list.follower.geometry.height(of: tall)
    list.follower.take(list.rows(80, grown: tall))
    try await list.frames(60)
    #expect(list.follower.geometry.height(of: tall) > was + 50, "it grew")
    #expect(list.onScreen == before, "the words at the top stay")
    #expect(list.follower.holding)
  }

  /// A row as tall across every width, as code is, grows while the list stands across another width: come back to the
  /// first, it stands as tall as it is, not as it was there.
  @Test func aRowThatChangedAcrossAnotherWidthMeasuresAnewAcrossTheFirst() async throws {
    let list = try await HostedList(rows: 5) { row, follower in
      var grown = false
      if case .item(let item) = row, case .error(let text) = item.kind { grown = text.hasSuffix("grown") }
      return UIHostingConfiguration {
        Color.red.frame(height: grown ? 200 : 60)
          .onGeometryChange(for: CGFloat.self) {
            ceil($0.size.height)
          } action: { [weak follower] height in
            follower?.report(row.id, height)
          }
          .frame(maxHeight: .infinity, alignment: .top)
      }
      .margins(.all, 0)
    }
    defer { list.close() }
    let width = list.view.frame.width
    try await list.stand(across: width - 90)
    list.follower.take(list.rows(5, grown: "item:4"))
    try await list.frames(60)
    #expect(list.follower.geometry.height(of: "item:4") == 200, "grown across the other width")
    try await list.stand(across: width)
    try await list.frames(60)
    #expect(list.follower.geometry.height(of: "item:4") == 200, "and across the first")
  }

  /// The row at the place leaves while the list stands there, with rows above it, as Prompts sent and queued do once the
  /// Server has them: what stood below it stands where it stood.
  @Test func theRowAtThePlaceLeavingWithRowsAboveItLeavesWhatStoodBelowItWhereItStood() async throws {
    let list = try await HostedList(wordRows: 80)
    defer { list.close() }
    let view = list.view
    try await list.drag(to: view.contentSize.height / 2)
    try await list.stand(across: view.frame.width - 90)
    #expect(list.follower.holding)
    let ids = list.follower.geometry.ids
    let place = try #require(list.atTheTop)
    let index = try #require(ids.firstIndex(of: place.row))
    let next = ids[index + 1]
    func below() throws -> CGFloat {
      try #require(list.follower.geometry.depth(of: ConversationPlace(row: next, share: 0, above: 0)))
        - view.visibleHead
    }
    let stood = try below()
    let gone = Set(ids[(index - 3)...index])
    list.follower.take(list.rows(80).filter { !gone.contains($0.id) })
    try await list.frames(30)
    #expect(list.follower.place?.row == next, "the place is now above it")
    #expect(abs(try below() - stood) < 0.5, "what stood below it, \(next), stands where it stood")
    #expect(list.follower.holding)
  }
}
