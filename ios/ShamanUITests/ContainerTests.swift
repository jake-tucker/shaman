import XCTest

/// The container every iPhone's screens stand in, where it has room for one column alone: what is open is pushed over
/// the listing, Back steps one screen at a time, by its button or by a swipe from the edge, and the bars carry what each
/// screen puts in them.
final class ContainerTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String = "paired", _ arguments: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--forget-scene"] + arguments
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
  private func titled(_ expected: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    let title = app.staticTexts.matching(identifier: "session.title")
      .matching(NSPredicate(format: "label == %@", expected)).firstMatch
    XCTAssertTrue(title.waitForExistence(timeout: 10), "the Session titled \(expected) shows", file: file, line: line)
  }

  /// Waits until a tap reaches `element`, as it does once what was pushed over it has gone.
  @MainActor
  private func reached(
    _ element: XCUIElement, _ what: String, file: StaticString = #filePath, line: UInt = #line
  ) {
    for _ in 0..<40 {
      if element.exists, element.isHittable { return }
      Thread.sleep(forTimeInterval: 0.5)
    }
    XCTFail("\(what) is reached", file: file, line: line)
  }

  /// Waits until the listing stands, its foot with it, and nothing over it.
  @MainActor
  private func listed(in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    XCTAssertTrue(app.buttons["listing.scope"].waitForExistence(timeout: 10), "the listing", file: file, line: line)
    XCTAssertTrue(
      app.staticTexts["session.title"].waitForNonExistence(timeout: 10), "nothing open", file: file, line: line)
    reached(app.buttons["home.compose"], "the listing's foot", file: file, line: line)
  }

  /// Opens the Session the listing's row `title` names.
  @MainActor
  private func open(_ title: String, in app: XCUIApplication) {
    XCTAssertTrue(elements("listing.active", in: app).firstMatch.waitForExistence(timeout: 10))
    app.scrolled(to: element("listing.active", holding: title, in: app)).tap()
    titled(title, in: app)
    XCTAssertTrue(elements("session.turn", in: app).firstMatch.waitForExistence(timeout: 10), "\(title) is read")
  }

  /// Swipes from the window's leading edge a little way, and gives it up there: so slowly that on a loaded Mac, which
  /// hands the app a drag in few pieces, what the system makes of its speed still carries it nowhere.
  @MainActor
  private func swipeFromTheEdgeGivenUp(in app: XCUIApplication) {
    app.point(across: 0, down: 0.5).press(
      forDuration: 0.1, thenDragTo: app.point(across: 0.22, down: 0.5), withVelocity: XCUIGestureVelocity(40),
      thenHoldForDuration: 1.5)
  }

  /// Swipes from the window's leading edge across it, as a reader going back does.
  @MainActor
  private func swipeFromTheEdge(in app: XCUIApplication) {
    app.point(across: 0, down: 0.5).press(
      forDuration: 0.1, thenDragTo: app.point(across: 0.9, down: 0.5), withVelocity: .slow, thenHoldForDuration: 0.1)
  }

  @MainActor
  func testBackStepsOneScreenAtATimeAndOnlyTheLastStepReturnsToTheListing() {
    let app = launched()
    open("Tidy zsh startup time", in: app)
    let cards = elements("session.turn", in: app)
    cards.element(boundBy: cards.count - 1).tap()
    let work = app.navigationBars["Worked for 3m 12s"]
    XCTAssertTrue(work.waitForExistence(timeout: 10), "a Turn's work pushed")
    app.scrolled(to: element("work.row", holding: "Edited src/tui/serve_panel.rs", in: app)).tap()
    XCTAssertTrue(app.navigationBars["File change"].waitForExistence(timeout: 10), "an Activity's whole pushed")
    attach("An activity's whole, two deep")
    app.back.tap()
    XCTAssertTrue(work.waitForExistence(timeout: 10), "back at the work")
    XCTAssertTrue(app.navigationBars["File change"].waitForNonExistence(timeout: 10))
    app.back.tap()
    titled("Tidy zsh startup time", in: app)
    XCTAssertTrue(work.waitForNonExistence(timeout: 10), "back at the session")
    XCTAssertTrue(cards.firstMatch.isHittable)
    app.back.tap()
    listed(in: app)

    // And a Subagent's Session, pushed from a row of its parent's work.
    open("Find what holds the session lock", in: app)
    cards.firstMatch.tap()
    let explorer = element("work.row", holding: "Explorer", in: app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 10))
    explorer.tap()
    titled("Find what held the lock", in: app)
    app.back.tap()
    XCTAssertTrue(explorer.waitForExistence(timeout: 10), "back at the work")
    app.back.tap()
    titled("Find what holds the session lock", in: app)
    app.back.tap()
    listed(in: app)
  }

  @MainActor
  func testASwipeFromTheEdgeGivenUpClosesNothingAndOneCarriedThroughClosesOnce() {
    let app = launched()
    open("Tidy zsh startup time", in: app)
    swipeFromTheEdgeGivenUp(in: app)
    XCTAssertFalse(app.buttons["listing.scope"].waitForExistence(timeout: 2), "the listing is not returned to")
    titled("Tidy zsh startup time", in: app)
    XCTAssertTrue(elements("session.turn", in: app).firstMatch.isHittable, "the conversation stands")
    XCTAssertTrue(elements("session.composer", in: app).firstMatch.isHittable)
    XCTAssertFalse(app.buttons["home.compose"].exists, "and the listing's foot does not")
    attach("A swipe from the edge given up")

    // Nor within what is open: a Turn's work stays pushed, and Back from it is one step.
    elements("session.turn", in: app).firstMatch.tap()
    XCTAssertTrue(elements("work.list", in: app).firstMatch.waitForExistence(timeout: 10))
    swipeFromTheEdgeGivenUp(in: app)
    reached(elements("work.row", in: app).firstMatch, "the work")
    swipeFromTheEdge(in: app)
    titled("Tidy zsh startup time", in: app)
    XCTAssertTrue(elements("work.list", in: app).firstMatch.waitForNonExistence(timeout: 10))

    swipeFromTheEdge(in: app)
    listed(in: app)
    attach("A swipe from the edge carried through")
    // Opened again it is read afresh and stays, as nothing else is closed.
    open("Tidy zsh startup time", in: app)
    XCTAssertFalse(app.buttons["listing.scope"].waitForExistence(timeout: 2))
    XCTAssertTrue(elements("session.turn", in: app).firstMatch.isHittable)
  }

  @MainActor
  func testTheWorkspacePickerIsPushedAndAChoicePopsIt() {
    let app = launched("composing")
    XCTAssertTrue(app.buttons["home.compose"].waitForExistence(timeout: 10))
    app.buttons["home.compose"].tap()
    let elsewhere = app.buttons["landing.elsewhere"]
    XCTAssertTrue(elsewhere.waitForExistence(timeout: 10))
    app.scrolled(to: elsewhere).tap()
    let picker = elements("picker.screen", in: app).firstMatch
    XCTAssertTrue(picker.waitForExistence(timeout: 10), "the picker pushed")
    app.back.tap()
    XCTAssertTrue(picker.waitForNonExistence(timeout: 10), "Back pops it")
    XCTAssertTrue(app.staticTexts["landing.title"].waitForExistence(timeout: 10))
    app.scrolled(to: elsewhere).tap()
    XCTAssertTrue(picker.waitForExistence(timeout: 10))
    app.scrolled(to: app.buttons["workspace/workstation/dotfiles"]).tap()
    XCTAssertTrue(picker.waitForNonExistence(timeout: 10), "a choice pops it")
    XCTAssertTrue(
      app.staticTexts.matching(identifier: "landing.workspace")
        .waitForOne(NSPredicate(format: "label == %@", "dotfiles")), "and the screen begins there")
    attach("The new-session screen after a choice")
    // Where new Sessions begin, said above the container, reaches the listing beneath.
    app.back.tap()
    listed(in: app)
    XCTAssertTrue(
      elements("home.beginsIn", in: app).waitForOne(NSPredicate(format: "label CONTAINS %@", "dotfiles")))
  }

  @MainActor
  func testAConversationsPlaceThroughBackIsExact() throws {
    let app = launched("longSession", ["--open", "workstation-key/30"])
    let conversation = elements("session.conversation", in: app).firstMatch
    XCTAssertTrue(conversation.waitForExistence(timeout: 10))
    XCTAssertTrue(element("session.agentMessage", holding: "That's everything", in: app).waitForExistence(timeout: 10))
    for _ in 0..<3 {
      conversation.swipeDown(velocity: .fast)
    }
    let cards = elements("session.turn", in: app)
    XCTAssertTrue(cards.firstMatch.waitForExistence(timeout: 10), "a card away from the foot")
    // Still, once its fling has ended.
    var stood = CGRect.null
    for _ in 0..<20 {
      let stands = cards.firstMatch.frame
      if stands == stood { break }
      stood = stands
      Thread.sleep(forTimeInterval: 0.5)
    }
    let on = (0..<cards.count).map { cards.element(boundBy: $0) }.first { $0.isHittable }
    let card = try XCTUnwrap(on, "a card on screen away from the foot")
    let words = card.label
    let before = card.frame
    attach("Away from the foot")
    card.tap()
    XCTAssertTrue(elements("work.list", in: app).firstMatch.waitForExistence(timeout: 10))
    app.back.tap()
    XCTAssertTrue(elements("work.list", in: app).firstMatch.waitForNonExistence(timeout: 10))
    let again = cards.matching(NSPredicate(format: "label == %@", words)).firstMatch
    XCTAssertTrue(again.waitForExistence(timeout: 10))
    XCTAssertEqual(again.frame.minY, before.minY, accuracy: 0.5, "the same card stands where it stood")
    attach("Back from its work")
  }

  @MainActor
  func testTheBarsCarryWhatEachScreenPutsInThem() {
    let app = launched()
    listed(in: app)
    XCTAssertTrue(app.buttons["listing.scope"].isHittable, "the scope menu, as the listing's title")
    XCTAssertEqual(app.searchFields.count, 1, "the listing's search field")
    XCTAssertTrue(app.searchFields.firstMatch.isHittable)
    XCTAssertTrue(app.buttons["home.servers"].isHittable)
    XCTAssertTrue(elements("home.beginsIn", in: app).firstMatch.exists, "where new Sessions begin")
    XCTAssertFalse(app.back.exists, "the listing has no Back")
    attach("The listing's bars")

    // The scope menu opens, and narrows.
    app.buttons["listing.scope"].tap()
    let dotfiles = app.buttons["dotfiles"].firstMatch
    XCTAssertTrue(dotfiles.waitForExistence(timeout: 10))
    dotfiles.tap()
    XCTAssertTrue(
      app.buttons.matching(identifier: "listing.scope").waitForOne(NSPredicate(format: "label == %@", "dotfiles")))

    // A Session's header, and nothing of the listing's.
    app.scrolled(to: element("listing.active", holding: "Tidy zsh startup time", in: app)).tap()
    titled("Tidy zsh startup time", in: app)
    XCTAssertTrue(app.staticTexts["session.place"].exists, "its place under its title")
    XCTAssertTrue(app.back.isHittable)
    XCTAssertFalse(app.buttons["home.compose"].exists)
    XCTAssertFalse(app.buttons["home.servers"].exists)
    attach("A session's bars")
    app.back.tap()
    listed(in: app)

    // The new-session screen's, and the Workspace Picker's.
    app.buttons["home.compose"].tap()
    XCTAssertTrue(app.staticTexts["landing.title"].waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["landing.place"].exists)
    XCTAssertTrue(app.back.isHittable)
    XCTAssertFalse(app.buttons["home.compose"].exists)
    app.scrolled(to: app.buttons["landing.elsewhere"]).tap()
    XCTAssertTrue(elements("picker.screen", in: app).firstMatch.waitForExistence(timeout: 10))
    XCTAssertTrue(app.searchFields.firstMatch.waitForExistence(timeout: 10), "the picker's search field")
    XCTAssertTrue(app.searchFields.firstMatch.isHittable)
    XCTAssertTrue(app.back.isHittable)
    attach("The workspace picker's bars")
    app.back.tap()
    XCTAssertTrue(app.staticTexts["landing.title"].waitForExistence(timeout: 10))
    app.back.tap()
    listed(in: app)

    // A Session with a tree: its button.
    app.terminate()
    app.launchArguments = ["--fake", "paired", "--forget-scene", "--open", "workstation-key/12"]
    app.launch()
    titled("Find what holds the session lock", in: app)
    XCTAssertTrue(app.buttons["session.tree"].waitForExistence(timeout: 10))
    attach("The bars of a session with a tree")
  }
}
