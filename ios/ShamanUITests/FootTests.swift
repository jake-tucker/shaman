import XCTest

/// Where the composer's field stands at compact width, upright: where Messages stands its own, and on the software
/// keyboard.
final class FootTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ arguments: [String]) -> XCUIApplication {
    let app = XCUIApplication()
    // The software keyboard stays up where the simulator would minimise it.
    app.launchArguments = ["--fake"] + arguments + ["--forget-scene", "-AutomaticMinimizationEnabled", "NO"]
    app.launch()
    return app
  }

  /// Drags the software keyboard down, as a reader does over what is above it.
  @MainActor
  private func dragTheKeyboardDown(in app: XCUIApplication) {
    app.point(across: 0.5, down: 0.25).press(forDuration: 0.05, thenDragTo: app.point(across: 0.5, down: 0.98))
    XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 10), "its keyboard put down")
  }

  @MainActor
  func testASessionsFieldStandsWhereMessagesStandsItsOwnAndOnTheKeyboard() {
    let app = launched(["paired", "--open", "workstation-key/7"])
    let composer = app.descendants(matching: .any)["session.composer"]
    for _ in 0..<2 {
      fieldStands(28, aboveTheEdgeIn: app, "session.composer")
      attach("A session's foot, the keyboard down")
      composer.tap()
      fieldStandsOnTheKeyboard(in: app, "session.composer")
      attach("A session's foot, the keyboard up")
      XCTAssertEqual(composer.value(forKey: "hasKeyboardFocus") as? Bool, true, "it keeps its focus")
      dragTheKeyboardDown(in: app)
    }
    fieldStands(28, aboveTheEdgeIn: app, "session.composer")
  }

  @MainActor
  func testTheNewSessionScreensFieldStandsThereToo() {
    let app = launched(["composing", "--compose"])
    fieldStandsOnTheKeyboard(in: app, "landing.composer", "on arrival,")
    attach("The new-session screen's foot, the keyboard up")
    dragTheKeyboardDown(in: app)
    fieldStands(28, aboveTheEdgeIn: app, "landing.composer")
    attach("The new-session screen's foot, the keyboard down")
    app.descendants(matching: .any)["landing.composer"].tap()
    fieldStandsOnTheKeyboard(in: app, "landing.composer")
  }
}
