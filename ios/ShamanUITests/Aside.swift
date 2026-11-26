import XCTest

/// What stands where the Aside may stand, as the tests of the columns look for it on a turned iPhone 18 Pro Max, and the
/// acts that change what it holds.
extension TurningTestCase {
  /// Waits until the Aside stands beside the main view under a bar reading `title`, with Back in it where it `pushed`
  /// and nothing else, and the Sidebar behind its button.
  @MainActor
  func asideStands(
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
  func noAsideStands(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    columnsStand(app, file: file, line: line)
    waitForOne("session.aside", "label == 'Show aside'", in: app, file: file, line: line)
    XCTAssertFalse(element("work.list", in: app).exists, file: file, line: line)
    XCTAssertFalse(element("tree.row", in: app).exists, file: file, line: line)
  }

  /// Whether the Sidebar stands where a tap reaches it.
  @MainActor
  func sidebarStands(_ app: XCUIApplication) -> Bool {
    let scope = app.buttons["listing.scope"]
    return scope.exists && scope.isHittable
  }

  /// The card of the Turn whose line reads `line`, where a tap reaches it.
  @MainActor @discardableResult
  func card(_ line: String, in app: XCUIApplication) -> XCUIElement {
    app.scrolled(to: element("session.turn", holding: line, in: app), over: element("session.conversation", in: app))
  }

  /// Sends `text` to the open Session, at rest, which begins a Turn a moment later, as its bar says by offering Stop, and
  /// puts the keyboard down where one is up, as it leaves the conversation no room for a card.
  @MainActor
  func beginTurn(_ text: String, in app: XCUIApplication) {
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
  func stopTurn(in app: XCUIApplication) {
    app.buttons["session.stop"].tap()
    XCTAssertTrue(app.buttons["session.stop"].waitForNonExistence(timeout: 10), "stopped")
  }
}
