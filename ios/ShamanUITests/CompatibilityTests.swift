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
    // Slowly, so a slim row at the listing's foot is not flicked past between one swipe and the next.
    while !clear() && swipes < 24 {
      app.swipeUp(velocity: .slow)
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

  /// The `releases` phone's listing, with the note that the workstation is newer tapped away so the listing holds
  /// still beneath where it stood.
  @MainActor
  private func listing() -> XCUIApplication {
    let app = launched("releases")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let said = notice(app)
    XCTAssertTrue(said.waitForExistence(timeout: 10))
    said.tap()
    XCTAssertTrue(said.waitForNonExistence(timeout: 10))
    return app
  }

  @MainActor
  func testARemoteRefusingTheAppStandsWhereAnUnreachableRowWouldSayingToUpdateTheApp() {
    let app = listing()
    let laptop = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "laptop · ")).firstMatch
    XCTAssertTrue(laptop.waitForExistence(timeout: 10), "its rows stay")

    let appBehind = scrolled(to: slim("Update the app to reach laptop", in: app), in: app)
    XCTAssertTrue(appBehind.label.contains("Try again"), appBehind.label)
    XCTAssertTrue(slim("Update Shaman on homelab", in: app).exists)
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
  }

  @MainActor
  func testARemoteTooOldForTheAppStandsWhereAnUnreachableRowWouldSayingToUpdateShamanThere() {
    let app = listing()
    let serverBehind = scrolled(to: slim("Update Shaman on homelab", in: app), in: app)
    serverBehind.tap()
    let explanation = app.staticTexts["unreachable.explanation"]
    XCTAssertTrue(explanation.waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["Update Shaman on homelab"].exists)
    XCTAssertTrue(explanation.label.contains("Update Shaman there"), explanation.label)
    attach(app, "The home lab's sheet saying to update Shaman there")
  }

  /// What the core says of its own accord, wherever it stands.
  @MainActor
  private func notice(_ app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "runs a newer Shaman")).firstMatch
  }

  @MainActor
  func testAServerNewerThanTheAppIsSaidOnceInABannerBeneathTheBar() {
    let app = launched("releases")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let banner = notice(app)
    XCTAssertTrue(banner.waitForExistence(timeout: 10))
    XCTAssertEqual(banner.label, "workstation runs a newer Shaman; some things may not show")
    let title = app.buttons["listing.scope"]
    XCTAssertTrue(title.isHittable, "the title's menu stands clear of it")
    XCTAssertGreaterThanOrEqual(banner.frame.minY, app.navigationBars.firstMatch.frame.maxY - 1, "beneath the bar")
    attach(app, "The workstation said once to be newer, beneath the bar")

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

  @MainActor
  func testANoticeOutlastsASheetOverItAndGoesOnceItHasStoodOnScreen() {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "releases", "--forget-scene", "--notice-stands", "5"]
    app.launch()
    let servers = app.buttons["home.servers"]
    XCTAssertTrue(servers.waitForExistence(timeout: 10))
    servers.tap()
    XCTAssertTrue(app.navigationBars["Servers & device"].waitForExistence(timeout: 10))
    // Longer than it stands, beneath the sheet.
    sleep(7)
    app.swipeDown(velocity: .fast)
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let banner = notice(app)
    XCTAssertTrue(banner.exists, "not seen while the sheet hid it")
    attach(app, "The notice still standing once the sheet goes")
    XCTAssertTrue(banner.waitForNonExistence(timeout: 10), "and gone once it has stood on screen")
  }

  @MainActor
  func testASessionAcrossTheGapSaysWhyItCannotBeReadAndTakesNothing() {
    let app = launched("releases")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let laptop = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "laptop · design-system")).firstMatch
    XCTAssertTrue(laptop.waitForExistence(timeout: 10))
    laptop.tap()
    let unread = app.staticTexts["session.unread"]
    XCTAssertTrue(unread.waitForExistence(timeout: 10))
    XCTAssertTrue(unread.label.contains("laptop no longer works with an app as old as this one"), unread.label)
    let line = app.descendants(matching: .any)["session.unreachable"]
    XCTAssertTrue(line.waitForExistence(timeout: 10))
    XCTAssertTrue(line.label.contains("update the app. Nothing sent goes anywhere until then"), line.label)
    attach(app, "A session on the laptop, which cannot be read until the app is updated")
  }
}
