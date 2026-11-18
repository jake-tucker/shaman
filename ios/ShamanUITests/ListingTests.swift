import XCTest

/// The listing of every Remote's Sessions, over the fake: one Remote, and several standing together, narrowed by the
/// title's menu and searched by Title.
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

  @MainActor
  private func beginsIn(_ app: XCUIApplication) -> String {
    app.descendants(matching: .any)["home.beginsIn"].label
  }

  /// Opens the title's menu and chooses `words` in it.
  @MainActor
  private func choose(_ words: String, in app: XCUIApplication) {
    app.buttons["listing.scope"].tap()
    let choice = app.buttons[words].firstMatch
    XCTAssertTrue(choice.waitForExistence(timeout: 10), "the menu offers \(words)")
    choice.tap()
  }

  @MainActor
  func testTheTitleMenuOffersEverywhereThenEachRemoteWithAllOfItAndItsWorkspaces() {
    let app = launched("several")
    let menu = app.buttons["listing.scope"]
    XCTAssertTrue(menu.waitForExistence(timeout: 10))
    XCTAssertEqual(menu.label, "Everywhere", "the listing starts at Everywhere")
    menu.tap()
    for words in [
      "Everywhere", "All of workstation", "dotfiles", "notes", "All of laptop", "design-system",
      "All of homelab", "home-assistant", "relay-infra",
    ] {
      XCTAssertTrue(app.buttons[words].firstMatch.waitForExistence(timeout: 10), "offers \(words)")
    }
    XCTAssertFalse(app.buttons["Sidekick"].exists, "the Sidekick Workspace is never offered")
    for header in ["workstation", "laptop", "homelab"] {
      let headed = app.staticTexts.matching(NSPredicate(format: "label == %@", header)).firstMatch
      XCTAssertTrue(headed.exists, "a section is headed \(header)")
    }
    attach(app, "The title's menu with several servers")
  }

  @MainActor
  func testWithOneRemoteTheMenuOffersAllWorkspacesAndItsWorkspaces() {
    let app = launched("paired")
    let menu = app.buttons["listing.scope"]
    XCTAssertTrue(menu.waitForExistence(timeout: 10))
    XCTAssertEqual(menu.label, "All workspaces")
    menu.tap()
    for words in ["All workspaces", "shaman", "dotfiles", "notes"] {
      XCTAssertTrue(app.buttons[words].firstMatch.waitForExistence(timeout: 10), "offers \(words)")
    }
    XCTAssertFalse(app.buttons["All of workstation"].exists, "no Origin is named with one server")
    attach(app, "The title's menu with one server")
  }

  @MainActor
  func testChoosingARemoteOrAWorkspaceNarrowsTheListingAndTurnsTheOutlookAndEverywhereTurnsNothing() {
    let app = launched("several")
    XCTAssertTrue(rows("active", in: app).firstMatch.waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn(app), "New sessions begin in workstation")

    choose("relay-infra", in: app)
    XCTAssertTrue(app.navigationBars["homelab · relay-infra"].waitForExistence(timeout: 10))
    XCTAssertTrue(
      row("active", titled: "Why the nightly backup skipped Tuesday", in: app).waitForExistence(timeout: 10))
    XCTAssertFalse(row("active", titled: "Upgrade Home Assistant", in: app).exists, "another Workspace's")
    XCTAssertFalse(row("active", titled: "workstation ·", in: app).exists, "another Remote's")
    XCTAssertEqual(beginsIn(app), "New sessions begin in homelab · relay-infra")
    attach(app, "Narrowed to a workspace, the outlook turned")

    choose("All of laptop", in: app)
    XCTAssertTrue(app.navigationBars["laptop"].waitForExistence(timeout: 10))
    XCTAssertTrue(row("active", titled: "laptop · design-system", in: app).waitForExistence(timeout: 10))
    XCTAssertFalse(row("active", titled: "homelab ·", in: app).exists)
    XCTAssertEqual(beginsIn(app), "New sessions begin in laptop")
    attach(app, "Narrowed to a server, the outlook turned")

    choose("Everywhere", in: app)
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    XCTAssertTrue(row("active", titled: "homelab · home-assistant", in: app).waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn(app), "New sessions begin in laptop", "Everywhere turns nothing")

    choose("relay-infra", in: app)
    choose("All of homelab", in: app)
    XCTAssertTrue(app.navigationBars["homelab"].waitForExistence(timeout: 10))
    XCTAssertEqual(
      beginsIn(app), "New sessions begin in homelab · relay-infra", "the Workspace remembered for the server")
  }

  @MainActor
  func testASearchAnswersWithOneFlatListEachRowKeepingItsShelfsShape() {
    let app = launched("paired")
    XCTAssertTrue(rows("active", in: app).firstMatch.waitForExistence(timeout: 10))
    let field = app.searchFields.firstMatch
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    XCTAssertEqual(field.placeholderValue, "Search by title")
    field.tap()
    field.typeText("ZSH")
    XCTAssertTrue(row("active", titled: "Tidy zsh startup time", in: app).waitForExistence(timeout: 10))
    XCTAssertEqual(rows("active", in: app).count, 1)

    field.typeText(XCUIKeyboardKey.delete.rawValue)
    field.typeText(XCUIKeyboardKey.delete.rawValue)
    field.typeText(XCUIKeyboardKey.delete.rawValue)
    field.typeText("t")
    XCTAssertTrue(
      row("active", titled: "Draw the invite as a QR code", in: app).waitForExistence(timeout: 10),
      "active rows keep their shape")
    let stale = row("settled", titled: "Prune the stale worktrees", in: app)
    XCTAssertTrue(scrolled(to: stale, in: app).exists, "the whole shelf is searched, past Show more")
    XCTAssertFalse(app.staticTexts["listing.settledHeader"].exists, "the shelves stand down")
    XCTAssertFalse(app.buttons["listing.showMore"].exists)
    attach(app, "A search, one flat list")

    field.typeText("nothing like it")
    let empty = app.staticTexts["listing.empty"]
    XCTAssertTrue(empty.waitForExistence(timeout: 10))
    XCTAssertEqual(empty.label, "No sessions match")
    attach(app, "A search that finds nothing")
  }

  @MainActor
  func testASearchSaysItFoundNothingWhileItGoesOnReachingARemote() {
    let app = launched("reaching")
    XCTAssertTrue(rows("active", in: app).firstMatch.waitForExistence(timeout: 10))
    let field = app.searchFields.firstMatch
    field.tap()
    field.typeText("nothing like it")
    let empty = app.staticTexts["listing.empty"]
    XCTAssertTrue(empty.waitForExistence(timeout: 10))
    XCTAssertEqual(empty.label, "No sessions match")
    let waiting = app.descendants(matching: .any)["listing.waiting"]
    XCTAssertTrue(waiting.exists, "the Remote yet to answer is still said to be reached")
    XCTAssertTrue(waiting.label.contains("Reaching homelab…"), waiting.label)
    attach(app, "A search while a remote is reached")
  }
}
