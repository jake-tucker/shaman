import XCTest

/// A Session's Subagents over the fake: a Subagent's row among a Turn's work pushes its own Session, to any depth, with
/// no composer; the button in the navigation bar opens the tree as a sheet that switches between its Sessions; and a
/// working Subagent is stopped from its bar or by a swipe in the tree, wherever it can be stopped alone.
final class SubagentTests: XCTestCase {
  /// The Session in `paired` whose Agent only waits on the Subagents it delegated to.
  private let delegating = "workstation-key/12"

  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String = "paired", opening session: String) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--open", session]
    app.launch()
    return app
  }

  @MainActor
  private func attach(_ app: XCUIApplication, _ name: String) {
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = name
    screenshot.lifetime = .keepAlways
    add(screenshot)
  }

  @MainActor
  private func elements(_ identifier: String, in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: identifier)
  }

  /// The element `identifier` names whose label holds `text`.
  @MainActor
  private func element(_ identifier: String, holding text: String, in app: XCUIApplication) -> XCUIElement {
    elements(identifier, in: app).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
  }

  @MainActor
  private func title(_ app: XCUIApplication) -> String {
    app.staticTexts["session.title"].label
  }

  /// Waits until the open Session's Title reads `expected`.
  @MainActor
  private func titled(_ expected: String, in app: XCUIApplication) {
    let title = app.staticTexts.matching(identifier: "session.title")
      .matching(NSPredicate(format: "label == %@", expected)).firstMatch
    XCTAssertTrue(title.waitForExistence(timeout: 10), "the Session titled \(expected) shows")
  }

  /// The cell of the tree's sheet whose entry holds `text`, which a swipe acts on.
  @MainActor
  private func cell(holding text: String, in app: XCUIApplication) -> XCUIElement {
    app.cells.containing(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
  }

  /// Waits for the open Session's conversation to land, so the screen has settled from being pushed.
  @MainActor
  private func landed(_ app: XCUIApplication) {
    XCTAssertTrue(elements("session.turn", in: app).firstMatch.waitForExistence(timeout: 10))
  }

  /// Opens the tree's sheet from the navigation bar, and waits for it to land.
  @MainActor
  private func openTree(in app: XCUIApplication) {
    landed(app)
    let button = app.buttons["session.tree"]
    XCTAssertTrue(button.waitForExistence(timeout: 10))
    button.tap()
    XCTAssertTrue(
      element("tree.row", holding: "Find what holds the session lock", in: app).waitForExistence(timeout: 10))
  }

  @MainActor
  func testASubagentsRowPushesItsSessionToAnyDepthAndBackReturnsToTheListItWasTappedIn() {
    let app = launched(opening: delegating)
    XCTAssertTrue(
      element("session.barWords", holding: "Waiting on 2 subagents", in: app).waitForExistence(timeout: 10),
      "the parent's bar names what its Agent waits on")
    let card = elements("session.turn", in: app).firstMatch
    XCTAssertTrue(card.waitForExistence(timeout: 10))
    card.tap()

    // The row: its name over what it was asked, its outcome and duration at the right.
    let writer = element("work.row", holding: "Writer", in: app)
    XCTAssertTrue(writer.waitForExistence(timeout: 10))
    XCTAssertTrue(writer.label.contains("Draft the fix"), writer.label)
    XCTAssertTrue(writer.label.contains("Failed · 12s"), writer.label)
    attach(app, "A turn's work with its subagents' rows")
    element("work.row", holding: "Explorer", in: app).tap()

    // The Subagent's Session: its Title over its name and Model, its Delegation at the left, and no composer.
    titled("Find what held the lock", in: app)
    XCTAssertEqual(app.staticTexts["session.place"].label, "Explorer · haiku")
    let delegation = element("session.delegation", holding: "Find what held the session lock", in: app)
    XCTAssertTrue(delegation.waitForExistence(timeout: 10))
    XCTAssertTrue(delegation.label.contains("The delegating agent"), delegation.label)
    XCTAssertFalse(app.textViews["session.composer"].exists)
    XCTAssertFalse(app.textFields["session.composer"].exists, "a subagent's session takes no prompt")
    XCTAssertTrue(app.buttons["session.stop"].exists, "its bar stands alone, with Stop")
    attach(app, "A subagent's session")

    // Its own Subagent pushes over it the same way.
    elements("session.turn", in: app).firstMatch.tap()
    let grepper = element("work.row", holding: "Grepper", in: app)
    XCTAssertTrue(grepper.waitForExistence(timeout: 10))
    grepper.tap()
    titled("Grep the logs for the lock", in: app)
    XCTAssertTrue(app.otherElements["session.bar"].waitForExistence(timeout: 10))
    XCTAssertFalse(app.buttons["session.stop"].exists, "one that cannot be stopped alone offers no Stop")
    attach(app, "A subagent that cannot be stopped alone")

    // Back returns to the list it was tapped in.
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertTrue(element("work.row", holding: "Grepper", in: app).waitForExistence(timeout: 10))
  }

  @MainActor
  func testTheTreesSheetSwitchesBetweenItsSessionsPushingPoppingOrStayingPut() {
    let app = launched(opening: delegating)
    titled("Find what holds the session lock", in: app)
    let button = app.buttons["session.tree"]
    XCTAssertTrue(button.waitForExistence(timeout: 10))
    button.tap()
    let top = element("tree.row", holding: "Find what holds the session lock", in: app)
    XCTAssertTrue(top.waitForExistence(timeout: 10))
    XCTAssertTrue(button.label.contains("3 working"), "counted once read: \(button.label)")
    XCTAssertTrue(top.isSelected, "the open session is highlighted")
    let rows = elements("tree.row", in: app)
    XCTAssertEqual(rows.count, 7)
    let order = (0..<rows.count).map { rows.element(boundBy: $0).label }
    XCTAssertTrue(order[1].contains("Run the lock's tests"), order[1])
    XCTAssertTrue(order[2].contains("Find what held the lock"), order[2])
    XCTAssertTrue(order[3].contains("Grep the logs for the lock"), order[3])
    XCTAssertTrue(order[4].contains("monitoring"), "a settled subagent watching reads monitoring: \(order[4])")
    XCTAssertEqual(app.staticTexts["tree.count"].label, "6 · 3 working")
    attach(app, "The tree as a sheet")

    // Chosen, an entry not on the stack is pushed.
    element("tree.row", holding: "Grep the logs for the lock", in: app).tap()
    titled("Grep the logs for the lock", in: app)

    // The one open does nothing more than close the sheet.
    openTree(in: app)
    XCTAssertTrue(element("tree.row", holding: "Grep the logs for the lock", in: app).isSelected)
    element("tree.row", holding: "Grep the logs for the lock", in: app).tap()
    XCTAssertFalse(app.otherElements["tree.sheet"].waitForExistence(timeout: 2))
    titled("Grep the logs for the lock", in: app)

    // One on the stack is popped back to.
    openTree(in: app)
    element("tree.row", holding: "Find what holds the session lock", in: app).tap()
    titled("Find what holds the session lock", in: app)
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertTrue(app.navigationBars["All workspaces"].waitForExistence(timeout: 10), "nothing was left above it")
  }

  @MainActor
  func testAWorkingSubagentIsStoppedByASwipeInTheTreeOrFromItsBarAskingNothing() {
    let app = launched(opening: delegating)
    openTree(in: app)

    cell(holding: "Run the lock's tests", in: app).swipeLeft()
    XCTAssertTrue(app.buttons["tree.stop"].waitForExistence(timeout: 5))
    app.buttons["tree.stop"].tap()
    let stopped = element("tree.row", holding: "haiku · Stopped", in: app)
    XCTAssertTrue(stopped.waitForExistence(timeout: 10), "stopped, without asking first")
    XCTAssertEqual(app.staticTexts["tree.count"].label, "6 · 2 working")
    attach(app, "A subagent stopped by a swipe")

    // From its own bar: Explorer, and what it delegated with it.
    element("tree.row", holding: "Find what held the lock", in: app).tap()
    titled("Find what held the lock", in: app)
    let stop = app.buttons["session.stop"]
    XCTAssertTrue(stop.waitForExistence(timeout: 10))
    stop.tap()
    XCTAssertTrue(app.otherElements["session.bar"].waitForNonExistence(timeout: 10), "its work stopped")
    openTree(in: app)
    XCTAssertTrue(element("tree.row", holding: "Grep the logs for the lock", in: app).label.contains("Stopped"))
    XCTAssertFalse(app.staticTexts["tree.count"].label.contains("working"))
  }

  @MainActor
  func testAStopRefusedInTheTreeIsSaidOnceInTheSheetAndNotBeneathIt() {
    let app = launched("subagentStopRefused", opening: delegating)
    openTree(in: app)
    cell(holding: "Run the lock's tests", in: app).swipeLeft()
    XCTAssertTrue(app.buttons["tree.stop"].waitForExistence(timeout: 5))
    app.buttons["tree.stop"].tap()
    let banners = elements("banner", in: app)
    XCTAssertTrue(banners.firstMatch.waitForExistence(timeout: 10), "the refusal is said")
    XCTAssertTrue(banners.firstMatch.label.contains("Could not stop the subagent"), banners.firstMatch.label)
    XCTAssertEqual(banners.count, 1, "in the sheet alone")
    attach(app, "A stop refused in the tree")
  }

  @MainActor
  func testATreeNotInHandIsOfferedSayingLoadingAndOneThatCannotBeReadSaysWhy() {
    for (scenario, said, shown) in [
      ("unreadTree", "tree.loading", "A tree not yet in hand"),
      ("unreadableTree", "tree.failed", "A tree that cannot be read"),
    ] {
      let app = launched(scenario, opening: delegating)
      landed(app)
      // Offered as the session shows it has subagents, though they cannot be counted.
      let button = app.buttons["session.tree"]
      XCTAssertTrue(button.waitForExistence(timeout: 10))
      XCTAssertEqual(button.label, "Subagents")
      button.tap()
      XCTAssertTrue(elements(said, in: app).firstMatch.waitForExistence(timeout: 10), scenario)
      XCTAssertFalse(elements("tree.row", in: app).firstMatch.exists)
      attach(app, shown)
      app.terminate()
    }
  }
}
