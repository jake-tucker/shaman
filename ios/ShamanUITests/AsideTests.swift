import XCTest

/// The Aside beside the main view, on a turned iPhone 18 Pro Max: what shows it and what it shows, that its own button
/// alone takes it down, that while it is shown it stands in the Sidebar's place, and what becomes of what it holds as
/// the Session or its latest Turn changes.
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

  /// Opens the Session the listing's row `title` names from the Sidebar, called back over the Aside.
  @MainActor
  private func openFromTheSidebar(_ title: String, in app: XCUIApplication) {
    app.buttons["Show Sidebar"].firstMatch.tap()
    open(title, in: app)
  }

  /// Sends `text` to the open Session, at rest, which begins a Turn a moment later, as its bar says by offering Stop, and
  /// puts the keyboard down where one is up, as it leaves the conversation no room for a card.
  @MainActor
  private func beginTurn(_ text: String, in app: XCUIApplication) {
    let composer = element("session.composer", in: app)
    composer.tap()
    composer.typeText(text)
    app.buttons["session.send"].tap()
    XCTAssertTrue(app.buttons["session.stop"].waitForExistence(timeout: 20), "a Turn began")
    let keyboard = app.keyboards.firstMatch
    guard keyboard.exists, keyboard.frame.minY < app.window.frame.maxY - 100 else { return }
    // From the foot of the conversation, above the bar, down over the keyboard.
    element("session.bar", in: app).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0))
      .withOffset(CGVector(dx: 0, dy: -20))
      .press(forDuration: 0.05, thenDragTo: app.point(across: 0.3, down: 0.98))
    XCTAssertTrue(keyboard.waitForNonExistence(timeout: 10), "the keyboard is put down")
  }

  /// Stops the open Session's Turn.
  @MainActor
  private func stopTurn(in app: XCUIApplication) {
    app.buttons["session.stop"].tap()
    XCTAssertTrue(app.buttons["session.stop"].waitForNonExistence(timeout: 10), "stopped")
  }

  /// Waits until the Aside stands saying `words` where its work would be, under a bar that says nothing.
  @MainActor
  private func asideSays(
    _ words: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    waitForOne("work.empty", "label == '\(words)'", in: app, file: file, line: line)
    waitForOne("session.aside", "label == 'Hide aside'", in: app, file: file, line: line)
    XCTAssertLessThanOrEqual(
      element("session.conversation", in: app).frame.maxX, element("work.empty", in: app).frame.minX, file: file,
      line: line)
    XCTAssertFalse(sidebarStands(app), file: file, line: line)
  }

  @MainActor
  func testTheKindCarriesFromOneSessionToTheNextAndASessionWithNoTreeShowsItsWork() {
    let invite = "Draw the invite as a QR code in /serve"
    let app = launched("paired", ["--open", delegating, "--aside", "work"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Working")
    app.buttons["session.tree"].tap()
    asideStands(app, at: "Subagents")

    // One with no tree.
    openFromTheSidebar(invite, in: app)
    asideStands(app, at: "Working")
    XCTAssertTrue(element("work.row", holding: "cargo nextest run qr", in: app).waitForExistence(timeout: 10))
    XCTAssertFalse(element("tree.row", in: app).exists)
    attach("A session with no tree, the kind still the tree")
    openFromTheSidebar(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    waitForOne("tree.row", "label CONTAINS '\(delegatingTitle)' AND selected == true", in: app)

    // A card turns the kind to work, which carries in its turn, and while no Aside is on screen.
    element("session.turn", in: app).tap()
    asideStands(app, at: "Working")
    openFromTheSidebar(invite, in: app)
    asideStands(app, at: "Working")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
    open(delegatingTitle, in: app)
    app.buttons["session.aside"].tap()
    asideStands(app, at: "Working")
    XCTAssertTrue(element("work.row", holding: "Explorer", in: app).waitForExistence(timeout: 10))
    app.buttons["session.tree"].tap()
    asideStands(app, at: "Subagents")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
    open(invite, in: app)
    open(delegatingTitle, in: app)
    app.buttons["session.aside"].tap()
    asideStands(app, at: "Subagents")
  }

  @MainActor
  func testWithinOneTreeTheTreeStandsAndOnlyItsHighlightMoves() {
    let app = launched("paired", ["--open", delegating, "--aside", "tree"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    let last = elements("tree.row", in: app)
    XCTAssertTrue(last.firstMatch.waitForExistence(timeout: 10))
    // Scrolled a little from its head, where a tree drawn afresh would stand.
    let list = element("tree.list", in: app)
    let point = { (down: Double) in list.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: down)) }
    point(0.8).press(forDuration: 0.05, thenDragTo: point(0.55), withVelocity: .slow, thenHoldForDuration: 0.3)
    let explorer = element("tree.row", holding: "Find what held the lock", in: app)
    XCTAssertTrue(explorer.isHittable)
    let stood = explorer.frame
    explorer.tap()
    titled("Find what held the lock", in: app)
    waitForOne("tree.row", "label CONTAINS 'Find what held the lock' AND selected == true", in: app)
    XCTAssertEqual(explorer.frame, stood, "where it stood")
    attach("The same tree, scrolled, its highlight moved")
    app.back.tap()
    titled(delegatingTitle, in: app)
    waitForOne("tree.row", "label CONTAINS '\(delegatingTitle)' AND selected == true", in: app)
    XCTAssertEqual(explorer.frame, stood)
  }

  @MainActor
  func testItFollowsTheLatestTurnAndHoldsAtAnEarlierOneUntilTheLatestIsAskedFor() {
    let app = launched("paired", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    beginTurn("Now the other shell", in: app)
    asideStands(app, at: "Working")
    waitForOne("work.empty", "label == 'Nothing done yet'", in: app)
    attach("The Aside at a Turn just begun")
    stopTurn(in: app)
    asideStands(app, at: "Stopped after 4m 12s")

    // Held at an earlier one as another begins.
    card("Worked for 3m 12s", in: app).tap()
    asideStands(app, at: "Worked for 3m 12s")
    beginTurn("And fish", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    XCTAssertTrue(element("work.row", holding: "Read src/tui/serve_panel.rs", in: app).exists)
    attach("The Aside held at an earlier Turn")
    // The latest Turn's card resumes following.
    card("Working", in: app).tap()
    asideStands(app, at: "Working")
    stopTurn(in: app)
    asideStands(app, at: "Stopped after 4m 12s")
    beginTurn("And nushell", in: app)
    asideStands(app, at: "Working")

    // Held at an earlier one, another Session opened and this one again begins at its latest.
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    openFromTheSidebar("Draw the invite as a QR code in /serve", in: app)
    asideStands(app, at: "Working")
    openFromTheSidebar("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
  }

  @MainActor
  func testItFollowsAndHoldsByTheSameRulesWhileItIsOffScreen() {
    let app = launched("paired", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
    beginTurn("Now the other shell", in: app)
    app.buttons["session.aside"].tap()
    asideStands(app, at: "Working")
    stopTurn(in: app)
    asideStands(app, at: "Stopped after 4m 12s")
    // Held, with no room for it.
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    beginTurn("And fish", in: app)
    turn(.landscapeLeft, app)
    asideStands(app, at: "Worked for 1m 23s")
  }

  @MainActor
  func testWithAnActivityPushedANewTurnChangesNothingAndBackReturnsToItsTurn() {
    let app = launched("paired", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    let check = element("work.row", holding: "cargo check", in: app)
    app.scrolled(to: check, over: element("work.list", in: app)).tap()
    asideStands(app, at: "Command", pushed: true)
    beginTurn("Now the other shell", in: app)
    asideStands(app, at: "Command", pushed: true)
    attach("An Activity's whole as a Turn began")
    app.navigationBars["Command"].buttons["BackButton"].tap()
    asideStands(app, at: "Worked for 3m 12s")
    XCTAssertTrue(check.waitForExistence(timeout: 10))
    // Held there, as at any earlier Turn.
    stopTurn(in: app)
    beginTurn("And fish", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    card("Working", in: app).tap()
    asideStands(app, at: "Working")
  }

  @MainActor
  func testASessionWithNoTurnSaysSoAndFillsInPlaceAsTheFirstBegins() {
    // Working only for a Prompt not yet delivered, which a stop gives back.
    let app = launched("paired", ["--open", "workstation-key/11", "--aside", "work"], turned: true)
    titled("Note the floor in the pairing docs", in: app)
    asideSays("Nothing done yet", in: app)
    XCTAssertFalse(app.buttons["session.tree"].exists)
    attach("A session with no Turn, and the Aside")
    let words = element("work.empty", in: app).frame
    app.buttons["session.stop"].tap()
    waitForOne("session.composer", "value == 'Note the floor in the pairing docs'", in: app)
    asideSays("Nothing done yet", in: app)
    app.buttons["session.send"].tap()
    asideStands(app, at: "Working")
    waitForOne("work.empty", "label == 'Nothing done yet'", in: app)
    XCTAssertEqual(element("work.empty", in: app).frame, words, "in place")
  }

  @MainActor
  func testASessionThatCannotBeReadSaysWhyInIt() {
    let app = launched("releases", ["--open", "laptop-key/4", "--aside", "tree", "--notice-stands", "1"], turned: true)
    let unread = element("session.unread", in: app)
    XCTAssertTrue(unread.waitForExistence(timeout: 10))
    asideSays(unread.label, in: app)
  }

  @MainActor
  func testItIsAbsentWhileNoSessionIsOpenAndReturnsAsAFirstPromptIsSent() {
    let app = launched("composing", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    app.buttons["Show Sidebar"].firstMatch.tap()
    app.buttons["home.compose"].tap()
    columnsStand(app, title: "landing.title")
    let button = app.buttons["session.aside"]
    XCTAssertFalse(button.exists)
    XCTAssertFalse(element("work.list", in: app).exists)
    let composer = element("landing.composer", in: app)
    composer.tap()
    composer.typeText("Map the workspace")
    columnsStand(app, title: "landing.title")
    app.buttons["landing.send"].tap()
    // Over the Provisional Session, whose work is none yet, and then over the Session begun.
    titled("Map the workspace", in: app)
    asideSays("Nothing done yet", in: app)
    attach("The Aside over a Provisional Session")
    asideStands(app, at: "Working")
    attach("The Aside over the Session begun")
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
