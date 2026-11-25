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
  private func rows(_ shelf: String, in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: "listing.\(shelf)")
  }

  /// The row of `shelf` whose label holds `title`.
  @MainActor
  private func row(_ shelf: String, titled title: String, in app: XCUIApplication) -> XCUIElement {
    rows(shelf, in: app).matching(NSPredicate(format: "label CONTAINS %@", title)).firstMatch
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

    let servers = app.buttons["home.servers"]
    let compose = app.buttons["home.compose"]
    XCTAssertTrue(servers.exists)
    XCTAssertTrue(compose.exists)
    // At the foot, "New session" lowest where the bar is vertical and last where it is not.
    XCTAssertGreaterThan(servers.frame.midY, app.window.frame.midY)
    if abs(compose.frame.midX - servers.frame.midX) < 2 {
      XCTAssertGreaterThan(compose.frame.minY, servers.frame.maxY, "lowest")
    } else {
      XCTAssertEqual(compose.frame.midY, servers.frame.midY, accuracy: 2, "level with it")
      XCTAssertGreaterThan(compose.frame.minX, servers.frame.maxX, "last")
    }
    XCTAssertEqual(app.descendants(matching: .any)["home.beginsIn"].label, "New sessions begin in workstation")
    attach("Several servers' sessions in one listing")
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
    attach("One server's sessions, naming no origin")
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
    XCTAssertTrue(active.waitForNonExistence(timeout: 10), "gone from the active rows")
    // The shelf stands beneath every active row, further down than the listing draws at first.
    XCTAssertTrue(app.scrolled(to: settled).isHittable, "settled onto the shelf")
    XCTAssertTrue(app.staticTexts["listing.settledHeader"].exists)
    XCTAssertEqual(app.staticTexts["listing.settledHeader"].label, "Settled")
    attach("A row settled onto the shelf")

    // Clear of the bottom bar, so the action the swipe reveals takes the tap.
    if settled.frame.maxY > app.window.frame.height * 0.75 {
      app.point(across: 0.5, down: 0.6)
        .press(forDuration: 0.05, thenDragTo: app.point(across: 0.5, down: 0.45))
    }
    settled.swipeLeft()
    app.buttons["Unsettle"].tap()
    XCTAssertTrue(settled.waitForNonExistence(timeout: 10), "gone from the shelf")
    // Back at the head of the active rows, above where the shelf was scrolled to.
    for _ in 0..<3 where !active.exists {
      app.window.swipeDown()
    }
    XCTAssertTrue(active.waitForExistence(timeout: 10), "brought back among the active")
    XCTAssertFalse(settled.exists)
  }

  @MainActor
  func testTheShelfShowsItsFirstRowsAndTheRestBehindShowMore() {
    let app = launched("paired")
    XCTAssertTrue(rows("active", in: app).firstMatch.waitForExistence(timeout: 10))
    // By its words, as at the foot of the listing on a closed Duo the capsule floats over the row, which the user found
    // fine on the phone, and takes a tap in its middle.
    let showMore = app.scrolled(to: app.buttons["listing.showMore"], at: 0.1)
    XCTAssertEqual(showMore.label, "Show 2 more")
    let hidden = row("settled", titled: "Prune the stale worktrees", in: app)
    XCTAssertFalse(hidden.exists, "the last of the shelf stands behind Show more")
    attach("The shelf, the rest behind Show more")

    showMore.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5)).tap()
    app.scrolled(to: hidden, at: 0.1)
    XCTAssertFalse(app.buttons["listing.showMore"].exists, "the shelf is whole")
  }

  @MainActor
  func testARemoteYetToAnswerIsSaidToBeReachedAtTheFootOfTheActiveRows() {
    let app = launched("reaching")
    XCTAssertTrue(rows("active", in: app).firstMatch.waitForExistence(timeout: 10))
    let waiting = app.descendants(matching: .any)["listing.waiting"]
    XCTAssertTrue(app.scrolled(to: waiting).exists)
    XCTAssertTrue(waiting.label.contains("Reaching homelab…"), waiting.label)
    XCTAssertFalse(row("active", titled: "homelab ·", in: app).exists, "nothing of it is listed yet")
    XCTAssertFalse(app.descendants(matching: .any)["listing.empty"].exists)
    attach("A remote yet to answer")
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
    attach("The title's menu with several servers")
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
    attach("The title's menu with one server")
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
    attach("Narrowed to a workspace, the outlook turned")

    choose("All of laptop", in: app)
    XCTAssertTrue(app.navigationBars["laptop"].waitForExistence(timeout: 10))
    XCTAssertTrue(row("active", titled: "laptop · design-system", in: app).waitForExistence(timeout: 10))
    XCTAssertFalse(row("active", titled: "homelab ·", in: app).exists)
    XCTAssertEqual(beginsIn(app), "New sessions begin in laptop")
    attach("Narrowed to a server, the outlook turned")

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
    XCTAssertTrue(app.scrolled(to: stale).exists, "the whole shelf is searched, past Show more")
    XCTAssertFalse(app.staticTexts["listing.settledHeader"].exists, "the shelves stand down")
    XCTAssertFalse(app.buttons["listing.showMore"].exists)
    attach("A search, one flat list")

    field.typeText("nothing like it")
    let empty = app.staticTexts["listing.empty"]
    XCTAssertTrue(empty.waitForExistence(timeout: 10))
    XCTAssertEqual(empty.label, "No sessions match")
    attach("A search that finds nothing")
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
    attach("A search while a remote is reached")
  }
}
