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
  private func wait(for format: String, _ element: XCUIElement, timeout: TimeInterval = 10) {
    expectation(for: NSPredicate(format: format), evaluatedWith: element)
    waitForExpectations(timeout: timeout)
  }

  /// Swipes the sheet away by its grabber, as a reader does.
  @MainActor
  private func swipeAway(in app: XCUIApplication) {
    let grabber = app.buttons["Sheet Grabber"]
    XCTAssertTrue(grabber.waitForExistence(timeout: 5))
    grabber.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.99)))
  }

  @MainActor
  private func facts(in app: XCUIApplication) -> [String] {
    app.descendants(matching: .any).matching(identifier: "approval.fact").allElementsBoundByIndex.map(\.label)
  }

  @MainActor
  func testAnApprovalPresentsItselfTakesNoTapForAMomentAndIsAnsweredThere() {
    let app = launched("paired", opening: "workstation-key/3", arming: 3)
    let title = element("approval.title", in: app)
    XCTAssertTrue(title.waitForExistence(timeout: 10))
    XCTAssertEqual(title.label, "Run this command?")
    XCTAssertEqual(
      facts(in: app),
      [
        "Command, cargo nextest run -p shaman-relay",
        "Directory, ~/Projects/shaman/.worktrees/peer-version-floor",
        "Reason, Needs network access outside the sandbox.",
      ])
    let accept = element("approval.acceptOnce", in: app)
    XCTAssertEqual(accept.label, "Accept once")
    XCTAssertEqual(element("approval.decline", in: app).label, "Decline")
    XCTAssertEqual(element("approval.declineAndInterrupt", in: app).label, "Decline and interrupt")
    XCTAssertEqual(element("approval.acceptForSession", in: app).label, "Accept for session")
    XCTAssertFalse(accept.isEnabled, "it takes no tap for a moment after it appears")
    attach(app, "An approval presenting itself")

    wait(for: "isEnabled == true", accept)
    accept.tap()
    XCTAssertTrue(
      element("approval.deciding", in: app).waitForExistence(timeout: 2), "the sheet is dimmed until the server answers"
    )
    XCTAssertFalse(accept.isEnabled)
    attach(app, "A decision on its way")

    let sheet = element("approval.sheet", in: app)
    wait(for: "exists == false", sheet)
    let card = app.descendants(matching: .any).matching(identifier: "session.turn").firstMatch
    wait(for: "label BEGINSWITH 'Working'", card)
    XCTAssertEqual(element("session.barWords", in: app).label, "Working")
    XCTAssertFalse(element("session.review", in: app).exists)
    attach(app, "The approval answered, its turn working again")
  }

  @MainActor
  func testSwipingAnApprovalAwayLeavesAnOrangeBarWhoseReviewBringsItBack() {
    let app = launched("paired", opening: "workstation-key/3")
    let sheet = element("approval.sheet", in: app)
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    swipeAway(in: app)
    wait(for: "exists == false", sheet)

    XCTAssertEqual(element("session.barWords", in: app).label, "Approval waiting")
    let review = element("session.review", in: app)
    XCTAssertTrue(review.exists)
    XCTAssertEqual(review.label, "Review")
    let card = app.descendants(matching: .any).matching(identifier: "session.turn").firstMatch
    XCTAssertTrue(card.label.contains("Waiting on you"), card.label)
    attach(app, "An approval dismissed, waiting behind the orange bar")

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
    let decline = element("approval.decline", in: app)
    wait(for: "isEnabled == true", decline)
    decline.tap()

    wait(for: "label == 'Use this tool?'", title)
    XCTAssertEqual(facts(in: app), ["Other tool, figma/export_variables"])
    let input = element("approval.section", in: app)
    XCTAssertTrue(input.label.hasPrefix("Input"), input.label)
    attach(app, "The next approval, presented once the first was answered")

    let acceptForSession = element("approval.acceptForSession", in: app)
    wait(for: "isEnabled == true", acceptForSession)
    acceptForSession.tap()
    wait(for: "exists == false", element("approval.sheet", in: app))
    XCTAssertEqual(element("session.barWords", in: app).label, "Working")
  }
}
