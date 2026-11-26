import XCTest

/// An open Session's own Questionnaire over the fake: presenting itself as a full-height sheet, one Question to a
/// page, its choices cards with a "Recommended" badge, a field beside them, "Skip" where a Question is not required,
/// a secret's field secure and reading "Answered (secret)" when shown again, "Next" leading to "Send answers", and
/// "Decline" in the corner; swiped away, waiting behind the orange bar with what was half given kept, a secret written
/// reading "Answered (secret)" once reviewed; and a written answer in view above the foot as it grows over the keyboard.
final class QuestionnaireTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ more: [String] = [], arming: Double = 0.6) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments =
      ["--fake", "paired", "--open", "workstation-key/13", "--arming-moment", String(arming)] + more
    app.launch()
    return app
  }

  @MainActor
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any)[identifier]
  }

  @MainActor
  private func choices(in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: "questionnaire.choice")
  }

  @MainActor
  private func choice(_ label: String, in app: XCUIApplication) -> XCUIElement {
    choices(in: app).matching(NSPredicate(format: "label BEGINSWITH %@", label)).firstMatch
  }

  @MainActor
  private func wait(for format: String, _ element: XCUIElement, timeout: TimeInterval = 10) {
    expectation(for: NSPredicate(format: format), evaluatedWith: element)
    waitForExpectations(timeout: timeout)
  }

  /// The title of the sheet's page on show.
  @MainActor
  private func position(in app: XCUIApplication) -> XCUIElement {
    app.navigationBars.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Question '")).firstMatch
  }

  @MainActor
  func testAQuestionnaireIsAnsweredAPageAtATimeAndSentWhole() {
    // A moment long enough that a slow runner, which has taken longer than three seconds to find the sheet, sees it.
    let app = launched(arming: 15)
    let next = element("questionnaire.next", in: app)
    XCTAssertTrue(next.waitForExistence(timeout: 10))
    XCTAssertFalse(element("questionnaire.decline", in: app).isEnabled, "it takes no tap for a moment")
    XCTAssertEqual(position(in: app).label, "Question 1 of 3")
    XCTAssertEqual(element("questionnaire.question", in: app).label, "Where should the relay's tests run?")
    XCTAssertEqual(choices(in: app).count, 3)
    let staging = choice("Staging", in: app)
    XCTAssertTrue(staging.label.contains("Recommended"), staging.label)
    XCTAssertTrue(staging.label.contains("The shared box"), staging.label)
    XCTAssertEqual(element("questionnaire.badge", in: app).label, "Recommended")
    let field = element("questionnaire.field", in: app)
    XCTAssertEqual(field.placeholderValue, "Additional text")
    XCTAssertFalse(element("questionnaire.skip", in: app).exists, "a required Question offers no Skip")
    XCTAssertEqual(next.label, "Next")
    XCTAssertFalse(element("questionnaire.back", in: app).exists)
    attach("A questionnaire presenting itself")

    wait(for: "isEnabled == true", element("questionnaire.decline", in: app), timeout: 30)
    XCTAssertFalse(next.isEnabled, "a required Question holds Next until it is answered")
    staging.tap()
    XCTAssertTrue(staging.isSelected)
    XCTAssertTrue(next.isEnabled)
    field.tap()
    field.typeText("after lunch")
    XCTAssertTrue(staging.isSelected, "written beside the choice, both stand")
    attach("A choice with additional text beside it")
    next.tap()

    // Several may be chosen; writing instead clears them, and choosing clears what was written.
    wait(for: "label == 'Question 2 of 3'", position(in: app))
    XCTAssertEqual(element("questionnaire.hint", in: app).label, "Choose any")
    let unit = choice("Unit", in: app)
    let soak = choice("Soak", in: app)
    unit.tap()
    soak.tap()
    XCTAssertTrue(unit.isSelected && soak.isSelected, "several are chosen")
    let instead = element("questionnaire.field", in: app)
    XCTAssertEqual(instead.placeholderValue, "Or write your own")
    instead.tap()
    instead.typeText("the relay suites alone")
    XCTAssertFalse(unit.isSelected || soak.isSelected, "writing clears the choices")
    attach("Several choices cleared by writing instead")
    // Over the keyboard the sheet may be too short for every choice.
    app.scrolled(to: unit, over: element("questionnaire.sheet", in: app)).tap()
    XCTAssertEqual(instead.value as? String, "Or write your own", "choosing clears the text")
    XCTAssertTrue(unit.isSelected)
    XCTAssertTrue(element("questionnaire.skip", in: app).exists, "a Question not required offers Skip")
    XCTAssertTrue(element("questionnaire.back", in: app).exists)
    next.tap()

    // A secret's field is a secure one, and once left it reads as answered and shows nothing of it.
    wait(for: "label == 'Question 3 of 3'", position(in: app))
    let secure = app.secureTextFields["questionnaire.field"]
    XCTAssertTrue(secure.exists, "a secret's field is a secure one")
    XCTAssertEqual(next.label, "Send answers", "the last page sends")
    secure.tap()
    secure.typeText("hunter2")
    element("questionnaire.back", in: app).tap()
    wait(for: "label == 'Question 2 of 3'", position(in: app))
    XCTAssertTrue(choice("Unit", in: app).isSelected, "what was given is kept")
    next.tap()
    wait(for: "label == 'Question 3 of 3'", position(in: app))
    XCTAssertEqual(element("questionnaire.secret", in: app).label, "Answered (secret)")
    attach("A secret answered, shown again")

    next.tap()
    XCTAssertTrue(element("questionnaire.sending", in: app).waitForExistence(timeout: 2), "dimmed until answered")
    let sheet = element("questionnaire.sheet", in: app)
    wait(for: "exists == false", sheet)
    let card = app.descendants(matching: .any).matching(identifier: "session.turn").firstMatch
    wait(for: "label BEGINSWITH 'Working'", card)
    XCTAssertEqual(element("session.barWords", in: app).label, "Working")
    attach("The questionnaire answered, its turn working again")
  }

  @MainActor
  func testSwipedAwayItWaitsBehindReviewWithWhatWasHalfGivenKept() {
    let app = launched()
    let local = choice("Local", in: app)
    XCTAssertTrue(local.waitForExistence(timeout: 10))
    let next = element("questionnaire.next", in: app)
    wait(for: "isEnabled == true", element("questionnaire.decline", in: app))
    local.tap()
    next.tap()
    wait(for: "label == 'Question 2 of 3'", position(in: app))
    choice("Integration", in: app).tap()

    app.swipeSheetAway()
    let sheet = element("questionnaire.sheet", in: app)
    wait(for: "exists == false", sheet)
    XCTAssertEqual(element("session.barWords", in: app).label, "Questionnaire waiting")
    let review = element("session.review", in: app)
    XCTAssertEqual(review.label, "Review")
    attach("A questionnaire dismissed, waiting behind the orange bar")

    review.tap()
    XCTAssertTrue(sheet.waitForExistence(timeout: 5))
    wait(for: "label == 'Question 2 of 3'", position(in: app))
    XCTAssertTrue(choice("Integration", in: app).isSelected, "half-given answers are kept")
    element("questionnaire.back", in: app).tap()
    XCTAssertTrue(choice("Local", in: app).isSelected)
    attach("Reviewed, as it was left")

    // A secret written and swiped away reads as answered once reviewed, showing nothing of itself.
    next.tap()
    next.tap()
    wait(for: "label == 'Question 3 of 3'", position(in: app))
    let secure = app.secureTextFields["questionnaire.field"]
    secure.tap()
    secure.typeText("hunter2")
    app.swipeSheetAway()
    wait(for: "exists == false", sheet)
    element("session.review", in: app).tap()
    XCTAssertTrue(sheet.waitForExistence(timeout: 5))
    wait(for: "label == 'Question 3 of 3'", position(in: app))
    XCTAssertEqual(element("questionnaire.secret", in: app).label, "Answered (secret)")
    attach("A secret swiped away, reviewed")
  }

  @MainActor
  func testSkipLeavesAQuestionUnansweredAndDeclineInTheCornerDeclinesItWhole() {
    let app = launched()
    let staging = choice("Staging", in: app)
    XCTAssertTrue(staging.waitForExistence(timeout: 10))
    let decline = element("questionnaire.decline", in: app)
    wait(for: "isEnabled == true", decline)
    staging.tap()
    element("questionnaire.next", in: app).tap()
    wait(for: "label == 'Question 2 of 3'", position(in: app))
    choice("Soak", in: app).tap()
    let skip = element("questionnaire.skip", in: app)
    skip.tap()
    wait(for: "label == 'Question 3 of 3'", position(in: app))
    let secure = app.secureTextFields["questionnaire.field"]
    secure.tap()
    secure.typeText("hunter2")
    skip.tap()
    XCTAssertEqual(position(in: app).label, "Question 3 of 3", "Skip on the last page stays; Send answers alone sends")
    XCTAssertTrue(element("questionnaire.sheet", in: app).exists)
    XCTAssertFalse(element("questionnaire.sending", in: app).exists)
    XCTAssertFalse(element("questionnaire.secret", in: app).exists, "skipped, it holds nothing")
    attach("Skip on the last page, staying")
    element("questionnaire.back", in: app).tap()
    wait(for: "label == 'Question 2 of 3'", position(in: app))
    XCTAssertFalse(choice("Soak", in: app).isSelected, "skipped, it holds nothing")

    decline.tap()
    wait(for: "exists == false", element("questionnaire.sheet", in: app))
    XCTAssertEqual(element("session.barWords", in: app).label, "Working")
  }

  /// Has iOS keep the software keyboard up for this launch alone, whatever the simulator's own setting, so that what
  /// is typed from a test is typed with it up.
  private let keyboardStays = ["-AutomaticMinimizationEnabled", "NO"]

  /// Waits for the software keyboard to stand fully up, where it comes up at all: a simulator holding a hardware
  /// keyboard that minimises it keeps it below the screen, and nothing then covers the foot, but in a launch in which
  /// it stays. Answers whether it is up.
  @MainActor
  private func keyboardUp(in app: XCUIApplication) -> Bool {
    let keyboard = app.keyboards.firstMatch
    let screen = app.window.frame
    let deadline = Date().addingTimeInterval(3)
    while Date() < deadline {
      if keyboard.exists, keyboard.frame.minY < screen.maxY - keyboard.frame.height / 2 {
        // Let it settle where it stands.
        RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        return true
      }
      RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    }
    return false
  }

  /// Checks that the software keyboard is up, and that Back, Skip and Next stand above it.
  @MainActor
  private func footClearOfTheKeyboard(in app: XCUIApplication) {
    XCTAssertTrue(keyboardUp(in: app), "the software keyboard is up")
    let keyboard = app.keyboards.firstMatch.frame
    for tap in ["questionnaire.back", "questionnaire.skip", "questionnaire.next"] {
      let frame = element(tap, in: app).frame
      XCTAssertLessThanOrEqual(frame.maxY, keyboard.minY, "\(tap) at \(frame) is clear of the keyboard at \(keyboard)")
    }
  }

  @MainActor
  func testTheFootStaysClearOfTheKeyboardOnTheWrittenAndSecretPages() {
    let app = launched(keyboardStays)
    let staging = choice("Staging", in: app)
    XCTAssertTrue(staging.waitForExistence(timeout: 10))
    wait(for: "isEnabled == true", element("questionnaire.decline", in: app))
    staging.tap()
    let next = element("questionnaire.next", in: app)
    next.tap()
    wait(for: "label == 'Question 2 of 3'", position(in: app))
    element("questionnaire.field", in: app).tap()
    footClearOfTheKeyboard(in: app)
    attach("A written answer, the keyboard up")
    next.tap()

    wait(for: "label == 'Question 3 of 3'", position(in: app))
    app.secureTextFields["questionnaire.field"].tap()
    footClearOfTheKeyboard(in: app)
    attach("A secret's field, the keyboard up")
  }

  @MainActor
  func testAWrittenAnswerStaysAboveTheFootAsItGrows() {
    let app = launched(keyboardStays)
    let field = element("questionnaire.field", in: app)
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    wait(for: "isEnabled == true", element("questionnaire.decline", in: app))
    app.scrolled(to: field, over: element("questionnaire.sheet", in: app)).tap()
    XCTAssertTrue(keyboardUp(in: app), "the software keyboard is up")
    var height = fieldInView(in: app, "focused")
    for lines in 2...4 {
      // A line at a time, each seen as it is begun, as a later key would bring into view what this one left out of it.
      field.typeText("\n")
      let grown = fieldInView(in: app, "at \(lines) lines")
      XCTAssertGreaterThan(grown, height, "at \(lines) lines, a line taller")
      height = grown
    }
    XCTAssertTrue(keyboardUp(in: app), "the software keyboard is still up")
    attach("An answer of four lines, whole above the foot")
  }

  /// In type so large that the page is longer than the room over the keyboard and without it alike.
  @MainActor
  func testAPageLongerThanItsRoomIsLeftWhereItWasScrolledAsTheKeyboardIsPutDown() {
    let app = launched(
      keyboardStays + ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL"])
    let field = element("questionnaire.field", in: app)
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    wait(for: "isEnabled == true", element("questionnaire.decline", in: app))
    app.scrolled(to: field, over: element("questionnaire.sheet", in: app)).tap()
    XCTAssertTrue(keyboardUp(in: app), "the software keyboard is up")
    // The page dragged down into the keyboard, which goes down with it, and held still before it is let go, so that
    // nothing of the drag is left to hold the page where it is.
    let bar = app.navigationBars.containing(.button, identifier: "questionnaire.decline").firstMatch.frame
    let screen = app.window.frame
    app.point(across: 0.5, down: (bar.maxY + 20) / screen.height)
      .press(
        forDuration: 0.05, thenDragTo: app.point(across: 0.5, down: 0.95), withVelocity: .slow,
        thenHoldForDuration: 0.5)
    let keyboard = app.keyboards.firstMatch
    let down = NSPredicate { _, _ in !keyboard.exists || keyboard.frame.minY >= screen.maxY }
    expectation(for: down, evaluatedWith: nil)
    waitForExpectations(timeout: 10)
    attach("The keyboard put down, the page left where it was scrolled")
    XCTAssertGreaterThan(
      field.frame.maxY, element("questionnaire.next", in: app).frame.minY, "the page has not gone to its foot")
  }
}
