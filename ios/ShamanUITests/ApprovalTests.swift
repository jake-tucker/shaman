import XCTest

/// An open Session's own Approval over the fake: presenting itself as a half-height sheet that takes no tap for a
/// moment, answered there, swiped away to wait behind the orange bar's "Review", and several presented oldest first,
/// one by one.
final class ApprovalTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  /// The app over the fake `scenario` with the Session `session` open, its Interventions taking no tap for
  /// `arming` seconds after appearing.
  @MainActor
  private func launched(_ scenario: String, opening session: String, arming: Double = 0.6) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--open", session, "--arming-moment", String(arming)]
    app.launch()
    return app
  }

  @MainActor
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any)[identifier]
  }

  /// Waits until an element `identifier` names stands as `format` says.
  @MainActor
  private func waitForOne(_ identifier: String, _ format: String, in app: XCUIApplication) {
    XCTAssertTrue(
      app.descendants(matching: .any).matching(identifier: identifier)
        .waitForOne(NSPredicate(format: format)),
      "\(identifier): \(format)")
  }

  @MainActor
  private func facts(in app: XCUIApplication) -> [String] {
    app.descendants(matching: .any).matching(identifier: "approval.fact").allElementsBoundByIndex.map(\.label)
  }

  @MainActor
  func testAnApprovalPresentsItselfTakesNoTapForAMomentAndIsAnsweredThere() {
    let app = launched("paired", opening: "workstation-key/3", arming: 5)
    // Looked at the moment it appears, before anything slower is asked of the screen.
    let accept = element("approval.acceptOnce", in: app)
    XCTAssertTrue(accept.waitForExistence(timeout: 10))
    XCTAssertFalse(accept.isEnabled, "it takes no tap for a moment after it appears")
    let title = element("approval.title", in: app)
    XCTAssertEqual(title.label, "Run this command?")
    XCTAssertEqual(
      facts(in: app),
      [
        "Command, cargo nextest run -p shaman-relay",
        "Directory, ~/Projects/shaman/.worktrees/peer-version-floor",
        "Reason, Needs network access outside the sandbox.",
      ])
    XCTAssertEqual(accept.label, "Accept once")
    XCTAssertEqual(element("approval.decline", in: app).label, "Decline")
    XCTAssertEqual(element("approval.declineAndInterrupt", in: app).label, "Decline and interrupt")
    XCTAssertEqual(element("approval.acceptForSession", in: app).label, "Accept for session")
    attach("An approval presenting itself")

    waitForOne("approval.acceptOnce", "isEnabled == true", in: app)
    accept.tap()
    XCTAssertTrue(
      element("approval.deciding", in: app).waitForExistence(timeout: 2), "the sheet is dimmed until the server answers"
    )
    XCTAssertFalse(accept.isEnabled)
    attach("A decision on its way")

    let sheet = element("approval.sheet", in: app)
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 20), "answered, the sheet goes")
    waitForOne("session.turn", "label BEGINSWITH 'Working'", in: app)
    XCTAssertEqual(element("session.barWords", in: app).label, "Working")
    XCTAssertFalse(element("session.review", in: app).exists)
    attach("The approval answered, its turn working again")
  }

  @MainActor
  func testSwipingAnApprovalAwayLeavesAnOrangeBarWhoseReviewBringsItBack() {
    let app = launched("paired", opening: "workstation-key/3")
    let sheet = element("approval.sheet", in: app)
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    app.swipeSheetAway()
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 20), "swiped away, it goes")

    XCTAssertEqual(element("session.barWords", in: app).label, "Approval waiting")
    let review = element("session.review", in: app)
    XCTAssertTrue(review.exists)
    XCTAssertEqual(review.label, "Review")
    let card = app.descendants(matching: .any).matching(identifier: "session.turn").firstMatch
    XCTAssertTrue(card.label.contains("Waiting on you"), card.label)
    attach("An approval dismissed, waiting behind the orange bar")

    review.tap()
    XCTAssertTrue(sheet.waitForExistence(timeout: 5))
    XCTAssertEqual(element("approval.title", in: app).label, "Run this command?")
  }

  @MainActor
  func testSeveralApprovalsArePresentedOldestFirstOneByOne() {
    let app = launched("several", opening: "laptop-key/4")
    let title = element("approval.title", in: app)
    XCTAssertTrue(title.waitForExistence(timeout: 10))
    XCTAssertEqual(title.label, "Make this file change?", "the oldest first")
    waitForOne("approval.decline", "isEnabled == true", in: app)
    element("approval.decline", in: app).tap()

    waitForOne("approval.title", "label == 'Use this tool?'", in: app)
    XCTAssertEqual(facts(in: app), ["Other tool, figma/export_variables"])
    let input = element("approval.section", in: app)
    XCTAssertTrue(input.label.hasPrefix("Input"), input.label)
    attach("The next approval, presented once the first was answered")

    waitForOne("approval.acceptForSession", "isEnabled == true", in: app)
    element("approval.acceptForSession", in: app).tap()
    XCTAssertTrue(element("approval.sheet", in: app).waitForNonExistence(timeout: 20), "answered, the last sheet goes")
    waitForOne("session.barWords", "label == 'Working'", in: app)
  }
}
