import XCTest

/// Servers on other releases than the app, over the fake: one that no longer works with the app and one too old for it
/// each stand where an Unreachable Remote's row would, naming the side to update, and one merely newer is said once, in
/// the banner an act's failure uses.
final class CompatibilityTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  /// The app over the fake `scenario`, a notice standing `noticeStands` seconds before it goes by itself: longer than any
  /// test takes, unless one is asked for, as a loaded Mac can outlast the usual while between two looks at the screen.
  @MainActor
  private func launched(
    _ scenario: String, noticeStands: TimeInterval = 3600, arguments: [String] = []
  ) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments =
      ["--fake", scenario, "--forget-scene", "--notice-stands", String(noticeStands)] + arguments
    app.launch()
    return app
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

    let appBehind = app.scrolled(to: slim("Update the app to reach laptop", in: app))
    XCTAssertTrue(appBehind.label.contains("Try again"), appBehind.label)
    XCTAssertTrue(slim("Update Shaman on homelab", in: app).exists)
    attach("The laptop and the home lab, each naming the side to update")

    appBehind.tap()
    let explanation = app.staticTexts["unreachable.explanation"]
    XCTAssertTrue(explanation.waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["Update the app"].exists)
    XCTAssertTrue(
      explanation.label.contains("laptop no longer works with an app as old as this one"), explanation.label)
    XCTAssertFalse(app.buttons["unreachable.openSettings"].exists, "no Settings to lead to")
    attach("The laptop's sheet saying to update the app")
    app.buttons["unreachable.tryAgain"].tap()
    let banner = app.descendants(matching: .any).matching(identifier: "banner")
      .matching(NSPredicate(format: "label CONTAINS %@", "update the app")).firstMatch
    XCTAssertTrue(banner.waitForExistence(timeout: 10))
    XCTAssertTrue(explanation.exists, "the sheet stays up")
    attach("Trying the laptop again says to update the app")
  }

  @MainActor
  func testARemoteTooOldForTheAppStandsWhereAnUnreachableRowWouldSayingToUpdateShamanThere() {
    // The notice, not tapped away, stands throughout, so what hides it is the sheet alone.
    let app = launched("releases")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let said = notice(app)
    XCTAssertTrue(said.waitForExistence(timeout: 10))
    let serverBehind = app.scrolled(to: slim("Update Shaman on homelab", in: app))
    serverBehind.tap()
    let explanation = app.staticTexts["unreachable.explanation"]
    XCTAssertTrue(explanation.waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["Update Shaman on homelab"].exists)
    XCTAssertTrue(explanation.label.contains("Update Shaman there"), explanation.label)
    attach("The home lab's sheet saying to update Shaman there")

    XCTAssertFalse(said.exists, "the sheet, which shows no notice of its own, hides the listing's")
    // Dragged down from its top, the half-height sheet goes.
    let top = explanation.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0))
      .withOffset(CGVector(dx: 0, dy: -80))
    top.press(forDuration: 0.1, thenDragTo: app.point(across: 0.5, down: 0.98))
    XCTAssertTrue(explanation.waitForNonExistence(timeout: 10), "the sheet went")
    XCTAssertTrue(said.waitForExistence(timeout: 10), "and the listing shows it again")
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
    attach("The workstation said once to be newer, beneath the bar")

    banner.tap()
    XCTAssertTrue(banner.waitForNonExistence(timeout: 10), "tapped away")
    // Opening a Session and coming back says it no more.
    let row = app.descendants(matching: .any).matching(identifier: "listing.active").firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    row.tap()
    XCTAssertTrue(app.back.waitForExistence(timeout: 10))
    app.back.tap()
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    XCTAssertFalse(banner.exists, "never said again")
  }

  @MainActor
  func testANoticeOutlastsASheetOverItAndGoesOnceItHasStoodOnScreen() {
    // Long enough that the tap that brings the sheet back and the drag that takes it away are a part of it alone.
    let stands: TimeInterval = 30
    // Opened on a Session whose Approval presents itself, so a sheet that shows no notice of its own soon covers this
    // one and one tap covers it again. It may show a moment before the sheet first comes, which is a part of its while.
    let app = launched("releases", noticeStands: stands, arguments: ["--open", "workstation-key/3"])
    let sheet = app.descendants(matching: .any)["approval.sheet"]
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    let banner = notice(app)
    XCTAssertFalse(banner.exists, "the sheet hides it")

    // What is left of its while runs from no sooner than the sheet is swiped away, and from no later than the notice is
    // next seen, so it would be over, whatever was left, its whole while after that.
    let uncovered = Date()
    app.swipeSheetAway()
    XCTAssertTrue(banner.waitForExistence(timeout: 10))
    let outstood = Date().addingTimeInterval(stands + 1)
    app.descendants(matching: .any)["session.review"].tap()
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    XCTAssertLessThan(
      Date(), uncovered.addingTimeInterval(stands - 10), "the sheet hid it with time to spare in its while")
    // Until it would have outstood its while on screen, beneath the sheet.
    let outlasted = XCTNSPredicateExpectation(
      predicate: NSPredicate { _, _ in Date() >= outstood }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [outlasted], timeout: stands + 2), .completed)
    app.swipeSheetAway()
    XCTAssertTrue(banner.waitForExistence(timeout: 10), "not seen while the sheet hid it")
    attach("The notice still standing once the sheet goes")
    XCTAssertTrue(banner.waitForNonExistence(timeout: stands + 5), "and gone once it has stood on screen")
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
    attach("A session on the laptop, which cannot be read until the app is updated")
  }
}
