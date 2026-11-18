import XCTest

/// The listing of every Remote's Sessions, over the fake: one Remote, and several standing together.
final class ListingTests: XCTestCase {
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
  private func attach(_ app: XCUIApplication, _ name: String) {
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = name
    screenshot.lifetime = .keepAlways
    add(screenshot)
  }

  @MainActor
  private func rows(_ shelf: String, in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: "listing.\(shelf)")
  }

  /// The row of `shelf` whose label holds `title`.
  @MainActor
  private func row(_ shelf: String, titled title: String, in app: XCUIApplication) -> XCUIElement {
    rows(shelf, in: app).matching(NSPredicate(format: "label CONTAINS %@", title)).firstMatch
  }

  /// Scrolls the listing until `element` can be tapped.
  @MainActor
  private func scrolled(to element: XCUIElement, in app: XCUIApplication) -> XCUIElement {
    var swipes = 0
    while !(element.exists && element.isHittable) && swipes < 12 {
      app.swipeUp()
      swipes += 1
    }
    XCTAssertTrue(element.isHittable, "scrolled to \(element)")
    return element
  }

  @MainActor
  func testSeveralRemotesSessionsStandTogetherEachNamingItsOrigin() {
    let app = launched("several")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    let first = rows("active", in: app).firstMatch
    XCTAssertTrue(first.waitForExistence(timeout: 10))
    XCTAssertTrue(first.label.contains("workstation · shaman"), first.label)
    XCTAssertTrue(first.label.contains("Draw the invite as a QR code in /serve"), first.label)
    XCTAssertTrue(first.label.contains("Working"), first.label)
    XCTAssertTrue(first.label.contains("feature/serve-qr"), first.label)
    XCTAssertTrue(row("active", titled: "homelab · home-assistant", in: app).exists)
    XCTAssertTrue(row("active", titled: "laptop · design-system", in: app).exists)

    XCTAssertTrue(app.buttons["home.servers"].exists)
    XCTAssertTrue(app.buttons["home.compose"].exists)
    XCTAssertEqual(app.descendants(matching: .any)["home.beginsIn"].label, "New sessions begin in workstation")
    attach(app, "Several servers' sessions in one listing")
  }

  @MainActor
  func testAPhoneHoldingOneRemoteNamesNoOrigin() {
    let app = launched("paired")
    XCTAssertTrue(app.navigationBars["All workspaces"].waitForExistence(timeout: 10))
    let active = rows("active", in: app)
    XCTAssertTrue(active.firstMatch.waitForExistence(timeout: 10))
    for index in 0..<active.count {
      let label = active.element(boundBy: index).label
      XCTAssertFalse(label.contains("workstation"), label)
    }
    XCTAssertTrue(row("active", titled: "shaman", in: app).exists)
    XCTAssertEqual(app.descendants(matching: .any)["home.beginsIn"].label, "New sessions begin in workstation")
    attach(app, "One server's sessions, naming no origin")
  }

  @MainActor
  func testSwipingARowSettlesItOntoTheShelfAndSwipingItThereBringsItBack() {
    let app = launched("paired")
    let title = "Draw the invite as a QR code in /serve"
    let active = row("active", titled: title, in: app)
    XCTAssertTrue(active.waitForExistence(timeout: 10))
    active.swipeLeft()
    app.buttons["Settle"].tap()

    let settled = row("settled", titled: title, in: app)
    XCTAssertTrue(settled.waitForExistence(timeout: 10), "settled onto the shelf")
    XCTAssertFalse(active.exists, "gone from the active rows")
    XCTAssertTrue(scrolled(to: settled, in: app).isHittable)
    XCTAssertTrue(app.staticTexts["listing.settledHeader"].exists)
    XCTAssertEqual(app.staticTexts["listing.settledHeader"].label, "Settled")
    attach(app, "A row settled onto the shelf")

    settled.swipeLeft()
    app.buttons["Unsettle"].tap()
    XCTAssertTrue(active.waitForExistence(timeout: 10), "brought back among the active")
    XCTAssertFalse(settled.exists)
  }

  @MainActor
  func testTheShelfShowsItsFirstRowsAndTheRestBehindShowMore() {
    let app = launched("paired")
    XCTAssertTrue(rows("active", in: app).firstMatch.waitForExistence(timeout: 10))
    let showMore = scrolled(to: app.buttons["listing.showMore"], in: app)
    XCTAssertEqual(showMore.label, "Show 2 more")
    let hidden = row("settled", titled: "Prune the stale worktrees", in: app)
    XCTAssertFalse(hidden.exists, "the last of the shelf stands behind Show more")
    attach(app, "The shelf, the rest behind Show more")

    showMore.tap()
    XCTAssertTrue(hidden.waitForExistence(timeout: 10))
    XCTAssertFalse(app.buttons["listing.showMore"].exists, "the shelf is whole")
  }

  @MainActor
  func testARemoteYetToAnswerIsSaidToBeReachedAtTheFootOfTheActiveRows() {
    let app = launched("reaching")
    let waiting = app.descendants(matching: .any)["listing.waiting"]
    XCTAssertTrue(scrolled(to: waiting, in: app).exists)
    XCTAssertTrue(waiting.label.contains("Reaching homelab…"), waiting.label)
    XCTAssertFalse(row("active", titled: "homelab ·", in: app).exists, "nothing of it is listed yet")
    XCTAssertFalse(app.descendants(matching: .any)["listing.empty"].exists)
    attach(app, "A remote yet to answer")
  }
}
