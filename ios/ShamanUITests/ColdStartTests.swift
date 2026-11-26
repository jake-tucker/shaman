import XCTest

/// A cold start over the fake: after iOS ends the app while it is suspended, the next launch returns to what was open,
/// fetched afresh, through what the scene kept: where the window's Outlook stood, the listing's scope and the Sessions
/// pushed over it, a Subagent's over its parent included, or the new-session screen. Sheets and the screens of a Turn's work and an Activity are not
/// restored, and what is gone falls back to the listing, or to Everywhere for a scope.
final class ColdStartTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String, _ arguments: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario] + arguments
    app.launch()
    // Whatever a test leaves open, the next one starts from a new scene's listing.
    addTeardownBlock { @MainActor in
      let forgetting = XCUIApplication()
      forgetting.launchArguments = ["--fake", "paired", "--forget-scene"]
      forgetting.launch()
      XCTAssertTrue(forgetting.buttons["home.compose"].waitForExistence(timeout: 10))
      self.ended(forgetting)
    }
    return app
  }

  /// Sends `app` to the background, where the scene keeps what is open, and has it ended there, as iOS ends a
  /// suspended app.
  @MainActor
  private func ended(_ app: XCUIApplication) {
    app.leaveTheFront()
    // iOS writes what the scene keeps a moment after the app leaves the front.
    _ = XCTWaiter.wait(for: [XCTestExpectation(description: "the scene kept")], timeout: 2)
    app.terminate()
  }

  /// `app` ended as iOS ends a suspended app, and launched again over the fake's `scenario`, asking for nothing to be
  /// open.
  @MainActor
  private func relaunched(_ app: XCUIApplication, _ scenario: String) -> XCUIApplication {
    ended(app)
    app.launchArguments = ["--fake", scenario]
    app.launch()
    return app
  }

  @MainActor
  private func elements(_ identifier: String, in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: identifier)
  }

  /// The element `identifier` names whose label holds `text`.
  @MainActor
  private func element(_ identifier: String, holding text: String, in app: XCUIApplication) -> XCUIElement {
    elements(identifier, in: app).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
  }

  /// Waits until the open Session's Title reads `expected`.
  @MainActor
  private func titled(_ expected: String, in app: XCUIApplication) {
    let title = app.staticTexts.matching(identifier: "session.title")
      .matching(NSPredicate(format: "label == %@", expected)).firstMatch
    XCTAssertTrue(title.waitForExistence(timeout: 10), "the Session titled \(expected) shows")
  }

  /// Waits until the listing is titled `words`, as its scope titles it.
  @MainActor
  private func listing(at words: String, in app: XCUIApplication) {
    let menu = app.buttons["listing.scope"]
    XCTAssertTrue(menu.waitForExistence(timeout: 10), "the listing shows")
    let at = NSPredicate(format: "label == %@", words)
    XCTAssertTrue(
      XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: at, object: menu)], timeout: 10) == .completed,
      "the listing stands at \(words), not \(menu.label)")
  }

  /// Opens the title's menu and chooses `words` in it, which titles the listing `title`.
  @MainActor
  private func choose(_ words: String, titled title: String, in app: XCUIApplication) {
    app.buttons["listing.scope"].tap()
    let choice = app.buttons[words].firstMatch
    XCTAssertTrue(choice.waitForExistence(timeout: 10), "the menu offers \(words)")
    choice.tap()
    listing(at: title, in: app)
  }

  @MainActor
  private func back(in app: XCUIApplication) {
    app.back.tap()
  }

  @MainActor
  func testTheSessionsPushedASubagentsOverItsParentIncludedAreOpenAgainButNoSheetOrWork() {
    var app = launched("paired")
    XCTAssertTrue(elements("listing.active", in: app).firstMatch.waitForExistence(timeout: 10))
    app.scrolled(to: element("listing.active", holding: "Find what holds the session lock", in: app)).tap()
    titled("Find what holds the session lock", in: app)
    let card = elements("session.turn", in: app).firstMatch
    XCTAssertTrue(card.waitForExistence(timeout: 10))
    card.tap()
    let explorer = element("work.row", holding: "Explorer", in: app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 10))
    explorer.tap()
    titled("Find what held the lock", in: app)
    // Its Turn's work pushed over it, and the tree's sheet up, neither of which comes back.
    elements("session.turn", in: app).firstMatch.tap()
    XCTAssertTrue(element("work.row", holding: "Grepper", in: app).waitForExistence(timeout: 10))
    back(in: app)
    titled("Find what held the lock", in: app)
    let tree = app.buttons["session.tree"]
    XCTAssertTrue(tree.waitForExistence(timeout: 10))
    tree.tap()
    XCTAssertTrue(elements("tree.sheet", in: app).firstMatch.waitForExistence(timeout: 10))

    app = relaunched(app, "paired")
    titled("Find what held the lock", in: app)
    XCTAssertEqual(app.staticTexts["session.place"].label, "Explorer · haiku", "read afresh")
    XCTAssertTrue(elements("session.delegation", in: app).firstMatch.waitForExistence(timeout: 10))
    XCTAssertFalse(elements("tree.sheet", in: app).firstMatch.exists, "a sheet is not restored")
    attach("A subagent's session open again after a cold start")

    // Beneath it, its parent's Session, not the Turn's work it was pushed from; beneath that, the listing.
    back(in: app)
    titled("Find what holds the session lock", in: app)
    XCTAssertFalse(elements("work.list", in: app).firstMatch.exists, "a Turn's work is not restored")
    back(in: app)
    listing(at: "All workspaces", in: app)
  }

  @MainActor
  func testAnActivityOpenReturnsToItsSession() {
    var app = launched("paired", ["--open", "workstation-key/7"])
    titled("Tidy zsh startup time", in: app)
    let card = elements("session.turn", in: app).firstMatch
    XCTAssertTrue(card.waitForExistence(timeout: 10))
    card.tap()
    let row = elements("work.row", in: app).firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    row.tap()
    XCTAssertTrue(elements("detail.screen", in: app).firstMatch.waitForExistence(timeout: 10))

    app = relaunched(app, "paired")
    titled("Tidy zsh startup time", in: app)
    XCTAssertTrue(card.waitForExistence(timeout: 10))
    XCTAssertFalse(elements("detail.screen", in: app).firstMatch.exists, "an Activity's screen is not restored")
    XCTAssertFalse(elements("work.list", in: app).firstMatch.exists)
    attach("A session open again after a cold start, not the activity pushed over it")
  }

  @MainActor
  func testTheListingsScopeAndTheNewSessionScreenAreReturnedTo() {
    var app = launched("several")
    listing(at: "Everywhere", in: app)
    choose("All of laptop", titled: "laptop", in: app)
    app.buttons["home.compose"].tap()
    XCTAssertTrue(elements("landing.screen", in: app).firstMatch.waitForExistence(timeout: 10))

    app = relaunched(app, "several")
    XCTAssertTrue(elements("landing.screen", in: app).firstMatch.waitForExistence(timeout: 10))
    XCTAssertEqual(app.staticTexts["landing.title"].label, "New session")
    attach("The new-session screen open again after a cold start")
    back(in: app)
    listing(at: "laptop", in: app)
    XCTAssertTrue(element("listing.active", holding: "Token colours for the button palette", in: app).exists)
    XCTAssertFalse(element("listing.active", holding: "Draw the invite as a QR code", in: app).exists)
    attach("The listing narrowed to the laptop again after a cold start")

    // A Workspace's scope comes back the same way, and a sheet over the listing does not.
    choose("design-system", titled: "laptop · design-system", in: app)
    app.buttons["home.servers"].tap()
    XCTAssertTrue(app.staticTexts["Servers & device"].firstMatch.waitForExistence(timeout: 10))
    app = relaunched(app, "several")
    listing(at: "laptop · design-system", in: app)
    XCTAssertFalse(app.staticTexts["Servers & device"].firstMatch.exists, "a sheet is not restored")
  }

  /// Waits until the listing's capsule says new Sessions begin in `place`, where the Outlook stands.
  @MainActor
  private func begins(in place: String, in app: XCUIApplication) {
    let beginsIn = app.descendants(matching: .any)["home.beginsIn"]
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10), "the listing's capsule shows")
    let said = NSPredicate(format: "label == %@", "New sessions begin in \(place)")
    XCTAssertTrue(
      XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: said, object: beginsIn)], timeout: 10) == .completed,
      "new sessions begin in \(place), not as \(beginsIn.label) says")
  }

  @MainActor
  func testTheOutlookIsReturnedToBesideTheScopeAndOneWhoseRemoteIsGoneFallsToTheLatestHeld() {
    var app = launched("several")
    begins(in: "workstation", in: app)
    // A Workspace chosen turns the Outlook toward it; Everywhere chosen after turns nothing.
    choose("All of laptop", titled: "laptop", in: app)
    choose("design-system", titled: "laptop · design-system", in: app)
    choose("Everywhere", titled: "Everywhere", in: app)
    begins(in: "laptop · design-system", in: app)

    app = relaunched(app, "several")
    listing(at: "Everywhere", in: app)
    begins(in: "laptop · design-system", in: app)
    attach("The Outlook kept by the scene after a cold start, the listing under everywhere")

    // The `paired` phone holds no laptop: the Outlook stands where that phone was last turned.
    app = relaunched(app, "paired")
    listing(at: "All workspaces", in: app)
    begins(in: "workstation", in: app)
  }

  @MainActor
  func testWhatIsGoneFallsBackToTheListingAndAScopeGoneToEverywhere() {
    // The home lab the `several` phone holds is gone from the `paired` one.
    var app = launched("several")
    choose("relay-infra", titled: "homelab · relay-infra", in: app)
    let row = element("listing.active", holding: "Why the nightly backup skipped Tuesday", in: app)
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    row.tap()
    titled("Why the nightly backup skipped Tuesday", in: app)

    app = relaunched(app, "paired")
    listing(at: "All workspaces", in: app)
    XCTAssertFalse(elements("session.gone", in: app).firstMatch.exists, "back at the listing, not saying it is gone")
    XCTAssertTrue(element("listing.active", holding: "Draw the invite as a QR code", in: app).exists)
    attach("The listing under everywhere after the session and scope kept were gone")
  }

  /// Covers only the app's half of a launch after the user swipes the app away: a new scene, whose storage holds
  /// nothing, stood in for by `--forget-scene`, starts at the listing under Everywhere. Whether iOS gives the scene up
  /// on that swipe is iOS's half, which no test here reaches: the iOS 27.2 simulator keeps the scene's storage through
  /// a swipe away, and no scene API tells the app of one. That half is the spec's check on the phone, "Scene storage:
  /// back in the open Session after a system kill, at the listing after a swipe away" (#662, Checks on the phone, 5).
  @MainActor
  func testOnlyTheAppsHalfOfASwipeAwayANewSceneStartsAtTheListingUnderEverywhere() {
    var app = launched("several", ["--open", "workstation-key/1"])
    titled("Draw the invite as a QR code in /serve", in: app)
    ended(app)
    app = XCUIApplication()
    app.launchArguments = ["--fake", "several", "--forget-scene"]
    app.launch()
    listing(at: "Everywhere", in: app)
    XCTAssertFalse(app.staticTexts["session.title"].exists)
  }

  @MainActor
  func testAPhoneThatPairsAgainLandsOnItsNewListingAndNotOnWhatItKeptBefore() {
    // Kept while the phone held the workstation, under a scope of it.
    var app = launched("paired", ["--open", "workstation-key/1"])
    titled("Draw the invite as a QR code in /serve", in: app)
    back(in: app)
    choose("dotfiles", titled: "dotfiles", in: app)
    element("listing.active", holding: "Tidy zsh startup time", in: app).tap()
    titled("Tidy zsh startup time", in: app)
    ended(app)

    // Launched again holding no Remote, it pairs with the same Server.
    app = XCUIApplication()
    app.launchArguments = ["--fake", "fresh"]
    app.launchEnvironment["SHAMAN_INVITE"] = "shaman-v1-fake-workstation"
    app.launch()
    XCTAssertTrue(app.descendants(matching: .any)["confirm.hostname"].waitForExistence(timeout: 10))
    app.scrolled(to: app.buttons["confirm.pair"]).tap()
    listing(at: "All workspaces", in: app)
    XCTAssertFalse(app.staticTexts["session.title"].exists, "nothing of what was kept before is pushed")
    XCTAssertTrue(element("listing.active", holding: "Draw the invite as a QR code", in: app).exists)
    attach("Paired again, on the new listing")

    // Nor does a cold start after it bring back what was kept before the phone held no Remote.
    app = relaunched(app, "paired")
    listing(at: "All workspaces", in: app)
    XCTAssertFalse(app.staticTexts["session.title"].exists)
  }
}
