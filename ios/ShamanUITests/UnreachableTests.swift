import XCTest

/// A Remote the phone cannot reach, over the fake: its rows stay, dimmed, it stands as a slim row at the foot of the
/// active ones, and tapping that opens a short sheet offering to try it again.
final class UnreachableTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario]
    app.launch()
    return app
  }

  @MainActor
  func testAnUnreachableRemoteStandsAsASlimRowWhoseSheetLeadsToSettingsAndTryingAgainBringsItBack() {
    let app = launched("unreachable")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let homelab = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "homelab · home-assistant")).firstMatch
    XCTAssertTrue(homelab.waitForExistence(timeout: 10), "its rows stay")

    let slim = app.scrolled(to: app.buttons["listing.unreachable"])
    XCTAssertTrue(slim.label.contains("homelab unreachable"), slim.label)
    XCTAssertTrue(slim.label.contains("Try again"), slim.label)
    attach("The home lab's slim row at the foot of the active rows")

    slim.tap()
    let explanation = app.staticTexts["unreachable.explanation"]
    XCTAssertTrue(explanation.waitForExistence(timeout: 10))
    XCTAssertTrue(explanation.label.contains("local network access may be off"), explanation.label)
    XCTAssertTrue(app.staticTexts["homelab is unreachable"].exists)
    XCTAssertTrue(app.buttons["unreachable.openSettings"].exists)
    attach("The sheet saying local network access may be off")

    app.buttons["unreachable.tryAgain"].tap()
    XCTAssertTrue(explanation.waitForNonExistence(timeout: 10), "the sheet closes once it answers")
    XCTAssertFalse(app.buttons["listing.unreachable"].exists, "and the slim row leaves")
    attach("The home lab answering again")
  }

  @MainActor
  func testWithNoNetworkTheSheetSaysSoAndTryingAgainSaysWhyItStillCannot() {
    let app = launched("offline")
    let slim = app.buttons["listing.unreachable"]
    XCTAssertTrue(app.navigationBars["All workspaces"].waitForExistence(timeout: 10))
    XCTAssertTrue(
      app.descendants(matching: .any).matching(identifier: "listing.active").firstMatch.waitForExistence(timeout: 10))
    app.scrolled(to: slim)
    XCTAssertTrue(slim.label.contains("Server unreachable"), slim.label)
    slim.tap()

    let explanation = app.staticTexts["unreachable.explanation"]
    XCTAssertTrue(explanation.waitForExistence(timeout: 10))
    XCTAssertTrue(explanation.label.contains("no network connection"), explanation.label)
    XCTAssertFalse(app.buttons["unreachable.openSettings"].exists, "no Settings to lead to")
    app.buttons["unreachable.tryAgain"].tap()
    let banner = app.descendants(matching: .any)["banner"]
    XCTAssertTrue(banner.waitForExistence(timeout: 10))
    XCTAssertEqual(banner.label, "This phone has no network connection")
    XCTAssertTrue(explanation.exists, "the sheet stays up")
    attach("No network: the sheet stays and says why")
  }
}
