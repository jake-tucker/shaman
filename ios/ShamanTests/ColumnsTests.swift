import Testing
import UIKit

@testable import ShamanUI

@Suite(.serialized, .oneWindowAtATime) @MainActor struct ColumnsTests {
  private let session = SessionRoute(session: "workstation-key/7")
  private var open: Pushed { .session(session) }
  private var work: Pushed { .work(session, turn: "work") }
  private var detail: Pushed { .detail(session, activity: "check") }

  @Test func theSidebarIsPreferredBesideTheMainViewWhichIsGivenNoLeastWidth() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    let columns = hosted.columns
    let preferred = {
      #expect(columns.style == .doubleColumn)
      #expect(columns.preferredDisplayMode == .oneBesideSecondary)
      #expect(columns.preferredSplitBehavior == .tile)
      // With one the split view will not divide the Sidebar and the main view at a fold.
      #expect(columns.minimumSecondaryColumnWidth == UISplitViewController.automaticDimension)
    }
    preferred()
    // Nor is the Sidebar's column ever hidden, which would write the main view alone into what is preferred.
    for wide in [false, true, false] {
      try await hosted.resize(wide: wide)
      hosted.stack.path = [open, work]
      try await hosted.settle()
      preferred()
      hosted.stack.path = []
      try await hosted.settle()
      preferred()
    }
  }

  @Test func whatIsOpenIsPushedOverTheListingAndTheScreensHeldAreKept() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    #expect(hosted.listingAlone)
    hosted.stack.path = [open]
    try await hosted.until { hosted.sidebar.topViewController !== hosted.sidebar.viewControllers.first }
    let root = try #require(hosted.main.viewControllers.first)
    hosted.stack.path += [work, detail]
    try await hosted.settle()
    #expect(hosted.held == [open, work, detail])
    #expect(hosted.main.viewControllers.first === root)
    let pushed = hosted.main.viewControllers[1]
    hosted.stack.path = [open, work]
    try await hosted.settle()
    #expect(hosted.main.viewControllers == [root, pushed], "popped to, each the screen it was")
    // Another Session in its place is a screen of its own.
    hosted.stack.open("workstation-key/7")
    #expect(hosted.stack.path == [open])
    hosted.stack.open("workstation-key/1")
    try await hosted.settle()
    #expect(hosted.main.viewControllers.count == 1)
    #expect(hosted.main.viewControllers.first !== root)
  }

  @Test func whatTheReaderPopsIsReadBackAndOnlyReturningToTheListingClosesWhatIsOpen() async throws {
    let hosted = try await HostedColumns(open: [open, work, detail])
    defer { hosted.close() }
    try await hosted.until { hosted.held == [open, work, detail] }
    hosted.main.popViewController(animated: true)
    try await hosted.until { hosted.stack.path == [open, work] }
    hosted.main.popViewController(animated: true)
    try await hosted.until { hosted.stack.path == [open] }
    try await hosted.settle()
    #expect(hosted.stack.path == [open])
    hosted.sidebar.popViewController(animated: true)
    try await hosted.until { hosted.stack.path.isEmpty && hosted.listingAlone }
    try await hosted.until { hosted.held == [nil] }
    #expect((hosted.main.viewControllers.first as? HostedScreen)?.atRest == hosted.stack.rest)
    // And again, what closed the first leaving the second whole.
    hosted.stack.open("workstation-key/1")
    try await hosted.settle()
    #expect(hosted.stack.path.count == 1)
    #expect(!hosted.listingAlone)
  }

  @Test func screensPushedOneUponAnotherAllStand() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    // Each in the middle of the last one's push.
    hosted.stack.path = [open]
    hosted.stack.path.append(work)
    try await Task.sleep(for: .milliseconds(60))
    hosted.stack.path.append(detail)
    try await Task.sleep(for: .milliseconds(60))
    // Twice, as tapped twice it would be were it not held back.
    hosted.stack.path.append(detail)
    try await hosted.settle()
    #expect(hosted.stack.path == [open, work, detail, detail])
    #expect(hosted.held == [open, work, detail, detail])
    #expect(!hosted.listingAlone)
    // And popped in the middle of it.
    hosted.stack.path.removeLast()
    try await Task.sleep(for: .milliseconds(60))
    hosted.stack.path.removeLast()
    try await hosted.settle()
    #expect(hosted.stack.path == [open, work])
    #expect(hosted.held == [open, work])
  }

  @Test func aSessionGoneReturnsToTheListingWhereItStoodAlone() async throws {
    let hosted = try await HostedColumns(open: [open, work])
    defer { hosted.close() }
    #expect(!hosted.listingAlone)
    hosted.stack.leave(session)
    try await hosted.until { hosted.listingAlone }
    try await hosted.until { hosted.held == [nil] }
    #expect(hosted.stack.path.isEmpty)
  }

  @Test func aSessionGoneAsItOrAScreenOverItIsPushedReturnsToTheListing() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    for pushed in [[], [work], [work, detail]] {
      hosted.stack.path = [open]
      if !pushed.isEmpty {
        try await hosted.settle()
        hosted.stack.path += pushed
      }
      try await Task.sleep(for: .milliseconds(60))
      hosted.stack.leave(session)
      try await hosted.until { hosted.listingAlone && hosted.held == [nil] && !hosted.sidebar.isToolbarHidden }
      try await hosted.settle()
      #expect(hosted.stack.path.isEmpty)
      #expect(hosted.listingAlone)
      #expect(hosted.held == [nil])
      #expect(!hosted.sidebar.isToolbarHidden)
    }
  }

  @Test func aSessionGoneAsTheReaderPopsWithinItIsNotOpenedAgain() async throws {
    let hosted = try await HostedColumns(open: [open, work])
    defer { hosted.close() }
    try await hosted.until { hosted.held == [open, work] }
    hosted.main.popViewController(animated: true)
    try await Task.sleep(for: .milliseconds(60))
    #expect(hosted.stack.path == [open, work], "in the middle of the pop")
    hosted.stack.leave(session)
    #expect(hosted.stack.path.isEmpty)
    try await hosted.settle()
    #expect(hosted.stack.path.isEmpty)
    #expect(hosted.listingAlone)
    #expect(hosted.held == [nil])
  }

  @Test func anotherOpenedAsTheListingIsReturnedToStaysOpen() async throws {
    let hosted = try await HostedColumns(open: [open])
    defer { hosted.close() }
    for session in ["workstation-key/7", "workstation-key/1"] {
      hosted.sidebar.popViewController(animated: true)
      try await Task.sleep(for: .milliseconds(80))
      hosted.stack.open(session)
      try await hosted.settle()
      #expect(hosted.stack.path.count == 1)
      #expect(!hosted.listingAlone)
      #expect(hosted.held == hosted.stack.path.map(Optional.some))
    }
  }

  @Test func collapsingAndExpandingClosesNothingAndMakesNoScreenAfresh() async throws {
    let hosted = try await HostedColumns(open: [open, work])
    defer { hosted.close() }
    let screens = hosted.main.viewControllers
    #expect(screens.count == 2)
    for wide in [true, false, true, false] {
      try await hosted.resize(wide: wide)
      #expect(hosted.stack.path == [open, work])
      #expect(hosted.main.viewControllers == screens)
      #expect(hosted.listingAlone == false)
    }
    hosted.main.popViewController(animated: true)
    try await hosted.until { hosted.stack.path == [open] }
    try await hosted.resize(wide: true)
    #expect(hosted.columns.displayMode == .oneBesideSecondary, "the Sidebar beside the main view")
    hosted.stack.open("workstation-key/1")
    try await hosted.settle()
    #expect(hosted.held == hosted.stack.path.map(Optional.some))
    #expect(hosted.columns.displayMode == .oneBesideSecondary)
  }

  @Test func withNothingOpenTheMainViewRestsAndCollapsingLandsOnTheListing() async throws {
    let hosted = try await HostedColumns(wide: true)
    defer { hosted.close() }
    #expect(hosted.held == [nil])
    try await hosted.resize(wide: false)
    #expect(hosted.listingAlone)
    #expect(hosted.stack.path.isEmpty)
    try await hosted.resize(wide: true)
    #expect(hosted.held == [nil])
    hosted.stack.path = [open]
    try await hosted.settle()
    hosted.stack.path = []
    try await hosted.settle()
    #expect(hosted.held == [nil], "what is open closed beside the listing")
  }

  @Test func theScreenAtRestOpenedIsTheSameScreen() async throws {
    let hosted = try await HostedColumns(wide: true)
    defer { hosted.close() }
    let resting = try #require(hosted.main.viewControllers.first as? HostedScreen)
    let route = hosted.stack.rest
    hosted.stack.openRest(route)
    #expect(hosted.stack.path == [.landing(route)])
    #expect(hosted.main.viewControllers == [resting])
    #expect(resting.named == .landing(route))
    #expect(hosted.stack.rest != route, "another rests once nothing is open")
    try await hosted.resize(wide: false)
    #expect(!hosted.listingAlone, "opened, it is what one column shows")
    #expect(hosted.main.viewControllers == [resting])
  }

  @Test func theAsidesColumnHoldsItOnlyWhileItIsShownWithASessionOpenAndRoom() async throws {
    let hosted = try await HostedColumns(open: [open], wide: true)
    defer { hosted.close() }
    let columns = hosted.columns
    let aside = hosted.stack.aside
    let held = { columns.viewController(for: .inspector) }
    let stands = {
      try await hosted.until { held() != nil && columns.isShowing(.inspector) }
      try await hosted.settle()
      #expect(held() != nil && columns.isShowing(.inspector))
      #expect(hosted.stack.asideIsOnScreen)
    }
    let gone = {
      try await hosted.until { held() == nil }
      try await hosted.settle()
      #expect(held() == nil && !columns.isShowing(.inspector))
      #expect(!hosted.stack.asideIsOnScreen)
    }
    #expect(columns.minimumInspectorColumnWidth == 290)
    #expect(aside.hasRoom)
    #expect(held() == nil, "hidden as it begins")

    aside.shown = true
    try await stands()
    let screen = held()
    #expect(columns.displayMode == .secondaryOnly, "the Sidebar behind its button")
    aside.shown = false
    try await gone()
    #expect(columns.displayMode == .oneBesideSecondary, "the Sidebar back beside the main view")

    // Asked for as it goes, and taken down as it comes.
    for shown in [true, false, true, false] {
      aside.shown = shown
      try await Task.sleep(for: .milliseconds(80))
    }
    try await gone()
    for shown in [true, false, true] {
      aside.shown = shown
      try await Task.sleep(for: .milliseconds(80))
    }
    try await stands()

    // Called over it, the Sidebar leaves it standing.
    columns.show(.primary)
    try await hosted.until { columns.displayMode == .oneOverSecondary }
    try await stands()
    columns.show(.secondary)
    try await hosted.until { columns.displayMode == .secondaryOnly }
    try await stands()

    // From one Session to the next, and not while none is open.
    aside.shown = true
    try await stands()
    hosted.stack.open("workstation-key/1")
    try await stands()
    hosted.stack.compose()
    try await gone()
    #expect(columns.displayMode == .oneBesideSecondary)
    hosted.stack.open("workstation-key/7")
    try await stands()
    #expect(held() === screen)

    // Nor with one column alone.
    try await hosted.resize(wide: false)
    try await gone()
    #expect(hosted.columns.presentedViewController == nil, "and it stands nowhere else")
    #expect(aside.shown)
    try await hosted.resize(wide: true)
    try await stands()
    #expect(columns.preferredDisplayMode == .oneBesideSecondary)
  }

  @Test func theAsideLeavesItsColumnAsTheColumnsCollapseAndReturnsOnlyAsTheirSizeTransitionEnds() async throws {
    let hosted = try await HostedColumns(open: [open], wide: true)
    defer { hosted.close() }
    let columns = hosted.columns
    let stands = { columns.viewController(for: .inspector) != nil && columns.isShowing(.inspector) }
    hosted.stack.aside.shown = true
    try await hosted.until(stands)
    try await hosted.settle()
    for _ in 0..<2 {
      // Out of its column as the split view asks which column to collapse to, or it would stand as a sheet.
      let collapsed = hosted.sizing(width: 402, regular: false)
      try await hosted.until { columns.isCollapsed }
      #expect(columns.viewController(for: .inspector) == nil)
      #expect(columns.presentedViewController == nil)
      #expect(!hosted.stack.asideIsOnScreen)
      collapsed()
      try await hosted.settle()
      #expect(columns.viewController(for: .inspector) == nil)
      #expect(columns.presentedViewController == nil, "it stands nowhere")
      #expect(hosted.stack.aside.shown, "and waits")

      // Set again and shown once the transition has ended, and not before.
      let expanded = hosted.sizing(width: 951, regular: true)
      try await hosted.until { !columns.isCollapsed }
      try await hosted.settle()
      #expect(columns.viewController(for: .inspector) == nil)
      #expect(!hosted.stack.asideIsOnScreen)
      expanded()
      try await hosted.until(stands)
      try await hosted.settle()
      #expect(hosted.stack.asideIsOnScreen)
      #expect(columns.presentedViewController == nil)
    }
  }

  @Test func theAsideFollowsTheLatestTurnWhateverIsPushedOverItsSession() async throws {
    let hosted = try await HostedColumns(open: [open], wide: true, sessions: true)
    defer { hosted.close() }
    let model = hosted.stack.model(for: session)
    hosted.stack.aside.shown = true
    try await hosted.until { hosted.columns.isShowing(.inspector) && model.conversation.latestTurn == "work" }
    hosted.stack.path.append(work)
    try await hosted.settle()
    try await hosted.until { hosted.held == [open, work] }
    model.text = "Go on"
    await model.send()
    try await hosted.until { model.conversation.latestTurn == "turn-sent-1" }
    // Back, its own screen reads it on, one change after another.
    hosted.stack.path.removeLast()
    try await hosted.settle()
    #expect(model.conversation.latestTurn == "turn-sent-1")
    await model.stop()
    model.text = "And again"
    await model.send()
    try await hosted.until { model.conversation.latestTurn == "turn-sent-2" }
  }

  @Test func theAsideHasRoomWhereColumnsStandIn690PointsOrMore() async throws {
    let hosted = try await HostedColumns(open: [open], wide: true)
    defer { hosted.close() }
    for (width, regular, room) in [
      (690, true, true), (689, true, false), (951, true, true), (669, true, false), (874, false, false),
      (956, true, true),
    ] as [(CGFloat, Bool, Bool)] {
      hosted.size(width: width, regular: regular)
      try await hosted.until { hosted.stack.aside.hasRoom == room }
      try await hosted.settle()
      #expect(hosted.stack.aside.hasRoom == room, "\(width)")
    }
    // Shown once, it stands wherever there is enough, the window's change of size alone bringing it back.
    hosted.stack.aside.shown = true
    for (width, stands) in [(669, false), (951, true), (669, false), (690, true)] as [(CGFloat, Bool)] {
      hosted.size(width: width, regular: true)
      try await hosted.until { (hosted.columns.viewController(for: .inspector) != nil) == stands }
      try await hosted.settle()
      #expect((hosted.columns.viewController(for: .inspector) != nil) == stands, "\(width)")
      #expect(hosted.columns.isShowing(.inspector) == stands, "\(width)")
      #expect(hosted.stack.asideIsOnScreen == stands, "\(width)")
    }
  }

  @Test func aNoticeIsSaidOnceByTheListingAloneOrOverTheMainViewAndByTheMainViewOtherwise() async throws {
    let hosted = try await HostedColumns(open: [open], wide: true, sessions: true, scenario: .releases, noticed: true)
    defer { hosted.close() }
    let columns = hosted.columns
    let stack = hosted.stack
    let notices = try #require(hosted.notices)
    let standing = { (how: String) in
      "\(how): \(stack.noticeSaid), shown \(notices.showing), \(columns.displayMode.rawValue)"
    }
    let saidOnce = { (said: NoticeSaid, how: String) in
      try await hosted.until({ stack.noticeSaid == said && notices.showing == 1 }, standing(how))
      try await hosted.settle()
      #expect(stack.noticeSaid == said, "\(standing(how))")
      #expect(notices.showing == 1, "\(standing(how))")
      #expect(notices.notice != nil, "\(standing(how))")
    }
    let saidNowhere = { (how: String) in
      try await hosted.until({ stack.sheetStands && notices.showing == 0 }, standing(how))
      try await hosted.settle()
      #expect(notices.showing == 0, "\(standing(how))")
      #expect(notices.notice != nil, "\(standing(how))")
    }
    try await saidOnce(.mainView, "by the Session beside the Sidebar")
    stack.aside.shown = true
    try await hosted.until { columns.displayMode == .secondaryOnly }
    try await saidOnce(.mainView, "by the Session, the Sidebar put away")
    columns.show(.primary)
    try await hosted.until { columns.displayMode == .oneOverSecondary }
    try await saidOnce(.listing, "by the listing over the Session")
    columns.show(.secondary)
    try await hosted.until { columns.displayMode == .secondaryOnly }
    try await saidOnce(.mainView, "by the Session, the Sidebar put away again")

    // A sheet the listing presents stands over the main view too, which says nothing beneath it, and says it again as
    // the sheet goes, however the columns stand by then: put away, collapsed, or beside.
    stack.listingSheet = .servers
    try await saidNowhere("beneath the listing's sheet, the Sidebar put away")
    stack.listingSheet = nil
    try await saidOnce(.mainView, "by the Session once the listing's sheet goes, the Sidebar put away")
    stack.aside.shown = false
    try await hosted.until { columns.displayMode == .oneBesideSecondary }
    stack.listingSheet = .servers
    try await saidNowhere("beneath the listing's sheet beside the Sidebar")
    try await hosted.resize(wide: false)
    try await saidNowhere("beneath the listing's sheet, collapsed")
    stack.listingSheet = nil
    try await saidOnce(.oneColumn, "by the Session over the listing once the listing's sheet goes")
    try await hosted.resize(wide: true)
    try await saidOnce(.mainView, "by the Session beside the Sidebar again")

    // So does a sheet the main view presents, an Intervention's, wherever the listing stands.
    let waiting = SessionRoute(session: "workstation-key/3")
    stack.path = [.session(waiting)]
    try await hosted.until { stack.model(for: waiting).presented != nil }
    try await saidNowhere("beneath the Session's own sheet beside the Sidebar")
    stack.aside.shown = true
    try await hosted.until { columns.displayMode == .secondaryOnly }
    columns.show(.primary)
    try await hosted.until { columns.displayMode == .oneOverSecondary }
    try await saidNowhere("beneath the Session's own sheet, the listing over it")
    stack.model(for: waiting).dismissInterventions()
    try await saidOnce(.listing, "by the listing over the Session once its sheet goes")
    columns.show(.secondary)
    try await hosted.until { columns.displayMode == .secondaryOnly }
    stack.aside.shown = false
    stack.path = []
    try await saidOnce(.mainView, "by the screen at rest beside the Sidebar")

    // With one column alone, whichever screen stands in it.
    try await hosted.resize(wide: false)
    try await saidOnce(.oneColumn, "by the listing alone")
    stack.path = [open]
    try await saidOnce(.oneColumn, "by the Session over the listing")
    stack.path = []
    try await saidOnce(.oneColumn, "by the listing returned to")
  }

  @Test func touchingTheFieldOfTheScreenAtRestOpensItWhereItStands() async throws {
    let hosted = try await HostedColumns(wide: true)
    defer { hosted.close() }
    let resting = try #require(hosted.main.viewControllers.first as? HostedScreen)
    let route = hosted.stack.rest
    // At rest no change of size puts the cursor in it.
    for wide in [false, true] {
      try await hosted.resize(wide: wide)
      #expect(hosted.main.viewControllers == [resting])
      #expect(hosted.stack.path.isEmpty)
    }
    let field = try await hosted.field()
    #expect(!field.isFirstResponder)
    #expect(field.becomeFirstResponder())
    try await hosted.until { hosted.stack.path == [.landing(route)] }
    #expect(resting.named == .landing(route) && resting.atRest == nil)
    for wide in [false, true] {
      try await hosted.resize(wide: wide)
      #expect(hosted.main.viewControllers == [resting], "no screen is made afresh")
      #expect(try await hosted.field() === field)
      #expect(field.isFirstResponder, "nor is the keyboard put down")
      #expect(!hosted.listingAlone)
    }
    // Its keyboard put down, it stays open, and down.
    field.resignFirstResponder()
    for wide in [false, true, false] {
      try await hosted.resize(wide: wide)
      #expect(hosted.stack.path == [.landing(route)])
      #expect(!field.isFirstResponder)
      #expect(!hosted.listingAlone)
    }
    // Returned from, nothing is open, and another rests.
    hosted.sidebar.popViewController(animated: true)
    try await hosted.until { hosted.stack.path.isEmpty && hosted.listingAlone && hosted.held == [nil] }
    #expect((hosted.main.viewControllers.first as? HostedScreen)?.atRest == hosted.stack.rest)
    #expect(hosted.main.viewControllers != [resting])
  }

  @Test func composingGivesTheScreenAfresh() async throws {
    let hosted = try await HostedColumns(wide: true)
    defer { hosted.close() }
    let resting = hosted.main.viewControllers
    hosted.stack.compose()
    try await hosted.settle()
    #expect(hosted.held == hosted.stack.path.map(Optional.some))
    let composed = hosted.main.viewControllers
    #expect(composed != resting)
    hosted.stack.compose()
    try await hosted.settle()
    #expect(hosted.main.viewControllers != composed, "and again over itself")
    try await hosted.resize(wide: false)
    #expect(!hosted.listingAlone, "it is open")
  }

  @Test func theListingsFootStandsWhileTheListingDoes() async throws {
    let hosted = try await HostedColumns()
    defer { hosted.close() }
    #expect(!hosted.sidebar.isToolbarHidden)
    hosted.stack.path = [open]
    try await hosted.settle()
    #expect(hosted.sidebar.isToolbarHidden)
    hosted.stack.path = []
    try await hosted.settle()
    #expect(!hosted.sidebar.isToolbarHidden)
    hosted.stack.path = [open]
    try await hosted.settle()
    try await hosted.resize(wide: true)
    #expect(!hosted.sidebar.isToolbarHidden, "beside the main view")
    try await hosted.resize(wide: false)
    #expect(hosted.stack.path == [open])
    #expect(hosted.sidebar.isToolbarHidden, "the session over the listing again")
  }
}
