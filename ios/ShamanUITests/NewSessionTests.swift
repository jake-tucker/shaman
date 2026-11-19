import XCTest

/// Beginning a Session over the fake: the compose button pushes the new-session screen where the Outlook stands, and
/// a first Prompt turns it into the Provisional Session and then the Session the fake's Server made, which back leaves
/// listed; a refusal stands where the work would be, asked again or replaced, and left keeps its text as the draft.
final class NewSessionTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String, _ arguments: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario] + arguments
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

  /// Pushes the new-session screen from the listing's compose button.
  @MainActor
  private func compose(_ app: XCUIApplication) {
    let compose = app.buttons["home.compose"]
    XCTAssertTrue(compose.waitForExistence(timeout: 10))
    compose.tap()
    XCTAssertTrue(element("landing.title", in: app).waitForExistence(timeout: 5))
  }

  /// Writes `text` in the new-session screen's composer, whose keyboard is up on arrival.
  @MainActor
  private func write(_ text: String, in app: XCUIApplication) {
    let composer = element("landing.composer", in: app)
    XCTAssertTrue(composer.waitForExistence(timeout: 5))
    XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "the keyboard is up on arrival")
    composer.typeText(text)
  }

  @MainActor
  private func back(_ app: XCUIApplication) {
    app.navigationBars.buttons.firstMatch.tap()
    XCTAssertTrue(app.buttons["home.compose"].waitForExistence(timeout: 5), "back at the listing")
  }

  @MainActor
  func testAFirstPromptBeginsASessionWhereTheOutlookStandsAndBackLeavesItListed() {
    let app = launched("composing")
    compose(app)
    XCTAssertEqual(element("landing.title", in: app).label, "New session")
    XCTAssertEqual(
      element("landing.place", in: app).label, "shaman", "over where it will begin, with one Remote naming no Origin")
    XCTAssertEqual(element("landing.workspace", in: app).label, "shaman")
    XCTAssertEqual(element("landing.origin", in: app).label, "workstation")
    XCTAssertTrue(element("landing.description", in: app).label.hasPrefix("A terminal client for coding agents"))
    XCTAssertEqual(element("landing.agent", in: app).label, "Claude · Fable 5.2 · high", "the Agent it will get")
    XCTAssertFalse(app.buttons["landing.send"].isEnabled, "nothing written yet")
    attach(app, "The new-session screen where the Outlook stands")

    write("Tidy the build warnings\nand the lint", in: app)
    XCTAssertTrue(app.buttons["landing.send"].isEnabled)
    app.buttons["landing.send"].tap()

    // The same screen is the Provisional Session at once: the Prompt as a bubble, its first line as the Title, and
    // a bar reading Starting with no Stop.
    let title = element("session.title", in: app)
    XCTAssertTrue(title.waitForExistence(timeout: 5))
    XCTAssertEqual(title.label, "Tidy the build warnings")
    XCTAssertEqual(element("session.barWords", in: app).label, "Starting")
    XCTAssertFalse(element("session.stop", in: app).exists, "no Stop while it starts")
    XCTAssertTrue(element("session.sendingMessage", in: app).exists)
    attach(app, "Starting")

    // The real Session replaces it in place.
    let working = NSPredicate(format: "label == %@", "Working")
    expectation(for: working, evaluatedWith: element("session.barWords", in: app))
    waitForExpectations(timeout: 10)
    XCTAssertTrue(element("session.stop", in: app).exists)
    XCTAssertTrue(element("session.userMessage", in: app).exists)
    attach(app, "The session begun, in its place")

    back(app)
    let row = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "Tidy the build warnings")).firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 5), "the Session begun is listed")
    row.tap()
    XCTAssertTrue(element("session.title", in: app).waitForExistence(timeout: 5))
  }

  @MainActor
  func testLeavingBeforeTheServerAnswersStopsNothing() {
    let app = launched("composing")
    compose(app)
    write("Trim the shell startup", in: app)
    app.buttons["landing.send"].tap()
    XCTAssertTrue(element("session.barWords", in: app).waitForExistence(timeout: 5))
    back(app)
    let row = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "Trim the shell startup")).firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 10), "begun all the same, and listed")
  }

  @MainActor
  func testARefusalReadsWhyAndTryingAgainBeginsIt() {
    let app = launched("composing")
    compose(app)
    write("Refuse this one", in: app)
    app.buttons["landing.send"].tap()
    let refused = element("session.refusedWords", in: app)
    XCTAssertTrue(refused.waitForExistence(timeout: 10))
    XCTAssertTrue(refused.label.hasPrefix("Could not create session: "), refused.label)
    XCTAssertFalse(element("session.bar", in: app).exists, "no bar once refused")
    attach(app, "A refused beginning")

    app.buttons["session.tryAgain"].tap()
    let working = NSPredicate(format: "label == %@", "Working")
    let bar = element("session.barWords", in: app)
    XCTAssertTrue(bar.waitForExistence(timeout: 5))
    expectation(for: working, evaluatedWith: bar)
    waitForExpectations(timeout: 10)
    XCTAssertFalse(refused.exists)
  }

  @MainActor
  func testAPromptWrittenInsteadReplacesTheRefusedOne() {
    let app = launched("composing")
    compose(app)
    write("Refuse this one", in: app)
    app.buttons["landing.send"].tap()
    XCTAssertTrue(element("session.refused", in: app).waitForExistence(timeout: 10))

    let composer = element("session.composer", in: app)
    composer.tap()
    composer.typeText("Draw the invite instead")
    app.buttons["session.send"].tap()
    let title = element("session.title", in: app)
    expectation(for: NSPredicate(format: "label == %@", "Draw the invite instead"), evaluatedWith: title)
    waitForExpectations(timeout: 10)
    let working = NSPredicate(format: "label == %@", "Working")
    expectation(for: working, evaluatedWith: element("session.barWords", in: app))
    waitForExpectations(timeout: 10)
    let bubbles = app.descendants(matching: .any).matching(identifier: "session.userMessage")
    XCTAssertEqual(bubbles.count, 1)
    XCTAssertEqual(bubbles.firstMatch.label, "Draw the invite instead", "in place of the refused one")
  }

  @MainActor
  func testLeavingARefusedBeginningKeepsItsTextAsTheDraftAndTheScreenKeepsADraftOfItsOwn() {
    let app = launched("composing")
    compose(app)
    write("Refuse this one", in: app)
    app.buttons["landing.send"].tap()
    XCTAssertTrue(element("session.refused", in: app).waitForExistence(timeout: 10))
    back(app)
    compose(app)
    XCTAssertEqual(element("landing.composer", in: app).value as? String, "Refuse this one", "kept as the draft")

    // Whatever is written there is the new-session screen's own draft.
    element("landing.composer", in: app).typeText(" again")
    back(app)
    compose(app)
    XCTAssertEqual(element("landing.composer", in: app).value as? String, "Refuse this one again")
  }

  @MainActor
  func testABeginningNeverAnsweredSaysItMayHaveBegunAndLeftTheLandingSaysSo() {
    let app = launched("composing")
    compose(app)
    write("Unanswered, this one", in: app)
    app.buttons["landing.send"].tap()
    let words = element("session.refusedWords", in: app)
    XCTAssertTrue(words.waitForExistence(timeout: 10))
    XCTAssertEqual(words.label, "The server did not answer, so the session may have begun")
    XCTAssertTrue(element("session.mayHaveBegun", in: app).exists, "never that it was refused")
    XCTAssertFalse(element("session.refused", in: app).exists)
    attach(app, "A beginning never answered")

    back(app)
    compose(app)
    XCTAssertEqual(element("landing.composer", in: app).value as? String, "Unanswered, this one", "kept as the draft")
    let note = element("landing.note", in: app)
    XCTAssertTrue(note.waitForExistence(timeout: 5))
    XCTAssertTrue(note.label.contains("may have begun"), note.label)
    attach(app, "The landing after a beginning never answered")
  }

  @MainActor
  func testWhereNoWorkspaceIsRememberedTheScreenAsksForOneAndSendIsDisabled() {
    let app = launched("paired")
    compose(app)
    XCTAssertFalse(element("landing.place", in: app).exists, "with one Remote and no Workspace, nothing to say")
    let asks = element("landing.asks", in: app)
    XCTAssertTrue(asks.exists)
    XCTAssertTrue(asks.label.hasPrefix("Choose a workspace to begin in"), asks.label)
    write("Tidy the build", in: app)
    XCTAssertFalse(app.buttons["landing.send"].isEnabled, "nowhere to begin")
    attach(app, "No workspace remembered")
  }

  @MainActor
  func testOnAnUnreachableRemoteTheComposerTakesADraftAndSendIsDisabled() {
    // Opening the home lab's Session turns the Outlook toward it, which no longer answers.
    let app = launched("unreachable", ["--open", "homelab-key/2"])
    XCTAssertTrue(element("session.title", in: app).waitForExistence(timeout: 10))
    back(app)
    compose(app)
    XCTAssertEqual(element("landing.place", in: app).label, "homelab · home-assistant")
    let line = element("landing.unreachable", in: app)
    XCTAssertTrue(line.exists)
    XCTAssertEqual(line.label, "homelab is unreachable. Nothing begins there until it answers.")
    XCTAssertEqual(
      element("landing.agent", in: app).label, "Agent unavailable", "where its Server says none would serve")
    write("Upgrade the add-ons", in: app)
    XCTAssertEqual(element("landing.composer", in: app).value as? String, "Upgrade the add-ons", "it takes a draft")
    XCTAssertFalse(app.buttons["landing.send"].isEnabled)
    attach(app, "An unreachable remote")
  }
}
