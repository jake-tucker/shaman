import XCTest

/// What a test that turns the simulator stands on, as every test of the columns does: the app launched either way up,
/// turned and turned back with it up, and what tells whether its columns stand. An iPhone 18 Pro Max turned is regular
/// width, so its columns stand side by side, and collapse as it is turned back.
///
/// A class of these is named in the table of `scripts/ios-test.sh`, among those of the columns, which a Pro Max alone
/// stands, or those that turn, which a Pro runs as well. No test turns a Duo, so neither is run on one.
class TurningTestCase: XCTestCase {
  /// The app whose animations XCTest miscounts, once it does.
  @MainActor private var miscounted: XCUIApplication?

  override func setUp() {
    continueAfterFailure = false
  }

  override func tearDown() async throws {
    await MainActor.run {
      // Ended first, as turning it waits for it to stand still, which one whose animations were miscounted never does.
      XCUIApplication().terminate()
      // As every other test begins, whatever became of this one.
      XCUIDevice.shared.orientation = .portrait
    }
  }

  /// The app over the fake's `scenario`, as a new scene opens it, launched on its side where `turned`.
  @MainActor
  func launched(
    _ scenario: String = "paired", _ arguments: [String] = [], turned: Bool = false, file: StaticString = #filePath,
    line: UInt = #line
  ) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--forget-scene"] + arguments
    XCUIDevice.shared.orientation = turned ? .landscapeLeft : .portrait
    app.launch()
    stands(turned: turned, app, file: file, line: line)
    return app
  }

  /// Turns the device, upright or on either side, and waits until the app has turned with it and stands still.
  ///
  /// XCTest waits until as many of the app's animations have stopped as have started, or a minute. As the columns
  /// collapse, CoreAnimation now and then tells of one starting twice, of the edge effect UIKit draws where a list
  /// scrolls beneath its bars, and of its stopping once. From then on the app never stands still to XCTest, and each
  /// step of the test waits out that minute, until the app ends. So where a turn took one, no step after it waits but
  /// for what it looks for.
  @MainActor
  func turn(
    _ orientation: UIDeviceOrientation, _ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    let began = Date()
    XCUIDevice.shared.orientation = orientation
    if Date().timeIntervalSince(began) > 55 {
      // That it waits neither before an event nor after one, in XCTest's own words for it, as it offers no others.
      let skips = NSSelectorFromString("setCurrentInteractionOptions:")
      if app.responds(to: skips) { app.setValue(0b11, forKey: "currentInteractionOptions") }
      miscounted = app
    }
    if miscounted === app {
      // As long as a turn's own animations take, and more.
      RunLoop.current.run(until: Date().addingTimeInterval(2))
    }
    stands(turned: orientation.isLandscape, app, file: file, line: line)
  }

  @MainActor
  private func stands(turned: Bool, _ app: XCUIApplication, file: StaticString, line: UInt) {
    for _ in 0..<60 {
      let frame = app.window.exists ? app.window.frame : .zero
      if !frame.isEmpty, (frame.width > frame.height) == turned { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail("the app stands \(turned ? "on its side" : "upright"): \(app.window.frame)", file: file, line: line)
  }

  /// Waits until the Sidebar stands beside the main view, whose screen's title is `title`'s element: both reached, the
  /// one clear of the other.
  @MainActor
  func columnsStand(
    _ app: XCUIApplication, title: String = "session.title", file: StaticString = #filePath, line: UInt = #line
  ) {
    let scope = app.buttons["listing.scope"]
    let main = app.staticTexts[title]
    for _ in 0..<40 {
      if scope.exists, main.exists, scope.isHittable, main.isHittable, scope.frame.maxX < main.frame.minX { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail("the Sidebar stands beside the main view", file: file, line: line)
  }

  /// Waits until one column alone stands: the listing, or what is open over it.
  @MainActor
  func oneColumnStands(
    _ app: XCUIApplication, showing identifier: String, file: StaticString = #filePath, line: UInt = #line
  ) {
    let shown = elements(identifier, in: app).firstMatch
    let scope = app.buttons["listing.scope"]
    let listing = identifier == "listing.scope"
    for _ in 0..<40 {
      if shown.exists, shown.isHittable {
        if listing, !app.staticTexts["session.title"].exists, !app.staticTexts["landing.title"].exists { return }
        if !listing, !scope.exists || !scope.isHittable { return }
      }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail("\(identifier) stands alone", file: file, line: line)
  }

  @MainActor
  func elements(_ identifier: String, in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: identifier)
  }

  @MainActor
  func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    elements(identifier, in: app).firstMatch
  }

  /// The element `identifier` names whose label holds `text`.
  @MainActor
  func element(_ identifier: String, holding text: String, in app: XCUIApplication) -> XCUIElement {
    elements(identifier, in: app).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
  }

  /// Waits until an element `identifier` names stands as `format` says.
  @MainActor
  func waitForOne(
    _ identifier: String, _ format: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    XCTAssertTrue(
      elements(identifier, in: app).waitForOne(NSPredicate(format: format)), "\(identifier): \(format)", file: file,
      line: line)
  }

  /// Waits until the open Session's Title reads `expected`.
  @MainActor
  func titled(_ expected: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    XCTAssertTrue(
      app.staticTexts.matching(identifier: "session.title").waitForOne(NSPredicate(format: "label == %@", expected)),
      "the Session titled \(expected) shows", file: file, line: line)
  }

  /// Opens the Session the listing's row `title` names.
  @MainActor
  func open(_ title: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    let row = element("listing.active", holding: title, in: app)
    XCTAssertTrue(elements("listing.active", in: app).firstMatch.waitForExistence(timeout: 10), file: file, line: line)
    app.scrolled(to: row, over: listing(in: app), file: file, line: line).tap()
    titled(title, in: app, file: file, line: line)
    // What of it is on screen where the display is short is the foot of its last Turn.
    let read = NSPredicate(format: "identifier IN %@", ["session.turn", "session.agentMessage", "session.userMessage"])
    XCTAssertTrue(
      element("session.conversation", in: app).descendants(matching: .any).waitForOne(read), "\(title) is read",
      file: file, line: line)
  }

  /// The listing's own list, where the main view shows another beside it.
  @MainActor
  func listing(in app: XCUIApplication) -> XCUIElement {
    app.collectionViews["listing.list"]
  }

  /// Whether `element` holds the keyboard, which a simulator may keep minimised below the screen.
  @MainActor
  func holdsTheKeyboard(_ element: XCUIElement) -> Bool {
    element.value(forKey: "hasKeyboardFocus") as? Bool ?? false
  }

  /// Waits until `element` holds the keyboard, or no longer does.
  @MainActor
  func waitForKeyboard(
    _ holds: Bool, on element: XCUIElement, _ message: String, file: StaticString = #filePath, line: UInt = #line
  ) {
    for _ in 0..<20 {
      if element.exists, holdsTheKeyboard(element) == holds { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail(message, file: file, line: line)
  }
}
