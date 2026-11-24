import XCTest

/// The text of a Prompt written and not yet sent survives the app being ended: over the fake, which keeps its drafts
/// as the core does where a directory is named, what a composer held as the app left the front is in it the next run.
final class DraftTests: XCTestCase {
  /// The directory this test's runs keep their drafts in.
  private let run = "drafts-\(UUID().uuidString)"

  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ arguments: [String]) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchEnvironment["SHAMAN_CORE_DIRECTORY"] = run
    app.launchArguments = arguments
    app.launch()
    return app
  }

  /// Leaves nothing behind: neither the drafts kept nor what the scene kept of the screen left open.
  @MainActor
  private func leavingNothing() {
    addTeardownBlock { @MainActor in
      let removing = self.launched(["--remove-core-directory"])
      XCTAssertTrue(removing.staticTexts["core.removed"].waitForExistence(timeout: 10))
      removing.terminate()
      let forgetting = XCUIApplication()
      forgetting.launchArguments = ["--fake", "paired", "--forget-scene"]
      forgetting.launch()
      XCTAssertTrue(forgetting.buttons["home.compose"].waitForExistence(timeout: 10))
      self.ended(forgetting)
    }
  }

  /// Sends `app` to the background and has it ended there, as iOS ends a suspended app.
  @MainActor
  private func ended(_ app: XCUIApplication) {
    app.leaveTheFront()
    _ = XCTWaiter.wait(for: [XCTestExpectation(description: "the scene kept")], timeout: 2)
    app.terminate()
  }

  @MainActor
  private func field(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any)[identifier]
  }

  /// Waits until `field` holds `text`.
  @MainActor
  private func holds(_ field: XCUIElement, _ text: String) {
    let holding = NSPredicate(format: "value == %@", text)
    XCTAssertTrue(
      XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: holding, object: field)], timeout: 10) == .completed,
      "the field holds \(text), not \(String(describing: field.value))")
  }

  @MainActor
  func testWhatASessionsComposerHeldIsInItAfterTheAppIsEnded() {
    leavingNothing()
    let opening = ["--fake", "paired", "--open", "workstation-key/7"]
    var app = launched(opening)
    var composer = field("session.composer", in: app)
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    composer.tap()
    composer.typeText("Also time the plugin loads")
    holds(composer, "Also time the plugin loads")

    ended(app)
    app = launched(opening)
    composer = field("session.composer", in: app)
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    holds(composer, "Also time the plugin loads")
    attach("A session's unsent text after the app was ended")
  }

  @MainActor
  func testWhatTheNewSessionScreensComposerHeldIsInItAfterTheAppIsEnded() {
    leavingNothing()
    let composing = ["--fake", "composing", "--compose"]
    var app = launched(composing)
    var composer = field("landing.composer", in: app)
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    composer.tap()
    composer.typeText("Find why the relay drops idle joins")
    holds(composer, "Find why the relay drops idle joins")

    ended(app)
    app = launched(composing)
    composer = field("landing.composer", in: app)
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    holds(composer, "Find why the relay drops idle joins")
    attach("The new-session screen's unsent text after the app was ended")
  }
}
