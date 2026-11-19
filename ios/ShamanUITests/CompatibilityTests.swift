import XCTest

/// Servers on other releases than the app, over the fake: one that no longer works with the app and one too old for it
/// each stand where an Unreachable Remote's row would, naming the side to update, and one merely newer is said once, in
/// the banner an act's failure uses.
final class CompatibilityTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--forget-scene"]
    app.launch()
    return app
  }

  @MainActor
  private func attach(_ app: XCUIApplication, _ name: String) {
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = name
    screenshot.lifetime = .keepAlways
    add(screenshot)
  }

  /// Scrolls the listing until `element` can be tapped, clear of the bottom bar standing over the listing's foot.
  @MainActor
  private func scrolled(to element: XCUIElement, in app: XCUIApplication) -> XCUIElement {
    let bar = app.buttons["home.servers"]
    let clear = { element.exists && element.isHittable && (!bar.exists || element.frame.maxY <= bar.frame.minY) }
    var swipes = 0
    while !clear() && swipes < 12 {
      app.swipeUp()
      swipes += 1
    }
    XCTAssertTrue(element.isHittable, "scrolled to \(element)")
    return element
  }

  /// The slim row whose words carry `words`.
  @MainActor
  private func slim(_ words: String, in app: XCUIApplication) -> XCUIElement {
    app.buttons.matching(identifier: "listing.unreachable").matching(NSPredicate(format: "label CONTAINS %@", words))
      .firstMatch
  }

  @MainActor
  func testRemotesTooFarApartStandWhereUnreachableRowsWouldNamingTheSideToUpdateAndTryingAgainSaysSo() {
    let app = launched("releases")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let laptop = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "laptop · ")).firstMatch
    XCTAssertTrue(laptop.waitForExistence(timeout: 10), "its rows stay")

    let appBehind = scrolled(to: slim("Update the app to reach laptop", in: app), in: app)
    XCTAssertTrue(appBehind.label.contains("Try again"), appBehind.label)
    let serverBehind = slim("Update Shaman on homelab", in: app)
    XCTAssertTrue(serverBehind.exists)
    attach(app, "The laptop and the home lab, each naming the side to update")

    appBehind.tap()
    let explanation = app.staticTexts["unreachable.explanation"]
    XCTAssertTrue(explanation.waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["Update the app"].exists)
    XCTAssertTrue(
      explanation.label.contains("laptop no longer works with an app as old as this one"), explanation.label)
    XCTAssertFalse(app.buttons["unreachable.openSettings"].exists, "no Settings to lead to")
    attach(app, "The laptop's sheet saying to update the app")
    app.buttons["unreachable.tryAgain"].tap()
    let banner = app.descendants(matching: .any).matching(identifier: "banner")
      .matching(NSPredicate(format: "label CONTAINS %@", "update the app")).firstMatch
    XCTAssertTrue(banner.waitForExistence(timeout: 10))
    XCTAssertTrue(explanation.exists, "the sheet stays up")
    attach(app, "Trying the laptop again says to update the app")
    app.swipeDown(velocity: .fast)
    XCTAssertTrue(explanation.waitForNonExistence(timeout: 10))

    _ = scrolled(to: serverBehind, in: app)
    serverBehind.tap()
    XCTAssertTrue(explanation.waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["Update Shaman on homelab"].exists)
    XCTAssertTrue(explanation.label.contains("Update Shaman there"), explanation.label)
    attach(app, "The home lab's sheet saying to update Shaman there")
  }

  @MainActor
  func testAServerNewerThanTheAppIsSaidOnceInTheBanner() {
    let app = launched("releases")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let banner = app.descendants(matching: .any).matching(identifier: "banner")
      .matching(NSPredicate(format: "label CONTAINS %@", "runs a newer Shaman")).firstMatch
    XCTAssertTrue(banner.waitForExistence(timeout: 10))
    XCTAssertEqual(banner.label, "workstation runs a newer Shaman; some things may not show")
    attach(app, "The workstation said once to be newer")

    banner.tap()
    XCTAssertTrue(banner.waitForNonExistence(timeout: 10), "tapped away")
    // Opening a Session and coming back says it no more.
    let row = app.descendants(matching: .any).matching(identifier: "listing.active").firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    row.tap()
    XCTAssertTrue(app.navigationBars.buttons.firstMatch.waitForExistence(timeout: 10))
    app.navigationBars.buttons.firstMatch.tap()
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    XCTAssertFalse(banner.exists, "never said again")
  }
}
