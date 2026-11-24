import XCTest

/// A Turn's work and each Activity's whole over the fake: the card pushes the Turn's Activities in order, the Agent's
/// Messages among them, and a row pushes all of its Activity, a running command's output growing while it is up and
/// scrolling sideways.
final class WorkTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String, opening session: String) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--open", session]
    app.launch()
    return app
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

  /// Taps the card of the conversation's last Turn.
  @MainActor
  private func openLastCard(in app: XCUIApplication) {
    let cards = elements("session.turn", in: app)
    XCTAssertTrue(cards.firstMatch.waitForExistence(timeout: 10))
    cards.element(boundBy: cards.count - 1).tap()
  }

  @MainActor
  func testASettledTurnsCardPushesItsWorkInOrderAndARowPushesAllOfItsActivity() {
    let app = launched("paired", opening: "workstation-key/7")
    openLastCard(in: app)
    XCTAssertTrue(app.navigationBars["Worked for 3m 12s"].waitForExistence(timeout: 5))
    let rows = elements("work.row", in: app)
    XCTAssertTrue(rows.firstMatch.waitForExistence(timeout: 5))
    XCTAssertTrue(
      rows.element(boundBy: 0).label.contains("Read src/tui/serve_panel.rs"), rows.element(boundBy: 0).label)
    XCTAssertTrue(
      element("work.agentMessage", holding: "half blocks", in: app).exists, "the Agent's Message among the work")
    XCTAssertTrue(element("work.row", holding: "Edited src/tui/serve_panel.rs", in: app).exists)
    XCTAssertTrue(element("work.row", holding: "and 1 more file", in: app).exists)
    XCTAssertTrue(
      element("work.row", holding: "Approval · Command · Accepted once", in: app).exists,
      "a settled Approval reads as the TUI's row")
    XCTAssertTrue(element("work.row", holding: "Not shown: web search, from a newer Shaman", in: app).exists)
    attach("A settled turn's work")

    // A File Change names its paths, and nothing of what changed in them.
    element("work.row", holding: "Edited src/tui/serve_panel.rs", in: app).tap()
    XCTAssertTrue(app.navigationBars["File change"].waitForExistence(timeout: 5))
    XCTAssertTrue(element("detail.fact", holding: "src/tui/qr.rs", in: app).exists)
    XCTAssertFalse(elements("detail.monospace", in: app).firstMatch.exists, "no diff")
    attach("A file change's paths")
    app.back.tap()

    // A settled Approval opens onto what was asked and what was answered.
    element("work.row", holding: "Approval · Command", in: app).tap()
    XCTAssertTrue(app.navigationBars["Approval"].waitForExistence(timeout: 5))
    XCTAssertTrue(element("detail.fact", holding: "cargo nextest run serve_panel", in: app).exists)
    XCTAssertTrue(element("detail.fact", holding: "Runs the tests", in: app).exists)
    XCTAssertTrue(element("detail.fact", holding: "Accepted once", in: app).exists)
    attach("A settled approval, what was asked and answered")
    app.back.tap()

    // The placeholder says all there is, so it pushes nothing.
    element("work.row", holding: "Not shown", in: app).tap()
    XCTAssertTrue(app.navigationBars["Worked for 3m 12s"].exists, "still on the Turn's work")

    element("work.row", holding: "cargo check", in: app).tap()
    XCTAssertTrue(app.navigationBars["Command"].waitForExistence(timeout: 5))
    XCTAssertTrue(element("detail.monospace", holding: "Finished `dev` profile", in: app).exists)
    attach("A command's output")

    app.back.tap()
    XCTAssertTrue(app.navigationBars["Worked for 3m 12s"].waitForExistence(timeout: 5))
    app.back.tap()
    XCTAssertTrue(elements("session.turn", in: app).firstMatch.waitForExistence(timeout: 5), "back at the session")
  }

  @MainActor
  func testARunningCommandsOutputGrowsWhileItsScreenIsUpAndScrollsSideways() {
    let app = launched("paired", opening: "workstation-key/1")
    openLastCard(in: app)
    XCTAssertTrue(app.navigationBars["Working"].waitForExistence(timeout: 5))
    let running = element("work.row", holding: "cargo nextest run qr", in: app)
    XCTAssertTrue(running.waitForExistence(timeout: 5))
    attach("A working turn's work")
    running.tap()

    XCTAssertTrue(app.navigationBars["Command"].waitForExistence(timeout: 5))
    XCTAssertTrue(element("detail.fact", holding: "Running", in: app).exists)
    let output = elements("detail.monospace", in: app).firstMatch
    XCTAssertTrue(output.waitForExistence(timeout: 5))
    // What it has written by now, which depends on how long the screen took to reach, and more than that.
    let written = output.label
    let outputs = elements("detail.monospace", in: app)
    XCTAssertTrue(
      outputs.waitForOne(NSPredicate(format: "label BEGINSWITH %@ AND label != %@", written, written)), "it grows")
    XCTAssertTrue(
      outputs.waitForOne(NSPredicate(format: "label CONTAINS %@", "the_quiet_zone_stands_on_every_side")),
      "to its widest line")
    attach("A running command's output, grown")

    // Its lines are kept whole, wider than the screen, so the output scrolls sideways. Dragged by its first lines, as
    // it may have grown past the foot of the screen.
    let before = output.frame.minX
    let first = elements("detail.monospaceScroll", in: app).firstMatch
      .coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0)).withOffset(CGVector(dx: 0, dy: 20))
    first.press(forDuration: 0.05, thenDragTo: first.withOffset(CGVector(dx: -200, dy: 0)))
    XCTAssertLessThan(output.frame.minX, before, "dragged sideways, it moved")
    attach("A running command's output scrolled sideways")
  }

  @MainActor
  func testACompactionPushesItsSummary() {
    let app = launched("paired", opening: "workstation-key/7")
    let compaction = elements("session.compaction", in: app).firstMatch
    XCTAssertTrue(compaction.waitForExistence(timeout: 10))
    compaction.tap()
    XCTAssertTrue(app.navigationBars["Compaction"].waitForExistence(timeout: 5))
    XCTAssertTrue(element("detail.prose", holding: "We drew the invite", in: app).exists)
    attach("A compaction's summary")
  }
}
