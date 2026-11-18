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

    // Another Session there, in another Workspace, turns only the Remote: the Workspace remembered for it stays.
    let other = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", "Tidy zsh startup time")).firstMatch
    XCTAssertTrue(other.waitForExistence(timeout: 5))
    other.tap()
    XCTAssertEqual(element("session.place", in: app).label, "dotfiles · main")
    app.navigationBars.buttons.firstMatch.tap()
    XCTAssertTrue(app.navigationBars["All workspaces"].waitForExistence(timeout: 5))
    XCTAssertEqual(element("home.beginsIn", in: app).label, "New sessions begin in workstation · shaman")
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
  func testASessionThatIsGoneSaysSo() {
    let app = launched("paired", opening: "workstation-key/999")
    let gone = element("session.gone", in: app)
    XCTAssertTrue(gone.waitForExistence(timeout: 10))
    XCTAssertEqual(gone.label, "This session no longer exists")
    attach(app, "A session that is gone")
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

    // The code block is wider than the screen, so it scrolls sideways rather than wraps.
    let wide = element("session.conversation", in: app).scrollViews
      .containing(NSPredicate(format: "label BEGINSWITH 'Horizontal scroll bar' AND NOT (label ENDSWITH ', 1 page')"))
      .firstMatch
    XCTAssertTrue(wide.waitForExistence(timeout: 5), "a code block wider than the screen scrolls")
    let content = wide.children(matching: .other).element(boundBy: 0)
    let before = content.frame.minX
    wide.swipeLeft()
    XCTAssertLessThan(content.frame.minX, before, "swiped sideways, it moved")
    attach(app, "A settled turn's code block scrolled sideways")
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

  /// Opens the working Session from the listing, as a reader taps its row.
  @MainActor
  private func openedFromTheListing(_ title: String, in scenario: String = "paired") -> XCUIApplication {
    let app = launched(scenario)
    let row = app.descendants(matching: .any).matching(identifier: "listing.active")
      .matching(NSPredicate(format: "label CONTAINS %@", title)).firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    row.tap()
    XCTAssertTrue(element("session.composer", in: app).waitForExistence(timeout: 5))
    return app
  }

  @MainActor
  private func write(_ text: String, in app: XCUIApplication) {
    let composer = element("session.composer", in: app)
    composer.tap()
    composer.typeText(text)
  }

  /// The bubble the user said `text` in, the last of them.
  @MainActor
  private func bubble(_ text: String, in app: XCUIApplication) -> XCUIElement {
    elements("session.userMessage", in: app).matching(NSPredicate(format: "label == %@", text)).firstMatch
  }

  @MainActor
  private func waitFor(_ format: String, _ argument: Any, on target: XCUIElement) {
    expectation(for: NSPredicate(format: format, argumentArray: [argument]), evaluatedWith: target)
    waitForExpectations(timeout: 10)
  }

  @MainActor
  func testAPromptSentFromTheComposerStandsAtOnceAsSendingThenAsSent() {
    let app = openedFromTheListing("Draw the invite as a QR code")
    let bar = element("session.bar", in: app)
    XCTAssertTrue(bar.waitForExistence(timeout: 5))
    XCTAssertEqual(element("session.barWords", in: app).label, "Working")
    XCTAssertEqual(element("session.composer", in: app).placeholderValue, "Steer, or hold send to queue")

    write("Give it a quiet zone too", in: app)
    element("session.send", in: app).tap()
    let sending = elements("session.sendingMessage", in: app)
      .matching(NSPredicate(format: "label == %@", "Give it a quiet zone too")).firstMatch
    XCTAssertTrue(sending.waitForExistence(timeout: 5), "it stands at once, marked as sending")
    XCTAssertEqual(
      (element("session.composer", in: app).value as? String) ?? "", "Steer, or hold send to queue",
      "the field empties as it is sent")
    attach(app, "A prompt being sent")
    XCTAssertTrue(bubble("Give it a quiet zone too", in: app).waitForExistence(timeout: 10), "and then as sent")
    XCTAssertFalse(sending.exists)
    attach(app, "A prompt sent")
  }

  @MainActor
  func testHoldingSendQueuesAPromptWhichWaitsAtTheFootToBeSteeredNowOrCancelled() {
    let app = openedFromTheListing("Draw the invite as a QR code")
    write("Then the changelog", in: app)
    element("session.send", in: app).press(forDuration: 1.2)
    let queue = app.buttons["Queue for after this turn"]
    XCTAssertTrue(queue.waitForExistence(timeout: 5), "a held send offers to queue")
    queue.tap()

    let steerNow = element("session.steerNow", in: app)
    XCTAssertTrue(steerNow.waitForExistence(timeout: 5))
    XCTAssertEqual(element("session.queuedText", in: app).label, "Then the changelog")
    XCTAssertTrue(element("session.cancelQueued", in: app).exists)
    attach(app, "A queued prompt at the foot")
    steerNow.tap()
    XCTAssertTrue(bubble("Then the changelog", in: app).waitForExistence(timeout: 5), "steered, it was said")
    XCTAssertFalse(element("session.queued", in: app).exists)

    write("And the docs", in: app)
    element("session.send", in: app).press(forDuration: 1.2)
    app.buttons["Queue for after this turn"].tap()
    let cancel = element("session.cancelQueued", in: app)
    XCTAssertTrue(cancel.waitForExistence(timeout: 5))
    cancel.tap()
    let queued = element("session.queued", in: app)
    waitFor("exists == %@", false, on: queued)
    XCTAssertFalse(bubble("And the docs", in: app).exists, "cancelled, it was never said")
  }

  @MainActor
  func testStopTakesOneTapAndTheBarGoes() {
    let app = launched("paired", opening: "workstation-key/1")
    let stop = element("session.stop", in: app)
    XCTAssertTrue(stop.waitForExistence(timeout: 10))
    XCTAssertTrue(lastCard(in: app).label.contains("cargo nextest run qr"))
    stop.tap()
    waitFor("exists == %@", false, on: element("session.bar", in: app))
    XCTAssertTrue(lastCard(in: app).label.contains("Stopped after"), lastCard(in: app).label)
    XCTAssertEqual(element("session.composer", in: app).placeholderValue, "Message")
    attach(app, "A turn stopped with one tap")
  }

  @MainActor
  func testStopOnASessionWorkingOnlyForAnUndeliveredPromptGivesItsTextBackAheadOfWhatIsWritten() {
    let app = launched("paired", opening: "workstation-key/11")
    XCTAssertTrue(bubble("Note the floor in the pairing docs", in: app).waitForExistence(timeout: 10))
    write("and the changelog", in: app)
    element("session.stop", in: app).tap()
    let composer = element("session.composer", in: app)
    waitFor("value == %@", "Note the floor in the pairing docs\nand the changelog", on: composer)
    XCTAssertFalse(bubble("Note the floor in the pairing docs", in: app).exists, "withdrawn")
    XCTAssertFalse(element("session.bar", in: app).exists)
    attach(app, "A withdrawn prompt back in the composer")
  }

  @MainActor
  func testASendThatFailsSaysSoInABannerAndItsTextIsBackInTheComposer() {
    let app = openedFromTheListing("Draw the invite as a QR code")
    write("Please fail", in: app)
    element("session.send", in: app).tap()
    let banner = element("banner", in: app)
    XCTAssertTrue(banner.waitForExistence(timeout: 5))
    XCTAssertEqual(banner.label, "The server refused the prompt: its agent is not signed in")
    XCTAssertFalse(app.alerts.firstMatch.exists, "never an alert")
    waitFor("value == %@", "Please fail", on: element("session.composer", in: app))
    XCTAssertFalse(bubble("Please fail", in: app).exists, "its bubble went")
    attach(app, "A failed send")
  }

  @MainActor
  func testAnActFindingTheSessionGonePopsItsScreen() {
    let app = openedFromTheListing("Draw the invite as a QR code")
    write("Is it gone?", in: app)
    element("session.send", in: app).tap()
    XCTAssertTrue(app.navigationBars["All workspaces"].waitForExistence(timeout: 5), "popped back to the listing")
  }

  @MainActor
  func testOnAnUnreachableOriginTheComposerTakesNothingAndSaysWhy() {
    let app = launched("unreachable", opening: "homelab-key/2")
    let line = element("session.unreachable", in: app)
    XCTAssertTrue(line.waitForExistence(timeout: 10))
    XCTAssertEqual(line.label, "homelab is unreachable. Nothing sent goes anywhere until it answers.")
    XCTAssertFalse(element("session.composer", in: app).isEnabled)
    attach(app, "A session on an unreachable origin")
  }

  @MainActor
  func testAMonitoringSessionsBarNamesItsWatch() {
    let app = launched("several", opening: "homelab-key/6")
    let bar = element("session.bar", in: app)
    XCTAssertTrue(bar.waitForExistence(timeout: 10))
    XCTAssertEqual(element("session.barWords", in: app).label, "Monitoring")
    XCTAssertTrue(bar.staticTexts["the nightly backup"].exists, "it names the Watch it waits on")
    XCTAssertTrue(element("session.stop", in: app).exists)
    attach(app, "A monitoring session")
  }
}
