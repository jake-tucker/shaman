import XCTest

/// "This device", over the live core in a directory and with a Keychain item of each test's own, and over the fake.
/// A phone holding no Remote offers pairing alone, so each is launched with `--servers-and-device`, which shows the
/// sheet of Servers, Relays and This device by itself.
final class ThisDeviceTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  /// The app over the live core kept in `directory`, with its key in the Keychain item `key`, named as the directory
  /// is unless named otherwise.
  @MainActor
  private func launched(in directory: String, key: String? = nil, arguments: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchEnvironment["SHAMAN_CORE_DIRECTORY"] = directory
    app.launchEnvironment["SHAMAN_KEY_ITEM"] = key ?? directory
    app.launchArguments = arguments + ["--servers-and-device"]
    app.launch()
    return app
  }

  /// A directory and a Keychain item named for it, removed when the test ends.
  @MainActor
  private func ownDirectory() -> String {
    let directory = "ui-test-\(UUID().uuidString)"
    addTeardownBlock { @MainActor in self.remove(directory) }
    return directory
  }

  /// Removes the directory a test's run of the app kept its records in, and the Keychain item named as it is, which
  /// only the app can reach.
  @MainActor
  private func remove(_ directory: String, key: String? = nil) {
    let app = launched(in: directory, key: key, arguments: ["--remove-core-directory"])
    XCTAssertTrue(app.staticTexts["core.removed"].waitForExistence(timeout: 10))
    app.terminate()
  }

  /// The fingerprint "This device" shows.
  @MainActor
  private func fingerprint(_ app: XCUIApplication) -> String {
    let shown = app.descendants(matching: .any)["device.fingerprint"]
    XCTAssertTrue(shown.waitForExistence(timeout: 10))
    return shown.value as? String ?? ""
  }

  @MainActor
  private func attach(_ app: XCUIApplication, _ name: String) {
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = name
    screenshot.lifetime = .keepAlways
    add(screenshot)
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
    let directory = ownDirectory()
    var app = launched(in: directory)
    XCTAssertTrue(app.navigationBars["Servers & device"].waitForExistence(timeout: 10))
    let field = app.textFields["device.name"]
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    XCTAssertEqual(field.value as? String, "iPhone")

    rename(app, to: "everywhere")
    let refusal = app.staticTexts["device.name.refusal"]
    XCTAssertTrue(refusal.waitForExistence(timeout: 10))
    XCTAssertEqual(refusal.label, "\"everywhere\" names every server at once, so nothing may be named so")
    attach(app, "A refused name")

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
    app.launchArguments = ["--fake", "named", "--servers-and-device"]
    app.launch()
    let field = app.textFields["device.name"]
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    XCTAssertEqual(field.value as? String, "Pocket")
    XCTAssertEqual(app.staticTexts["device.version"].label, "Version, 0.1.0 (1)")
    XCTAssertEqual(app.staticTexts["device.protocol"].label, "Protocol, 116")
  }

  @MainActor
  func testTheKeysFingerprintIsShownAndStaysAcrossARelaunch() {
    let directory = ownDirectory()
    var app = launched(in: directory)
    let shown = fingerprint(app)
    XCTAssertEqual(shown.count, 64, shown)
    XCTAssertTrue(shown.allSatisfy(\.isHexDigit), shown)
    attach(app, "This device with its fingerprint")

    app.terminate()
    app = launched(in: directory)
    XCTAssertEqual(fingerprint(app), shown, "the one key, reused")
  }

  @MainActor
  func testUnderANewKeyTheNameStaysAndTheFingerprintIsTheNewKeys() {
    let directory = ownDirectory()
    var app = launched(in: directory)
    let lost = fingerprint(app)
    rename(app, to: "Pocket")
    XCTAssertEqual(app.textFields["device.name"].value as? String, "Pocket")
    app.terminate()

    // The Keychain no longer holds the key the records were kept under.
    let newKey = "\(directory)-new"
    addTeardownBlock { @MainActor in self.remove("\(directory)-gone", key: newKey) }
    app = launched(in: directory, key: newKey)
    let new = fingerprint(app)
    XCTAssertNotEqual(new, lost)
    XCTAssertEqual(app.textFields["device.name"].value as? String, "Pocket", "the name stays")
  }

  @MainActor
  func testABlobTheChipRefusesOffersToStartAgainUnderANewKey() {
    let directory = ownDirectory()
    let app = launched(in: directory, arguments: ["--spoil-key"])
    let startAgain = app.buttons["key.startAgain"]
    XCTAssertTrue(startAgain.waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["This phone's key cannot be used"].exists)
    attach(app, "A key the chip refuses")

    startAgain.tap()
    XCTAssertEqual(fingerprint(app).count, 64)
    XCTAssertEqual(app.textFields["device.name"].value as? String, "iPhone")
  }

  /// Where pairing is all a phone with a new key is offered.
  @MainActor
  func testThatThisPhoneMustPairAgainIsSaidUntilDismissed() {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "pairAgain"]
    app.launch()
    let said = app.staticTexts["pairAgain"]
    XCTAssertTrue(said.waitForExistence(timeout: 10))
    XCTAssertTrue(said.label.hasSuffix("Pair with each of them again."), said.label)
    attach(app, "This phone must pair again")

    app.buttons["pairAgain.dismiss"].tap()
    XCTAssertTrue(said.waitForNonExistence(timeout: 10))
  }
}
