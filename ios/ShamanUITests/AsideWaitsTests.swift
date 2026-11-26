import XCTest

/// The Aside through a collapse, on an iPhone 18 Pro Max turned mid-test: where its room goes the conversation shows
/// and the Aside waits, its value changing by the same rules as on screen, and as room comes again it returns holding
/// what it would have held. Nothing moves between the main view's stack and the Aside, either way.
final class AsideWaitsTests: TurningTestCase {
  /// The Session in `paired` with two Turns and no tree.
  private let tidy = "workstation-key/7"
  private let tidyTitle = "Tidy zsh startup time"
  /// The Session in `paired` whose Agent delegated to Subagents, and so has a tree.
  private let delegating = "workstation-key/12"
  private let delegatingTitle = "Find what holds the session lock"

  /// Waits until one column stands showing the open Session, with nothing of the Aside on screen, as a sheet or
  /// otherwise, and no button for it.
  @MainActor
  private func asideWaits(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    oneColumnStands(app, showing: "session.title", file: file, line: line)
    XCTAssertTrue(app.buttons["session.aside"].waitForNonExistence(timeout: 10), "no button", file: file, line: line)
    for part in ["work.list", "work.empty", "detail.screen", "tree.list", "aside.work"] {
      XCTAssertFalse(element(part, in: app).exists, "no \(part)", file: file, line: line)
    }
    XCTAssertFalse(app.otherElements["tree.sheet"].exists, "no sheet", file: file, line: line)
  }

  /// Waits until a Turn's work reading `title` stands pushed in the main view, with its Back, and the Sidebar beside it
  /// where `beside`, or the Aside holding the work reading `aside` beside it.
  @MainActor
  private func workStandsPushed(
    _ app: XCUIApplication, at title: String, beside: Bool = false, aside: String? = nil,
    file: StaticString = #filePath, line: UInt = #line
  ) {
    let bar = app.navigationBars[title]
    let scope = app.buttons["listing.scope"]
    let besides = aside.map { app.navigationBars[$0] }
    // The main view's bar reaches beneath the Sidebar, so where each stands is read from its Back.
    let back = bar.buttons["BackButton"]
    for _ in 0..<40 {
      if bar.exists, back.exists, back.isHittable, !app.staticTexts["session.title"].exists {
        if beside, sidebarStands(app), scope.frame.maxX < back.frame.minX, elements("work.list", in: app).count == 1 {
          return
        }
        if let besides, besides.exists, back.frame.maxX <= besides.frame.minX, !sidebarStands(app) { return }
        if !beside, besides == nil, !scope.exists || !scope.isHittable { return }
      }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail("\(title)'s work stands pushed", file: file, line: line)
  }

  @MainActor
  func testWhereItsRoomGoesTheConversationShowsAndTheAsideReturnsAsItWas() {
    let app = launched("paired", ["--open", tidy, "--aside", "work"], turned: true)
    titled(tidyTitle, in: app)
    asideStands(app, at: "Worked for 3m 12s")
    // Held at an earlier Turn, with an Activity's whole pushed over its work.
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    app.scrolled(to: element("session.compaction", in: app), over: element("session.conversation", in: app)).tap()
    asideStands(app, at: "Compaction", pushed: true)
    for side in [UIDeviceOrientation.landscapeRight, .landscapeLeft] {
      turn(.portrait, app)
      asideWaits(app)
      attach("The conversation, the Aside waiting")
      turn(side, app)
      asideStands(app, at: "Compaction", pushed: true)
    }
    attach("The Aside returned, an Activity's whole still pushed")
    app.navigationBars["Compaction"].buttons["BackButton"].tap()
    asideStands(app, at: "Worked for 1m 23s")
    XCTAssertFalse(app.back.exists, "nothing was pushed in the main view")
  }

  @MainActor
  func testWhileItWaitsAnotherSessionOpenedGivesThatSessionsWorkOrTree() {
    let app = launched("paired", ["--open", tidy, "--aside", "work"], turned: true)
    titled(tidyTitle, in: app)
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    turn(.portrait, app)
    asideWaits(app)
    app.back.tap()
    oneColumnStands(app, showing: "listing.scope")
    open(delegatingTitle, in: app)
    turn(.landscapeLeft, app)
    // Its latest Turn's work, following.
    asideStands(app, at: "Working")
    app.buttons["session.tree"].tap()
    asideStands(app, at: "Subagents")
    turn(.portrait, app)
    asideWaits(app)
    app.back.tap()
    oneColumnStands(app, showing: "listing.scope")
    open(tidyTitle, in: app)
    turn(.landscapeLeft, app)
    // Its work, as it has no tree, and at its latest Turn, opened afresh.
    asideStands(app, at: "Worked for 3m 12s")
    turn(.portrait, app)
    asideWaits(app)
    app.back.tap()
    oneColumnStands(app, showing: "listing.scope")
    open(delegatingTitle, in: app)
    turn(.landscapeLeft, app)
    // The kind stayed the tree.
    asideStands(app, at: "Subagents")
  }

  @MainActor
  func testWhileItWaitsANewTurnGivesTheNewTurnWhereItFollowed() {
    let app = launched("paired", ["--open", tidy, "--aside", "work"], turned: true)
    titled(tidyTitle, in: app)
    asideStands(app, at: "Worked for 3m 12s")
    // An Activity's whole pushed while it follows holds it at that Turn, so a new Turn changes nothing while it waits.
    app.scrolled(to: element("work.row", holding: "cargo check", in: app), over: element("work.list", in: app)).tap()
    asideStands(app, at: "Command", pushed: true)
    turn(.portrait, app)
    asideWaits(app)
    beginTurn("Now the other shell", in: app)
    turn(.landscapeLeft, app)
    asideStands(app, at: "Command", pushed: true)
    app.navigationBars["Command"].buttons["BackButton"].tap()
    asideStands(app, at: "Worked for 3m 12s")
    // Following again, it gives the Turn begun while it waits.
    card("Working", in: app).tap()
    asideStands(app, at: "Working")
    stopTurn(in: app)
    turn(.portrait, app)
    asideWaits(app)
    beginTurn("And fish", in: app)
    turn(.landscapeLeft, app)
    asideStands(app, at: "Working")
  }

  @MainActor
  func testATurnsWorkPushedWithNoRoomStaysPushedAndTheAsideComesOnlyWhereItWasWanted() {
    let app = launched("paired", ["--open", tidy])
    titled(tidyTitle, in: app)
    card("Worked for 1m 23s", in: app).tap()
    workStandsPushed(app, at: "Worked for 1m 23s")
    turn(.landscapeLeft, app)
    // Still pushed in the main view, beside the Sidebar, and the Aside not shown.
    workStandsPushed(app, at: "Worked for 1m 23s", beside: true)
    attach("A Turn's work pushed with no room, turned")
    app.navigationBars["Worked for 1m 23s"].buttons["BackButton"].tap()
    titled(tidyTitle, in: app)
    noAsideStands(app)

    // Wanted, it comes beside the work pushed while it waited, holding what it held.
    app.buttons["session.aside"].tap()
    asideStands(app, at: "Worked for 3m 12s")
    turn(.portrait, app)
    asideWaits(app)
    card("Worked for 1m 23s", in: app).tap()
    workStandsPushed(app, at: "Worked for 1m 23s")
    turn(.landscapeLeft, app)
    workStandsPushed(app, at: "Worked for 1m 23s", aside: "Worked for 3m 12s")
    attach("A Turn's work pushed with no room, the Aside beside it")
    app.navigationBars["Worked for 1m 23s"].buttons["BackButton"].tap()
    titled(tidyTitle, in: app)
    asideStands(app, at: "Worked for 3m 12s")
  }

  @MainActor
  func testTheTreesSheetStaysASheetAsRoomArrivesAndAfterItTheButtonGivesTheAside() {
    let app = launched("paired", ["--open", delegating])
    titled(delegatingTitle, in: app)
    let tree = app.buttons["session.tree"]
    XCTAssertTrue(tree.waitForExistence(timeout: 10))
    tree.tap()
    let sheet = app.otherElements["tree.sheet"]
    let entry = element("tree.row", holding: delegatingTitle, in: app)
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    for held in [UIDeviceOrientation.landscapeLeft, .portrait, .landscapeRight] {
      turn(held, app)
      XCTAssertTrue(sheet.exists, "still a sheet")
      XCTAssertTrue(entry.waitForExistence(timeout: 10) && entry.isHittable)
      XCTAssertEqual(elements("tree.list", in: app).count, 1, "and the tree nowhere else")
    }
    attach("The tree's sheet as room arrived")
    // The open Session's entry puts it away.
    app.scrolled(to: entry, over: sheet).tap()
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
    columnsStand(app)
    tree.tap()
    asideStands(app, at: "Subagents")
    XCTAssertFalse(sheet.exists)
    // Where room goes, no sheet stands for the tree in its place.
    for side in [UIDeviceOrientation.landscapeLeft, .landscapeRight] {
      turn(.portrait, app)
      asideWaits(app)
      turn(side, app)
      asideStands(app, at: "Subagents")
      XCTAssertFalse(sheet.exists)
    }
  }
}
