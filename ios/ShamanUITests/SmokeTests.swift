import XCTest

/// The app launched and walked one step, over the fake, on whichever simulator `scripts/ios-test.sh` runs it: the least
/// a simulator new to its table shows before any other test of the app is held there.
final class SmokeTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  func testASessionOpensFromTheListingAndBackReturnsToIt() {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "paired"]
    app.launch()
    let row = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "Draw the invite as a QR code")).firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    row.tap()

    let title = app.descendants(matching: .any)["session.title"]
    XCTAssertTrue(title.waitForExistence(timeout: 5))
    XCTAssertEqual(title.label, "Draw the invite as a QR code in /serve")
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = "A session opened from the listing"
    screenshot.lifetime = .keepAlways
    add(screenshot)

    // By its identifier, as Back is not among the navigation bar's buttons where the bar stands vertically.
    app.buttons["BackButton"].tap()
    XCTAssertTrue(row.waitForExistence(timeout: 5))
    XCTAssertFalse(title.exists)
  }
}
