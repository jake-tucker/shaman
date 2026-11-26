import Testing
import UIKit

@testable import ShamanUI

/// Where the system stands the main view alone, as on a Duo's inner display in portrait, 669 x 951 at regular width, and
/// where it divides the columns at the fold, folded like a book: in a window of the app's own scene, turned as a size
/// transition turns it. No test poses a Duo, so neither the system's own dimming of the main view, nor a tap beside the
/// Sidebar, nor a fold the split view itself divides at, is held here: `PosedPortraitTests` and `PosedBookTests` hold
/// those.
@Suite(.serialized) @MainActor struct StandingSidebarTests {
  private let session = SessionRoute(session: "workstation-key/7")
  private var open: Pushed { .session(session) }
  private static let portrait = CGSize(width: 669, height: 951)
  private static let landscape = CGSize(width: 951, height: 669)

  /// The split view's own word for where the Sidebar stands.
  private typealias Mode = UISplitViewController.DisplayMode

  private func turn(_ hosted: HostedColumns, to size: CGSize) async throws {
    hosted.size(width: size.width, height: size.height, regular: true)
    try await hosted.settle()
  }

  private func stands(_ mode: Mode, in hosted: HostedColumns, _ said: String = "") async throws {
    try await hosted.until({ hosted.columns.displayMode == mode }, "\(said): \(hosted.columns.displayMode)")
    try await hosted.settle()
    #expect(hosted.columns.displayMode == mode, "\(said)")
    // Asked for, the preference is never written.
    #expect(hosted.columns.preferredDisplayMode == .oneBesideSecondary, "\(said)")
    #expect(!hosted.columns.isCollapsed, "\(said)")
  }

  /// Presses the Sidebar's own button, the system's, as the reader does to put it away or show it: the one bar button
  /// in the window, as the screens the test hosts put none in their bars.
  private func pressTheSidebarsButton(_ hosted: HostedColumns) async throws {
    var found: [UIControl] = []
    try await hosted.until {
      found = Self.barButtons(in: hosted.window)
      return found.count == 1
    }
    try #require(found.count == 1)
    found[0].sendActions(for: .touchUpInside)
  }

  private static func barButtons(in view: UIView) -> [UIControl] {
    guard !view.isHidden, view.alpha > 0 else { return [] }
    if let control = view as? UIControl, "\(type(of: control))".contains("ButtonBarButton") { return [control] }
    return view.subviews.flatMap { barButtons(in: $0) }
  }

  @Test func whileNothingIsOpenInPortraitTheSidebarStandsOverTheMainViewAsTheColumnsAppear() async throws {
    let hosted = try await HostedColumns(size: Self.portrait)
    defer { hosted.close() }
    try await stands(.oneOverSecondary, in: hosted, "launched in portrait")
    #expect(hosted.held == [nil], "over the new-session screen at rest")
    #expect((hosted.main.viewControllers.first as? HostedScreen)?.atRest == hosted.stack.rest)
    #expect(hosted.stack.path.isEmpty)
  }

  @Test func withASessionOpenInPortraitItStandsAloneAndInLandscapeTheSidebarBesideAsTheyAppear() async throws {
    let alone = try await HostedColumns(open: [open], size: Self.portrait)
    try await stands(.secondaryOnly, in: alone, "a Session alone")
    #expect(alone.held == [open])
    alone.close()
    let beside = try await HostedColumns(size: Self.landscape)
    defer { beside.close() }
    try await stands(.oneBesideSecondary, in: beside, "nothing open in landscape")
  }

  @Test func itStandsOverTheMainViewAsTheColumnsExpandIntoPortraitBeforeTheirSizeTransitionEnds() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    #expect(hosted.listingAlone)
    let expanded = hosted.sizing(width: Self.portrait.width, height: Self.portrait.height, regular: true)
    try await hosted.until { !hosted.columns.isCollapsed }
    try await stands(.oneOverSecondary, in: hosted, "as the split view says it did expand")
    expanded()
    try await stands(.oneOverSecondary, in: hosted, "and as the transition ends")
    #expect(hosted.held == [nil])

    // With a Session open, it expands to the Session alone.
    try await hosted.resize(wide: false)
    hosted.stack.open("workstation-key/7")
    try await hosted.settle()
    try await turn(hosted, to: Self.portrait)
    try await stands(.secondaryOnly, in: hosted, "a Session open")
  }

  @Test func turnedToPortraitItStandsOverTheMainViewAsTheSizeTransitionEndsAndTurnedBackBesideIt() async throws {
    let hosted = try await HostedColumns(size: Self.landscape)
    defer { hosted.close() }
    try await stands(.oneBesideSecondary, in: hosted)
    for _ in 0..<2 {
      let turned = hosted.sizing(width: Self.portrait.width, height: Self.portrait.height, regular: true)
      try await hosted.settle()
      #expect(hosted.columns.displayMode == .secondaryOnly, "the system's main view alone, until the turn ends")
      turned()
      try await stands(.oneOverSecondary, in: hosted, "turned to portrait")
      // The system tiles it again by itself.
      try await turn(hosted, to: Self.landscape)
      try await stands(.oneBesideSecondary, in: hosted, "turned back")
    }
    #expect(hosted.held == [nil])
    #expect(hosted.stack.path.isEmpty)
  }

  @Test func itStandsOverTheMainViewAsThePathEmptiesAndGoesAsSomethingOpensWithNothingAsked() async throws {
    let hosted = try await HostedColumns(open: [open], size: Self.portrait)
    defer { hosted.close() }
    try await stands(.secondaryOnly, in: hosted, "a Session alone")
    hosted.stack.path = []
    try await stands(.oneOverSecondary, in: hosted, "closed")
    #expect(hosted.held == [nil])
    // Chosen from it, a Session stands alone.
    hosted.stack.open("workstation-key/7")
    try await stands(.secondaryOnly, in: hosted, "a Session chosen")
    #expect(hosted.held == hosted.stack.path.map(Optional.some))
    #expect(hosted.stack.openSession?.session == session.session)
    // Gone, as a Session goes.
    hosted.stack.leave(try #require(hosted.stack.openSession))
    try await stands(.oneOverSecondary, in: hosted, "the Session gone")
    #expect(hosted.held == [nil])
    // And the screen at rest composed afresh, then left.
    hosted.stack.compose()
    try await stands(.secondaryOnly, in: hosted, "the new-session screen open")
    hosted.stack.path = []
    try await stands(.oneOverSecondary, in: hosted, "left")
  }

  @Test func inLandscapeThePathEmptiedWithTheAsideShownStandsTheSidebarBesideTheScreenAtRest() async throws {
    let hosted = try await HostedColumns(open: [open], size: Self.landscape)
    defer { hosted.close() }
    hosted.stack.aside.shown = true
    try await hosted.until { hosted.columns.isShowing(.inspector) }
    try await stands(.secondaryOnly, in: hosted, "the Sidebar behind its button beside the Aside")
    hosted.stack.path = []
    try await hosted.until { hosted.columns.viewController(for: .inspector) == nil }
    try await stands(.oneBesideSecondary, in: hosted, "closed")
    #expect(hosted.held == [nil])
    #expect(hosted.columns.presentedViewController == nil)
    #expect(!hosted.columns.isShowing(.inspector))
  }

  @Test func putAwayItStaysAwayUntilTheNextMomentAndATapsAwayIsNotKeptThroughATurn() async throws {
    let hosted = try await HostedColumns(size: Self.portrait)
    defer { hosted.close() }
    try await stands(.oneOverSecondary, in: hosted)
    // As a tap beside it puts it away, which the system does and no hosted test can.
    hosted.columns.show(.secondary)
    try await stands(.secondaryOnly, in: hosted, "put away")
    // Laid out again, the path written as it was, the Aside's state told: none is a moment to ask at.
    hosted.stack.path = []
    hosted.stack.aside.shown = true
    hosted.stack.aside.shown = false
    hosted.columns.view.setNeedsLayout()
    hosted.columns.view.layoutIfNeeded()
    try await hosted.settle()
    #expect(hosted.columns.displayMode == .secondaryOnly, "it stays away")
    // Nor is the end of a change of size it began away, as where the window grows no wider.
    try await turn(hosted, to: CGSize(width: Self.portrait.width, height: Self.portrait.height - 1))
    try await turn(hosted, to: Self.portrait)
    #expect(hosted.columns.displayMode == .secondaryOnly, "still away")
    // The system does not keep a tap: turned, it tiles the Sidebar again, which then stands over the main view turned
    // back.
    try await turn(hosted, to: Self.landscape)
    try await stands(.oneBesideSecondary, in: hosted, "turned to landscape")
    try await turn(hosted, to: Self.portrait)
    try await stands(.oneOverSecondary, in: hosted, "turned back")
  }

  @Test func aSidebarPutAwayByItsOwnButtonStaysAwayThroughATurnAndBack() async throws {
    let hosted = try await HostedColumns(size: Self.landscape)
    defer { hosted.close() }
    try await stands(.oneBesideSecondary, in: hosted)
    try await pressTheSidebarsButton(hosted)
    try await stands(.secondaryOnly, in: hosted, "put away by its button")
    for size in [Self.portrait, Self.landscape, Self.portrait, Self.landscape] {
      try await turn(hosted, to: size)
      try await hosted.settle()
      #expect(hosted.columns.displayMode == .secondaryOnly, "away at \(size)")
      #expect(hosted.columns.preferredDisplayMode == .oneBesideSecondary, "the button writes no preference")
    }
    // Shown by it again, it stands over the main view in portrait and beside it in landscape.
    try await pressTheSidebarsButton(hosted)
    try await stands(.oneBesideSecondary, in: hosted, "shown by its button")
    try await turn(hosted, to: Self.portrait)
    try await stands(.oneOverSecondary, in: hosted, "turned to portrait")
    try await turn(hosted, to: Self.landscape)
    try await stands(.oneBesideSecondary, in: hosted, "turned back")

    // With a Session open, put away it stays away too.
    hosted.stack.open("workstation-key/7")
    try await hosted.settle()
    try await pressTheSidebarsButton(hosted)
    try await stands(.secondaryOnly, in: hosted, "a Session open, the Sidebar put away")
    for size in [Self.portrait, Self.landscape] {
      try await turn(hosted, to: size)
      #expect(hosted.columns.displayMode == .secondaryOnly, "away at \(size)")
      #expect(hosted.held == hosted.stack.path.map(Optional.some))
    }
  }

  @Test func itIsNeverStoodOverAFieldThatHoldsTheKeyboard() async throws {
    let hosted = try await HostedColumns(size: Self.portrait)
    defer { hosted.close() }
    try await stands(.oneOverSecondary, in: hosted)
    let route = hosted.stack.rest
    hosted.columns.show(.secondary)
    try await stands(.secondaryOnly, in: hosted, "put away")
    let field = try await hosted.field()
    #expect(field.becomeFirstResponder())
    try await hosted.until { hosted.stack.path == [.landing(route)] }
    let untouched = {
      #expect(hosted.columns.displayMode != .oneOverSecondary, "not over the field")
      #expect(field.isFirstResponder, "which keeps the keyboard")
      #expect(hosted.stack.path == [.landing(route)], "open")
    }
    // Turned and back, collapsed and expanded, and its path written again.
    for size in [Self.landscape, Self.portrait] {
      try await turn(hosted, to: size)
      untouched()
    }
    try await hosted.resize(wide: false)
    let expanded = hosted.sizing(width: Self.portrait.width, height: Self.portrait.height, regular: true)
    try await hosted.until { !hosted.columns.isCollapsed }
    try await hosted.settle()
    untouched()
    expanded()
    try await hosted.settle()
    untouched()
    hosted.stack.path = [.landing(route)]
    try await hosted.settle()
    untouched()
  }

  /// What the container sets of the split view, and what it shows, read where nothing should change it.
  private struct Stood: Equatable {
    let mode: Mode
    let preferred: Mode
    let behavior: UISplitViewController.SplitBehavior
    let preferredBehavior: UISplitViewController.SplitBehavior
    let leastMain: CGFloat
    let leastSidebar: CGFloat
    let mostSidebar: CGFloat
    let sidebar: CGFloat
    let sidebarShare: CGFloat
    let showing: [Bool]
    let aside: Bool
    let presented: Bool
    let held: [ObjectIdentifier]
    let listed: [ObjectIdentifier]

    @MainActor init(_ hosted: HostedColumns) {
      let columns = hosted.columns
      mode = columns.displayMode
      preferred = columns.preferredDisplayMode
      behavior = columns.splitBehavior
      preferredBehavior = columns.preferredSplitBehavior
      leastMain = columns.minimumSecondaryColumnWidth
      leastSidebar = columns.minimumPrimaryColumnWidth
      mostSidebar = columns.maximumPrimaryColumnWidth
      sidebar = columns.preferredPrimaryColumnWidth
      sidebarShare = columns.preferredPrimaryColumnWidthFraction
      showing = [.primary, .secondary, .inspector].map(columns.isShowing)
      aside = columns.viewController(for: .inspector) != nil
      presented = columns.presentedViewController != nil
      held = hosted.main.viewControllers.map(ObjectIdentifier.init)
      listed = hosted.sidebar.viewControllers.map(ObjectIdentifier.init)
    }
  }

  @Test func foldedLikeABookTheContainerSetsAndShowsNothingAsTheFoldComesAndGoes() async throws {
    // The pose reads no fold standing upright, which the package's tests hold, so the container hears nothing of the
    // book's, and the split view divides the columns at it by itself, given no least width for the main view. Told
    // of a fold all the same, it sets nothing.
    let fold = Fold(top: 314.5, bottom: 354.5, window: Self.landscape.height)
    for path in [[open], []] {
      let hosted = try await HostedColumns(open: path, size: Self.landscape)
      #expect(!hosted.stack.aside.shown, "the Aside not shown")
      try await stands(.oneBesideSecondary, in: hosted, "\(path)")
      #expect(hosted.columns.minimumSecondaryColumnWidth == UISplitViewController.automaticDimension)
      let stood = Stood(hosted)
      for told in [fold, nil, fold, nil] {
        hosted.stack.pose.fold = told
        hosted.columns.view.setNeedsLayout()
        hosted.columns.view.layoutIfNeeded()
        try await hosted.settle()
        #expect(Stood(hosted) == stood, "\(path), the fold \(told.map { "\($0)" } ?? "gone")")
        #expect(hosted.stack.path == path)
      }
      hosted.close()
    }
  }
}
