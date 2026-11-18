import XCTest

/// An open Session over the fake: pushed from the listing, framed at once, its conversation landing in one piece and
/// reading as the core made it, for a Turn working, settled, stopped and failed.
final class SessionTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String, opening session: String? = nil) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario] + (session.map { ["--open", $0] } ?? [])
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
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any)[identifier]
  }

  @MainActor
  private func elements(_ identifier: String, in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: identifier)
  }

  /// The card of the conversation's last Turn.
  @MainActor
  private func lastCard(in app: XCUIApplication) -> XCUIElement {
    let cards = elements("session.turn", in: app)
    XCTAssertTrue(cards.firstMatch.waitForExistence(timeout: 10))
    return cards.element(boundBy: cards.count - 1)
  }

  @MainActor
  func testAWorkingSessionOpensFromTheListingReadsLiveAndPopsBack() {
    let app = launched("paired")
    let row = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "Draw the invite as a QR code")).firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    row.tap()

    XCTAssertTrue(element("session.title", in: app).waitForExistence(timeout: 5))
    XCTAssertEqual(element("session.title", in: app).label, "Draw the invite as a QR code in /serve")
    XCTAssertEqual(
      element("session.place", in: app).label, "shaman · feature/serve-qr",
      "with one Remote no Origin is named")
    let card = lastCard(in: app)
    XCTAssertTrue(card.label.contains("cargo nextest run qr"), card.label)
    XCTAssertTrue(card.label.contains("1 tool · 1 command · 1 edit"), card.label)
    XCTAssertFalse(element("session.loading", in: app).exists, "the conversation landed in its place")
    let bubbles = elements("session.userMessage", in: app)
    XCTAssertEqual(bubbles.element(boundBy: bubbles.count - 1).label, "Draw the invite as a QR code in /serve")
    let answers = elements("session.agentMessage", in: app)
    let answer = answers.element(boundBy: answers.count - 1)
    let streamed = NSPredicate(format: "label CONTAINS %@", "41 columns")
    expectation(for: streamed, evaluatedWith: answer)
    waitForExpectations(timeout: 10)
    attach(app, "A working session, its answer streamed in")

    app.navigationBars.buttons.firstMatch.tap()
    XCTAssertTrue(app.navigationBars["All workspaces"].waitForExistence(timeout: 5))
    XCTAssertEqual(
      element("home.beginsIn", in: app).label, "New sessions begin in workstation · shaman",
      "opening it turned the Outlook toward its Origin and Workspace")
  }

  @MainActor
  func testASessionNotYetReadIsFramedAtOnceWithAQuietMarkWhereItsConversationWillBe() {
    // The home lab has yet to answer, so its Session is never read.
    let app = launched("reaching", opening: "homelab-key/2")
    XCTAssertTrue(element("session.loading", in: app).waitForExistence(timeout: 10))
    XCTAssertEqual(element("session.title", in: app).label, "Upgrade Home Assistant to 2026.11")
    XCTAssertEqual(element("session.place", in: app).label, "homelab · home-assistant · main")
    XCTAssertFalse(element("session.turn", in: app).exists)
    attach(app, "A session framed at once, its conversation loading")
  }

  @MainActor
  func testASettledTurnReadsHowItSettledOverItsCountAndItsAnswerStandsOutside() {
    let app = launched("paired", opening: "workstation-key/7")
    let card = lastCard(in: app)
    XCTAssertTrue(card.label.contains("Worked for 3m 12s"), card.label)
    XCTAssertTrue(card.label.contains("3 tools · 3 commands · 2 edits"), card.label)
    XCTAssertEqual(
      element("session.compaction", in: app).label, "Compacted context · 180K → 40K (automatic)")
    XCTAssertEqual(element("session.model", in: app).label, "auto · gpt-5.5")
    attach(app, "A settled turn")
  }

  @MainActor
  func testAStoppedTurnReadsHowLongItRanAndItsSteeringMessageStandsOutside() {
    let app = launched("paired", opening: "workstation-key/8")
    let card = lastCard(in: app)
    XCTAssertTrue(card.label.contains("Stopped after 41s"), card.label)
    let bubbles = elements("session.userMessage", in: app)
    XCTAssertTrue(bubbles.element(boundBy: bubbles.count - 1).label.hasPrefix("Sorry, carry on."))
    attach(app, "A stopped turn")
  }

  @MainActor
  func testAFailedTurnsErrorStandsOutsideItsCardAndSeveralRemotesNameTheOrigin() {
    let app = launched("several", opening: "laptop-key/5")
    let card = lastCard(in: app)
    XCTAssertTrue(card.label.contains("Failed after 12s"), card.label)
    XCTAssertTrue(element("session.error", in: app).label.contains("The provider stopped answering"))
    XCTAssertEqual(element("session.place", in: app).label, "laptop · billing-api · fix/rounding-v2")
    attach(app, "A failed turn, the origin named")
  }
}
