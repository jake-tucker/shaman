import XCTest

/// The Duo open flat, its inner display in portrait: run by `scripts/ios-test.sh posed portrait`.
///
/// There the system stands the main view alone. With a Session open that is what stands; with nothing open the Sidebar
/// is asked for over the dimmed new-session screen, as the app launches and as the path empties, the two of the four
/// moments a test that does not change the pose reaches. `StandingSidebarTests`, hosted, holds all four.
final class PosedPortraitTests: XCTestCase {
  @MainActor
  func testTheBarStandsAcrossTheHead() throws {
    try PosedWalk.walk(.portrait, self, barStandsVertically: false)
  }

  /// The app over the fake's `paired` phone, launched afresh in portrait with `arguments`, the software keyboard one that
  /// stays up.
  @MainActor
  private func launched(_ arguments: [String] = []) throws -> XCUIApplication {
    try Pose.portrait.launched(
      self, ["--fake", "paired", "--forget-scene", "-AutomaticMinimizationEnabled", "NO"] + arguments)
  }

  @MainActor
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any).matching(identifier: identifier).firstMatch
  }

  /// Waits until it stands so.
  @MainActor
  private func waits(
    _ said: String, file: StaticString = #filePath, line: UInt = #line, _ stands: () -> Bool
  ) {
    for _ in 0..<40 {
      if stands() { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    attach("Not as it should stand")
    XCTFail(said, file: file, line: line)
  }

  /// Waits until the Sidebar stands over the main view: the listing at the window's leading side and short of its
  /// trailing edge, which the main view beneath reaches.
  @MainActor
  private func sidebarStandsOver(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    let scope = app.buttons["listing.scope"]
    let list = app.collectionViews["listing.list"]
    waits("the Sidebar stands over the main view", file: file, line: line) {
      scope.exists && scope.isHittable && list.exists && list.frame.minX < 1
        && list.frame.maxX < app.window.frame.maxX - 200
    }
  }

  /// Waits until the main view stands alone, showing `identifier`, with the Sidebar behind its button.
  @MainActor
  private func mainViewStandsAlone(
    _ app: XCUIApplication, showing identifier: String, file: StaticString = #filePath, line: UInt = #line
  ) {
    let shown = element(identifier, in: app)
    let scope = app.buttons["listing.scope"]
    waits("\(identifier) stands alone", file: file, line: line) {
      shown.exists && shown.isHittable && (!scope.exists || !scope.isHittable)
        && app.buttons["Show Sidebar"].firstMatch.isHittable
    }
    XCTAssertFalse(app.back.exists, "the main view's root", file: file, line: line)
  }

  /// Taps beside the Sidebar, over the dimmed main view, which puts it away.
  @MainActor
  private func tapBesideTheSidebar(_ app: XCUIApplication) {
    app.point(across: 0.85, down: 0.5).tap()
  }

  @MainActor
  func testASessionOpenStandsAlone() throws {
    let app = try launched(["--open", "workstation-key/7"])
    mainViewStandsAlone(app, showing: "session.title")
    attach("A session alone in portrait")
  }

  @MainActor
  func testLaunchedWithNothingOpenTheListingStandsOverTheNewSessionScreenAndASessionChosenStandsAlone() throws {
    let app = try launched()
    sidebarStandsOver(app)
    attach("Launched in portrait: the listing over the new-session screen")
    // Beneath it is the new-session screen at rest, which a tap beside it leaves.
    tapBesideTheSidebar(app)
    mainViewStandsAlone(app, showing: "landing.title")
    XCTAssertFalse(
      element("landing.composer", in: app).value(forKey: "hasKeyboardFocus") as? Bool ?? false, "at rest")
    // Shown again by its button, a Session chosen from it stands alone, the Sidebar going with nothing asked.
    app.buttons["Show Sidebar"].firstMatch.tap()
    sidebarStandsOver(app)
    let row = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "Tidy zsh startup time")).firstMatch
    app.scrolled(to: row, over: app.collectionViews["listing.list"]).tap()
    mainViewStandsAlone(app, showing: "session.title")
    XCTAssertTrue(
      app.staticTexts.matching(identifier: "session.title")
        .waitForOne(NSPredicate(format: "label == %@", "Tidy zsh startup time")))
    attach("A session chosen from the listing in portrait")
  }

  @MainActor
  func testTheListingStandsOverTheNewSessionScreenAsThePathEmpties() throws {
    let app = try launched(["--open", "workstation-key/7"])
    mainViewStandsAlone(app, showing: "session.title")
    // Over the fake a Prompt holding "gone" finds the Session gone, which empties the path.
    let composer = element("session.composer", in: app)
    composer.tap()
    composer.typeText("Is it gone?")
    app.buttons["session.send"].tap()
    XCTAssertTrue(app.staticTexts["session.title"].waitForNonExistence(timeout: 20), "the Session gone")
    sidebarStandsOver(app)
    attach("The path emptied in portrait: the listing over the new-session screen")
    tapBesideTheSidebar(app)
    mainViewStandsAlone(app, showing: "landing.title")
  }

  @MainActor
  func testItIsNeverStoodOverAFieldThatHoldsTheKeyboard() throws {
    let app = try launched()
    sidebarStandsOver(app)
    tapBesideTheSidebar(app)
    mainViewStandsAlone(app, showing: "landing.title")
    let composer = element("landing.composer", in: app)
    composer.tap()
    XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10), "touching its field raises its keyboard")
    composer.typeText("Somewhere to begin")
    // Touched, it is open, and no moment asks for the Sidebar over it.
    RunLoop.current.run(until: Date().addingTimeInterval(3))
    let scope = app.buttons["listing.scope"]
    XCTAssertFalse(scope.exists && scope.isHittable, "the Sidebar is not over it")
    XCTAssertTrue(composer.value(forKey: "hasKeyboardFocus") as? Bool ?? false, "which keeps the keyboard")
    attach("A touched new-session screen in portrait")
  }
}
