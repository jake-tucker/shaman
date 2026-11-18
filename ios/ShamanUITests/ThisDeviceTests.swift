import XCTest

/// "This device", over the live core in a directory of each test's own, and over the fake.
final class ThisDeviceTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  /// The app over the live core kept in `directory`.
  @MainActor
  private func launched(in directory: String) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchEnvironment["SHAMAN_CORE_DIRECTORY"] = directory
    app.launch()
    return app
  }

  @MainActor
  private func rename(_ app: XCUIApplication, to name: String) {
    let field = app.textFields["device.name"]
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    // At its trailing end, where the name ends, so every character of it is deleted.
    field.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
    let written = field.value as? String ?? ""
    field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: written.count) + name + "\n")
  }

  @MainActor
  func testTheNameIsFirstOfferedAsIPhoneRefusedInTheCoresWordsAndKeptAcrossARelaunch() {
    let directory = "ui-test-\(UUID().uuidString)"
    var app = launched(in: directory)
    XCTAssertTrue(app.navigationBars["Servers & device"].waitForExistence(timeout: 10))
    let field = app.textFields["device.name"]
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    XCTAssertEqual(field.value as? String, "iPhone")

    rename(app, to: "everywhere")
    let refusal = app.staticTexts["device.name.refusal"]
    XCTAssertTrue(refusal.waitForExistence(timeout: 10))
    XCTAssertEqual(refusal.label, "\"everywhere\" names every server at once, so nothing may be named so")
    let refused = XCTAttachment(screenshot: app.screenshot())
    refused.name = "A refused name"
    refused.lifetime = .keepAlways
    add(refused)

    rename(app, to: "Pocket")
    XCTAssertTrue(refusal.waitForNonExistence(timeout: 10))
    XCTAssertEqual(field.value as? String, "Pocket")

    app.terminate()
    app = launched(in: directory)
    let relaunched = app.textFields["device.name"]
    XCTAssertTrue(relaunched.waitForExistence(timeout: 10))
    XCTAssertEqual(relaunched.value as? String, "Pocket")
  }

  @MainActor
  func testTheFakeShowsItsScenarioWithTheVersions() {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "named"]
    app.launch()
    let field = app.textFields["device.name"]
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    XCTAssertEqual(field.value as? String, "Pocket")
    XCTAssertEqual(app.staticTexts["device.version"].label, "Version, 0.1.0 (1)")
    XCTAssertEqual(app.staticTexts["device.protocol"].label, "Protocol, 116")
  }
}
