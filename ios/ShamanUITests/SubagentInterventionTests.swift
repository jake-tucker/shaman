import XCTest

/// A Subagent's Intervention over the fake: it never presents itself over a Session that does not own it. The parent's
/// bar turns orange and says who needs the reader, once the parent's own is dismissed, and "Review" pushes that
/// Subagent's Session, where its sheet presents itself and the reader stays once it is answered; where several wait,
/// "Review" opens the tree, each entry saying what it waits on; and a Subagent's row among a Turn's work says so too.
final class SubagentInterventionTests: XCTestCase {
  /// The Session heading the tree, whose Agent delegated to Explorer and the rest.
  private let delegating = "workstation-key/12"

  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--open", delegating, "--arming-moment", "0.3"]
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

  /// Waits until the bar reads `words`.
  @MainActor
  private func bar(_ words: String, in app: XCUIApplication) {
    let said = app.staticTexts.matching(identifier: "session.barWords")
      .matching(NSPredicate(format: "label == %@", words)).firstMatch
    XCTAssertTrue(said.waitForExistence(timeout: 10), "the bar reads \(words)")
  }

  @MainActor
  func testASubagentsApprovalMarksTheParentsBarOnceItsOwnIsDismissedAndReviewLeadsToItsSession() {
    let app = launched("subagentWaits")

    // The parent's own comes first, as its sheet.
    let sheet = app.otherElements["approval.sheet"]
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    XCTAssertTrue(element("approval.fact", holding: "git stash list", in: app).exists, "the parent's own")
    app.swipeSheetAway()
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))

    // Dismissed, the Subagent's shows on the bar, and no sheet presents itself for it.
    bar("Explorer needs approval", in: app)
    XCTAssertFalse(sheet.waitForExistence(timeout: 2), "no sheet over a session for one not its own")
    attach("A subagent's approval on the parent's bar")

    // Its row among the Turn's work says so.
    elements("session.turn", in: app).firstMatch.tap()
    let row = element("work.row", holding: "Explorer", in: app)
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    XCTAssertTrue(row.label.contains("Needs approval"), row.label)
    attach("A subagent's row needing the reader")
    app.back.tap()

    // Review pushes the Subagent's Session, where its sheet presents itself.
    let review = app.buttons["session.review"]
    XCTAssertTrue(review.waitForExistence(timeout: 10))
    review.tap()
    titled("Find what held the lock", in: app)
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    XCTAssertTrue(element("approval.fact", holding: "rg session.lock", in: app).exists, "the Subagent's own")
    attach("A subagent's approval in its own session")

    // Answered there, the reader stays there.
    let accept = app.buttons["approval.acceptOnce"]
    XCTAssertTrue(accept.waitForExistence(timeout: 5))
    expectation(for: NSPredicate(format: "isEnabled == true"), evaluatedWith: accept)
    waitForExpectations(timeout: 5)
    accept.tap()
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
    titled("Find what held the lock", in: app)
    XCTAssertFalse(sheet.waitForExistence(timeout: 2))
    XCTAssertFalse(app.buttons["session.review"].exists)
    attach("After answering in a subagent's session")

    // Back at the parent, its own waits behind Review again.
    app.back.tap()
    titled("Find what holds the session lock", in: app)
    bar("Approval waiting", in: app)
  }

  @MainActor
  func testSeveralSubagentsWaitingAreCountedAndReviewOpensTheTreeSayingWhatEachWaitsOn() {
    let app = launched("subagentsWait")
    bar("2 subagents need you", in: app)
    XCTAssertFalse(app.otherElements["approval.sheet"].exists)
    XCTAssertFalse(app.otherElements["questionnaire.sheet"].exists)
    attach("Two subagents waiting on the parent's bar")

    // Explorer's row says what waits in it and beneath it.
    elements("session.turn", in: app).firstMatch.tap()
    let row = element("work.row", holding: "Explorer", in: app)
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    XCTAssertTrue(row.label.contains("Needs you"), row.label)
    app.back.tap()

    app.buttons["session.review"].tap()
    let explorer = element("tree.row", holding: "Find what held the lock", in: app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 10))
    XCTAssertTrue(explorer.label.contains("Needs approval"), explorer.label)
    let grepper = element("tree.row", holding: "Grep the logs for the lock", in: app)
    XCTAssertTrue(grepper.label.contains("Needs an answer"), grepper.label)
    attach("The tree saying what each subagent waits on")

    // Chosen there, its sheet presents itself in its own Session.
    grepper.tap()
    titled("Grep the logs for the lock", in: app)
    XCTAssertTrue(app.otherElements["questionnaire.sheet"].waitForExistence(timeout: 10))
    attach("A subagent's questionnaire in its own session")
  }

  /// Asserts the element `identifier` names stands whole within the screen.
  @MainActor
  private func onScreen(_ identifier: String, in app: XCUIApplication, _ said: String) {
    let element = elements(identifier, in: app).firstMatch
    XCTAssertTrue(element.waitForExistence(timeout: 10), "\(said) stands")
    let screen = app.window.frame
    XCTAssertTrue(screen.contains(element.frame), "\(said) stands on screen: \(element.frame) in \(screen)")
    XCTAssertTrue(element.isHittable, "\(said) takes a tap")
  }

  @MainActor
  func testABarNamingASubagentByALongNameCutsTheNameShortAndKeepsReviewStopAndSendOnScreen() {
    let app = launched("longSubagentName")
    let words = elements("session.barWords", in: app)
      .matching(NSPredicate(format: "label ENDSWITH %@", "needs approval")).firstMatch
    XCTAssertTrue(words.waitForExistence(timeout: 10))
    onScreen("session.review", in: app, "Review")
    onScreen("session.stop", in: app, "Stop")
    onScreen("session.send", in: app, "Send")
    XCTAssertTrue(app.window.frame.contains(words.frame), "the words are cut short within the bar")
    attach("A bar naming a subagent by a long name")

    // Answered in its Session, the parent waits on two; with one stopped, the bar names the one left.
    app.buttons["session.review"].tap()
    let accept = app.buttons["approval.acceptOnce"]
    XCTAssertTrue(accept.waitForExistence(timeout: 10))
    expectation(for: NSPredicate(format: "isEnabled == true"), evaluatedWith: accept)
    waitForExpectations(timeout: 5)
    accept.tap()
    XCTAssertTrue(app.otherElements["approval.sheet"].waitForNonExistence(timeout: 10))
    app.back.tap()
    titled("Find what holds the session lock", in: app)
    bar("Waiting on 2 subagents", in: app)
    app.buttons["session.tree"].tap()
    let tester = app.cells.containing(NSPredicate(format: "label CONTAINS %@", "Run the lock's tests")).firstMatch
    XCTAssertTrue(tester.waitForExistence(timeout: 10))
    tester.swipeLeft()
    XCTAssertTrue(app.buttons["tree.stop"].waitForExistence(timeout: 5))
    app.buttons["tree.stop"].tap()
    XCTAssertTrue(element("tree.row", holding: "haiku · Stopped", in: app).waitForExistence(timeout: 10))
    // The open one chosen only closes the sheet.
    element("tree.row", holding: "Find what holds the session lock", in: app).tap()
    XCTAssertTrue(app.otherElements["tree.sheet"].waitForNonExistence(timeout: 10))
    let waiting = elements("session.barWords", in: app)
      .matching(NSPredicate(format: "label BEGINSWITH %@", "Waiting on security")).firstMatch
    XCTAssertTrue(waiting.waitForExistence(timeout: 10))
    onScreen("session.stop", in: app, "Stop")
    onScreen("session.send", in: app, "Send")
    attach("A bar waiting on a subagent by a long name")
  }
}
