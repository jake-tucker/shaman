import XCTest

/// The Aside beside the main view, on a turned iPhone 18 Pro Max: what shows it and what it shows, that its own button
/// alone takes it down, and that while it is shown it stands in the Sidebar's place.
final class AsideTests: TurningTestCase {
  /// The Session in `paired` whose Agent delegated to Subagents, and so has a tree.
  private let delegating = "workstation-key/12"
  private let delegatingTitle = "Find what holds the session lock"

  /// Waits until the Aside stands beside the main view under a bar reading `title`, with Back in it where it `pushed`
  /// and nothing else, and the Sidebar behind its button.
  @MainActor
  private func asideStands(
    _ app: XCUIApplication, at title: String, pushed: Bool = false, file: StaticString = #filePath, line: UInt = #line
  ) {
    let bar = app.navigationBars[title]
    let main = element("session.conversation", in: app)
    let button = app.buttons["session.aside"]
    for _ in 0..<40 {
      if bar.exists, main.exists, main.frame.maxX <= bar.frame.minX + 1, button.exists, button.label == "Hide aside",
        bar.buttons.count == (pushed ? 1 : 0), !sidebarStands(app)
      {
        if pushed {
          XCTAssertTrue(bar.buttons["BackButton"].isHittable, "its own Back", file: file, line: line)
        }
        return
      }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail("the Aside stands at \(title)", file: file, line: line)
  }

  /// Waits until no Aside stands and the Sidebar is back beside the main view.
  @MainActor
  private func noAsideStands(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    columnsStand(app, file: file, line: line)
    waitForOne("session.aside", "label == 'Show aside'", in: app, file: file, line: line)
    XCTAssertFalse(element("work.list", in: app).exists, file: file, line: line)
    XCTAssertFalse(element("tree.row", in: app).exists, file: file, line: line)
  }

  @MainActor
  private func sidebarStands(_ app: XCUIApplication) -> Bool {
    let scope = app.buttons["listing.scope"]
    return scope.exists && scope.isHittable
  }

  /// The card of the Turn whose line reads `line`, where a tap reaches it.
  @MainActor
  private func card(_ line: String, in app: XCUIApplication) -> XCUIElement {
    app.scrolled(to: element("session.turn", holding: line, in: app), over: element("session.conversation", in: app))
  }

  @MainActor
  func testItBeginsHiddenAndItsButtonShowsItAtTheLatestTurnsWorkAndTakesItDown() {
    let app = launched("paired", ["--open", "workstation-key/7"], turned: true)
    titled("Tidy zsh startup time", in: app)
    noAsideStands(app)
    let button = app.buttons["session.aside"]
    for _ in 0..<2 {
      button.tap()
      asideStands(app, at: "Worked for 3m 12s")
      XCTAssertTrue(element("work.row", holding: "Read src/tui/serve_panel.rs", in: app).isHittable)
      XCTAssertFalse(app.back.exists, "nothing is pushed")
      attach("The Aside beside a session")
      button.tap()
      noAsideStands(app)
    }
    // None while no Session is open.
    app.buttons["home.compose"].tap()
    columnsStand(app, title: "landing.title")
    XCTAssertFalse(button.exists)
    // And none in one column.
    open("Tidy zsh startup time", in: app)
    XCTAssertTrue(button.waitForExistence(timeout: 10))
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    XCTAssertTrue(button.waitForNonExistence(timeout: 10))
  }

  @MainActor
  func testATurnsCardShowsThatTurnsWorkAndAnActivityIsPushedWithinIt() {
    let app = launched("paired", ["--open", "workstation-key/7"], turned: true)
    titled("Tidy zsh startup time", in: app)
    columnsStand(app)
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    attach("An earlier Turn's work in the Aside")
    card("Worked for 3m 12s", in: app).tap()
    asideStands(app, at: "Worked for 3m 12s")

    // An Activity's whole, under the Aside's own Back, the main view standing as it was.
    let list = element("work.list", in: app)
    app.scrolled(to: element("work.row", holding: "cargo check", in: app), over: list).tap()
    asideStands(app, at: "Command", pushed: true)
    titled("Tidy zsh startup time", in: app)
    attach("An Activity's whole in the Aside")
    app.navigationBars["Command"].buttons["BackButton"].tap()
    asideStands(app, at: "Worked for 3m 12s")

    // A card tapped with one pushed shows its Turn's work, and a Compaction's whole is pushed there too.
    app.scrolled(to: element("work.row", holding: "cargo check", in: app), over: list).tap()
    asideStands(app, at: "Command", pushed: true)
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    app.scrolled(to: element("session.compaction", in: app), over: element("session.conversation", in: app)).tap()
    asideStands(app, at: "Compaction", pushed: true)
    app.navigationBars["Compaction"].buttons["BackButton"].tap()
    asideStands(app, at: "Worked for 1m 23s")
    XCTAssertFalse(app.back.exists, "nothing was pushed in the main view")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
  }

  @MainActor
  func testTheTreesButtonShowsTheTreeAndNeverTakesTheAsideDown() {
    let app = launched("paired", ["--open", delegating], turned: true)
    titled(delegatingTitle, in: app)
    columnsStand(app)
    let tree = app.buttons["session.tree"]
    XCTAssertTrue(tree.waitForExistence(timeout: 10))
    XCTAssertGreaterThan(app.buttons["session.aside"].frame.minX, tree.frame.minX, "after the tree's button")
    let entry = element("tree.row", holding: delegatingTitle, in: app)
    for _ in 0..<2 {
      tree.tap()
      asideStands(app, at: "Subagents")
      XCTAssertTrue(entry.isSelected, "the open Session's entry is highlighted")
      XCTAssertFalse(app.otherElements["tree.sheet"].exists)
    }
    attach("The tree in the Aside")
    // From work to the tree, and back.
    element("session.turn", in: app).tap()
    asideStands(app, at: "Working")
    tree.tap()
    asideStands(app, at: "Subagents")

    // An entry chosen is shown in the main view, beside the same tree.
    let explorer = element("tree.row", holding: "Find what held the lock", in: app)
    app.scrolled(to: explorer, over: element("tree.list", in: app)).tap()
    titled("Find what held the lock", in: app)
    asideStands(app, at: "Subagents")
    waitForOne("tree.row", "label CONTAINS 'Find what held the lock' AND selected == true", in: app)
    attach("A subagent's session beside the tree")
    app.back.tap()
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
  }

  @MainActor
  func testAReviewThatLeadsToTheTreeShowsItThere() {
    let app = launched("subagentsWait", ["--open", delegating], turned: true)
    titled(delegatingTitle, in: app)
    columnsStand(app)
    for _ in 0..<2 {
      let review = app.buttons["session.review"]
      XCTAssertTrue(review.waitForExistence(timeout: 10))
      review.tap()
      asideStands(app, at: "Subagents")
    }
    XCTAssertFalse(app.otherElements["tree.sheet"].exists)
  }

  @MainActor
  func testASubagentsRowInItPushesItsSessionInTheMainView() {
    let app = launched("paired", ["--open", delegating, "--aside", "work"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Working")
    element("work.row", holding: "Explorer", in: app).tap()
    titled("Find what held the lock", in: app)
    asideStands(app, at: "Working")
    XCTAssertTrue(element("work.row", holding: "Grepper", in: app).waitForExistence(timeout: 10), "its own work")
    attach("A subagent's session beside its work")
    app.back.tap()
    titled(delegatingTitle, in: app)
    XCTAssertTrue(element("work.row", holding: "Explorer", in: app).waitForExistence(timeout: 10))
    asideStands(app, at: "Working")
  }

  @MainActor
  func testTheSidebarCalledBackStandsOverBothAndASessionOpenedFromItTakesItsPlace() {
    let app = launched("paired", ["--open", delegating, "--aside", "work"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Working")
    let aside = element("work.list", in: app)
    let stood = aside.frame
    app.buttons["Show Sidebar"].firstMatch.tap()
    let scope = app.buttons["listing.scope"]
    XCTAssertTrue(scope.waitForExistence(timeout: 10))
    XCTAssertTrue(app.navigationBars["Working"].exists, "over the main view and the Aside, which stand beneath it")
    XCTAssertEqual(aside.frame, stood)
    XCTAssertLessThan(element("session.conversation", in: app).frame.minX, scope.frame.maxX)
    attach("The Sidebar over the main view and the Aside")

    element("listing.active", holding: "Draw the invite as a QR code in /serve", in: app).tap()
    titled("Draw the invite as a QR code in /serve", in: app)
    // Shown still, at that Session's work.
    asideStands(app, at: "Working")
    XCTAssertTrue(element("work.row", holding: "cargo nextest run qr", in: app).waitForExistence(timeout: 10))
    XCTAssertFalse(app.back.exists)
    attach("Another session, the Aside still shown")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
  }

  @MainActor
  func testAnInterventionPresentsItselfOverItAndItStandsAsItWas() {
    let app = launched(
      "subagentWaits", ["--open", delegating, "--aside", "tree", "--arming-moment", "0.3"], turned: true)
    let sheet = app.otherElements["approval.sheet"]
    XCTAssertTrue(sheet.waitForExistence(timeout: 10), "presented over the tree")
    attach("An Approval over the Aside")
    // At full height, as a short display has it, a sheet has no grabber.
    let top = app.staticTexts["approval.title"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    top.press(forDuration: 0.1, thenDragTo: top.withOffset(CGVector(dx: 0, dy: 360)))
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
    asideStands(app, at: "Subagents")
    XCTAssertTrue(element("tree.row", holding: delegatingTitle, in: app).isSelected)
    // And over a Subagent's Session and its work, where its own does, and is answered.
    element("session.turn", in: app).tap()
    XCTAssertTrue(element("work.row", holding: "Explorer", in: app).waitForExistence(timeout: 10))
    app.buttons["session.review"].tap()
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    waitForOne("approval.acceptOnce", "isEnabled == true", in: app)
    app.buttons["approval.acceptOnce"].tap()
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
    titled("Find what held the lock", in: app)
    XCTAssertTrue(element("work.row", holding: "Grepper", in: app).waitForExistence(timeout: 10))
    asideStands(app, at: "Working")
    attach("The Aside after an Approval")
  }

  @MainActor
  func testNoNoticeIsSaidInIt() {
    let app = launched(
      "releases", ["--open", "workstation-key/7", "--aside", "work", "--notice-stands", "60"], turned: true)
    let notices = elements("notice", in: app)
    XCTAssertTrue(notices.firstMatch.waitForExistence(timeout: 10))
    let bar = app.navigationBars["Worked for 3m 12s"]
    XCTAssertTrue(bar.waitForExistence(timeout: 10))
    let aside = bar.frame.minX
    for notice in notices.allElementsBoundByIndex {
      XCTAssertLessThanOrEqual(notice.frame.maxX, aside + 1, "\(notice)")
    }
    attach("A notice beside the Aside")
  }

  @MainActor
  func testItIsOfferedOnlyOnceTurned() {
    let app = launched("paired", ["--open", delegating, "--aside", "tree"])
    oneColumnStands(app, showing: "session.title")
    XCTAssertTrue(app.buttons["session.tree"].waitForExistence(timeout: 10))
    let button = app.buttons["session.aside"]
    XCTAssertFalse(button.exists, "upright one column stands")
    XCTAssertFalse(element("tree.row", in: app).exists)
    turn(.landscapeLeft, app)
    asideStands(app, at: "Subagents")
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    XCTAssertTrue(button.waitForNonExistence(timeout: 10))
  }

  @MainActor
  func testItIsShownAtLaunchAtItsTreeAndNeverWithoutASessionToOpen() {
    let app = launched("paired", ["--open", delegating, "--aside", "tree"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    app.terminate()
    let again = launched("paired", ["--aside", "tree"], turned: true)
    columnsStand(again, title: "landing.title")
    open("Draw the invite as a QR code in /serve", in: again)
    noAsideStands(again)
  }
}
