import XCTest

/// The Duo sat like a laptop, partly folded and turned: run by `scripts/ios-test.sh posed laptop`.
///
/// What holds that it folded is where things stand about the fold, as the display open flat in the same portrait has
/// the conversation run on to its foot, and no Aside.
final class PosedLaptopTests: XCTestCase {
  /// The band kept for the fold, across the window.
  private let band: ClosedRange<CGFloat> = 455.5...495.5

  /// The app over the fake's `scenario` with the Session `session` open, sat like a laptop, the software keyboard one
  /// that stays up.
  @MainActor
  private func launched(_ scenario: String, _ session: String, _ arguments: [String] = []) throws -> XCUIApplication {
    try Pose.laptop.launched(
      self,
      ["--fake", scenario, "--open", session, "--forget-scene", "-AutomaticMinimizationEnabled", "NO"] + arguments)
  }

  @MainActor
  private func elements(_ identifier: String, in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: identifier)
  }

  @MainActor
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    elements(identifier, in: app).firstMatch
  }

  @MainActor
  private func element(_ identifier: String, holding text: String, in app: XCUIApplication) -> XCUIElement {
    elements(identifier, in: app).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
  }

  /// Waits until it stands so.
  @MainActor
  private func waits(
    _ said: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line, _ stands: () -> Bool
  ) {
    for _ in 0..<40 {
      if stands() { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    attach("Not as it should stand")
    XCTFail(said, file: file, line: line)
  }

  /// Waits until the Aside stands below the fold, its bar reading `title`, with Back in it where it is `pushed` and
  /// nothing else, above the field, which stands 22 points above the edge.
  @MainActor
  private func asideStands(
    _ app: XCUIApplication, at title: String, pushed: Bool = false, file: StaticString = #filePath, line: UInt = #line
  ) {
    let bar = app.navigationBars[title]
    waits("the Aside stands below the fold at \(title)", in: app, file: file, line: line) {
      bar.exists && bar.frame.minY >= band.upperBound - 1 && bar.buttons.count == (pushed ? 1 : 0)
    }
    if pushed {
      XCTAssertTrue(bar.buttons["BackButton"].isHittable, "its own Back", file: file, line: line)
    }
    fieldStands(22, aboveTheEdgeIn: app, "session.composer", file: file, line: line)
    XCTAssertFalse(app.buttons["session.aside"].exists, "no button puts it away", file: file, line: line)
    XCTAssertFalse(element("tree.sheet", in: app).exists, file: file, line: line)
  }

  /// Waits until the conversation, or the bar at its foot where there is one, ends where the fold begins; and with it
  /// `atItsFoot`, until every row on screen ends there too.
  @MainActor
  private func conversationEndsAtTheFold(
    _ app: XCUIApplication, atItsFoot: Bool = false, file: StaticString = #filePath, line: UInt = #line
  ) {
    let rows = ["session.agentMessage", "session.turn", "session.userMessage", "session.compaction"]
    let conversation = app.collectionViews["session.conversation"]
    waits("the conversation ends at the fold", in: app, file: file, line: line) {
      let bar = element("session.bar", in: app)
      let foot = bar.exists ? bar.frame.maxY : conversation.frame.maxY
      guard foot <= band.lowerBound + 1, foot > band.lowerBound - 24 else { return false }
      let ends = rows.flatMap { elements($0, in: app).allElementsBoundByIndex }.filter(\.isHittable).map(\.frame.maxY)
      return !atItsFoot || (!ends.isEmpty && ends.allSatisfy { $0 <= conversation.frame.maxY + 1 })
    }
  }

  /// Waits until the field stands just above the band kept for the fold, the software keyboard fully up over the half
  /// below, its own view beginning at either edge of that band.
  @MainActor
  private func fieldStandsAboveTheFold(
    in app: XCUIApplication, _ identifier: String = "session.composer", file: StaticString = #filePath,
    line: UInt = #line
  ) {
    let keyboard = app.otherElements["inputView"].firstMatch
    XCTAssertTrue(keyboard.waitForExistence(timeout: 10), "the software keyboard is up", file: file, line: line)
    let composer = element(identifier, in: app)
    waits("the field stands above the fold, over the keyboard", in: app, file: file, line: line) {
      let ends = composer.frame.midY + 20
      return abs(band.lowerBound - 8 - ends) < 1
        && keyboard.frame.minY > band.lowerBound - 1 && keyboard.frame.minY < band.upperBound + 1
    }
  }

  /// Drags the software keyboard down, as a reader does over the conversation.
  @MainActor
  private func dragTheKeyboardDown(
    in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    app.point(across: 0.5, down: 0.25).press(forDuration: 0.05, thenDragTo: app.point(across: 0.5, down: 0.98))
    XCTAssertTrue(
      app.keyboards.firstMatch.waitForNonExistence(timeout: 10), "its keyboard put down", file: file, line: line)
  }

  @MainActor
  private func holdsTheKeyboard(_ element: XCUIElement) -> Bool {
    element.value(forKey: "hasKeyboardFocus") as? Bool ?? false
  }

  @MainActor
  func testTheConversationEndsAtTheFoldWithItsStatusLinesAndTheAsideStandsBelowOverTheField() throws {
    for arguments in [[], ["--aside", "work"]] {
      let app = try launched("paired", "workstation-key/1", arguments)
      asideStands(app, at: "Working")
      conversationEndsAtTheFold(app, atItsFoot: true)
      attach("A working session, sat like a laptop")
      let list = element("work.list", in: app)
      XCTAssertGreaterThanOrEqual(list.frame.minY, band.upperBound - 1)
      XCTAssertLessThanOrEqual(list.frame.maxY, element("session.composer", in: app).frame.midY - 20)
      XCTAssertTrue(
        elements("session.turn", in: app).waitForOne(NSPredicate(format: "selected == true")), "its card is marked")
      let sidebar = app.buttons["Show Sidebar"].firstMatch
      XCTAssertTrue(sidebar.isHittable && sidebar.frame.maxY < 134, "the bar across the head")
      app.terminate()
    }
  }

  @MainActor
  func testAConversationAtItsFootIsThereAndADragMovesIt() throws {
    let app = try launched("longSession", "workstation-key/30")
    let last = element("session.agentMessage", holding: "The last item of this Session", in: app)
    XCTAssertTrue(last.waitForExistence(timeout: 15))
    asideStands(app, at: "Working")
    conversationEndsAtTheFold(app, atItsFoot: true)
    attach("A long conversation at its foot")
    let stood = last.frame.minY
    let start = app.point(across: 0.5, down: 0.2)
    let moved = { !last.exists || !last.isHittable || last.frame.minY > stood + 50 }
    // Again where it came before the list had landed.
    for _ in 0..<3 where !moved() {
      start.press(
        forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 150)), withVelocity: .slow,
        thenHoldForDuration: 0.3)
    }
    XCTAssertTrue(moved(), "the drag moved the conversation")
    // Nothing of it stands in the fold away from its foot either.
    XCTAssertLessThanOrEqual(app.collectionViews["session.conversation"].frame.maxY, band.lowerBound + 1)
  }

  @MainActor
  func testTheFieldGoesAboveTheFoldWithTheKeyboardAndBackKeepingItsFocus() throws {
    let app = try launched("paired", "workstation-key/1")
    let composer = element("session.composer", in: app)
    var typed = ""
    for _ in 0..<2 {
      asideStands(app, at: "Working")
      composer.tap()
      fieldStandsAboveTheFold(in: app)
      XCTAssertTrue(holdsTheKeyboard(composer), "it keeps its focus")
      let bar = element("session.bar", in: app)
      XCTAssertLessThanOrEqual(bar.frame.maxY, composer.frame.midY - 20, "under the status lines")
      XCTAssertTrue(element("work.list", in: app).waitForNonExistence(timeout: 10), "the keyboard has that half")
      composer.typeText("hi")
      typed += "hi"
      XCTAssertEqual(composer.value as? String, typed)
      XCTAssertTrue(holdsTheKeyboard(composer))
      attach("The field above the fold, typed in")
      dragTheKeyboardDown(in: app)
      XCTAssertEqual(composer.value as? String, typed)
    }
    asideStands(app, at: "Working")
    conversationEndsAtTheFold(app)
    attach("The keyboard dragged down")
  }

  @MainActor
  func testACardAndTheTreeButtonTurnTheAsideAndNeitherPushNorPresent() throws {
    let app = try launched("paired", "workstation-key/12")
    asideStands(app, at: "Working")
    for _ in 0..<2 {
      app.buttons["session.tree"].tap()
      asideStands(app, at: "Subagents")
      chooserStands(beneath: "tree.list", in: app)
      attach("The tree below the fold")
      element("session.turn", in: app).tap()
      asideStands(app, at: "Working")
      chooserStands(beneath: "work.list", in: app)
      XCTAssertFalse(app.back.exists, "nothing pushed")
    }
    // The tree's last Session is reached above the chooser.
    app.buttons["aside.tree"].tap()
    asideStands(app, at: "Subagents")
    let list = element("tree.list", in: app)
    let last = element("tree.row", holding: "Review the lock's tests", in: app)
    app.scrolled(to: last, over: list)
    list.swipeUp()
    waits("the last row stands above the chooser", in: app) {
      last.frame.maxY <= app.buttons["aside.work"].frame.minY + 1
    }
    attach("The tree's last row")
    app.buttons["aside.work"].tap()
    asideStands(app, at: "Working")
  }

  /// The chooser stands beneath the Aside's list, `list`, above the field, marking the one `list` is of.
  @MainActor
  private func chooserStands(
    beneath list: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    let chosen = list == "tree.list" ? "aside.tree" : "aside.work"
    XCTAssertTrue(
      elements(chosen, in: app).waitForOne(NSPredicate(format: "selected == true")), chosen, file: file, line: line)
    let above = element(list, in: app)
    XCTAssertTrue(above.waitForExistence(timeout: 10), file: file, line: line)
    for side in [app.buttons["aside.work"], app.buttons["aside.tree"]] {
      XCTAssertGreaterThanOrEqual(side.frame.minY, above.frame.maxY - 1, "beneath it", file: file, line: line)
      XCTAssertLessThanOrEqual(
        side.frame.maxY, element("session.composer", in: app).frame.midY - 20, "over the field", file: file, line: line)
      XCTAssertTrue(side.isHittable, file: file, line: line)
    }
  }

  @MainActor
  func testASessionWithNoTurnSaysSoThere() throws {
    let app = try launched("paired", "workstation-key/11")
    let empty = element("work.empty", in: app)
    waits("it says nothing is done yet", in: app) {
      empty.exists && empty.label == "Nothing done yet" && empty.frame.minY >= band.upperBound
    }
    fieldStands(22, aboveTheEdgeIn: app, "session.composer")
    attach("A session with no Turn")
  }

  @MainActor
  func testAnApprovalAtHalfHeightLandsBelowTheFoldWithTheStatusLinesInSight() throws {
    let app = try launched("paired", "workstation-key/3")
    let title = element("approval.title", in: app)
    XCTAssertTrue(title.waitForExistence(timeout: 10))
    waits("the sheet stands below the fold", in: app) { title.frame.minY >= band.upperBound - 8 }
    attach("An approval, sat like a laptop")
    // The system begins it 8 points inside the band.
    let grabber = app.buttons["Sheet Grabber"]
    XCTAssertTrue(grabber.waitForExistence(timeout: 10))
    XCTAssertGreaterThanOrEqual(grabber.frame.minY, band.lowerBound)
    XCTAssertTrue(element("approval.acceptOnce", in: app).isHittable)
    let words = element("session.barWords", in: app)
    XCTAssertTrue(words.exists, "the status lines in sight above it")
    XCTAssertLessThanOrEqual(words.frame.maxY, band.lowerBound)
    app.swipeSheetAway()
    XCTAssertTrue(element("approval.sheet", in: app).waitForNonExistence(timeout: 20))
    conversationEndsAtTheFold(app)
    element("session.review", in: app).tap()
    waits("and again", in: app) { title.exists && title.frame.minY >= band.upperBound - 8 }
    XCTAssertTrue(words.exists, "the status lines in sight above it")
    XCTAssertLessThanOrEqual(words.frame.maxY, band.lowerBound)
    attach("An approval reviewed, the status lines above it")
  }

  /// The app with `/13` open, its Questionnaire presenting itself, once it takes taps. That it goes down as the phone is
  /// sat down with it up, and up as the phone is laid flat, is held by `LaptopHostedTests.Sheets` about a fold of the
  /// test's own, and on the phone by the spec's check 38, as no test changes a pose.
  @MainActor
  private func asked() throws -> XCUIApplication {
    let app = try launched("paired", "workstation-key/13", ["--arming-moment", "0.6"])
    let decline = element("questionnaire.decline", in: app)
    XCTAssertTrue(decline.waitForExistence(timeout: 10))
    XCTAssertTrue(elements("questionnaire.decline", in: app).waitForOne(NSPredicate(format: "isEnabled == true")))
    return app
  }

  /// Waits until the Questionnaire's sheet stands on the half below the fold, its top 8 points past the band kept for
  /// the fold, Decline clear of it and the status lines in sight above it; or, where not, at full height, across the
  /// fold.
  @MainActor
  private func questionnaireStands(
    onTheHalfBelow below: Bool, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    let sheet = element("questionnaire.sheet", in: app)
    waits(
      below ? "the sheet stands on the half below the fold" : "the sheet stands at full height", in: app,
      file: file, line: line
    ) {
      sheet.exists
        && (below ? abs(sheet.frame.minY - band.upperBound - 8) <= 1 : sheet.frame.minY < band.lowerBound - 300)
    }
    guard below else { return }
    let decline = element("questionnaire.decline", in: app)
    XCTAssertGreaterThanOrEqual(
      decline.frame.minY, band.upperBound, "Decline clear of the fold", file: file, line: line)
    XCTAssertTrue(decline.isHittable, file: file, line: line)
    XCTAssertTrue(element("questionnaire.next", in: app).isHittable, file: file, line: line)
    let words = element("session.barWords", in: app)
    XCTAssertTrue(words.exists, "the status lines in sight above it", file: file, line: line)
    XCTAssertLessThanOrEqual(words.frame.maxY, band.lowerBound, file: file, line: line)
  }

  @MainActor
  func testAQuestionnaireStandsOnTheHalfBelowTheFoldWithDeclineClearOfIt() throws {
    let app = try asked()
    questionnaireStands(onTheHalfBelow: true, in: app)
    attach("A questionnaire, sat like a laptop")
    // Swiped away and reviewed, it is presented in the pose again.
    app.swipeSheetAway()
    XCTAssertTrue(element("questionnaire.sheet", in: app).waitForNonExistence(timeout: 20))
    element("session.review", in: app).tap()
    questionnaireStands(onTheHalfBelow: true, in: app)
  }

  @MainActor
  func testAQuestionnaireGoesToFullHeightAsAFieldTakesTheKeyboardWithTheKeyboardBelow() throws {
    let app = try asked()
    questionnaireStands(onTheHalfBelow: true, in: app)
    let field = element("questionnaire.field", in: app)
    app.scrolled(to: field, over: element("questionnaire.sheet", in: app)).tap()
    XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10), "the software keyboard is up")
    XCTAssertTrue(holdsTheKeyboard(field))
    questionnaireStands(onTheHalfBelow: false, in: app)
    field.typeText("after lunch")
    waits("the field above the keyboard", in: app) {
      app.keyboards.firstMatch.frame.minY >= element("questionnaire.next", in: app).frame.maxY
    }
    _ = fieldInView(in: app, "focused, sat like a laptop")
    attach("A questionnaire's field focused, sat like a laptop")
    // Put down, the keyboard leaves it up, where the reader may write on.
    dragTheKeyboardDown(in: app)
    questionnaireStands(onTheHalfBelow: false, in: app)
  }

  /// `walk-duo.sh`'s walk, but for its ends: laying the phone flat and turning it is a person's, as no test changes a
  /// pose.
  @MainActor
  func testAnActivityOpensWithinTheAsideAndStaysThroughTheKeyboardAndTheSidebar() throws {
    let app = try launched("paired", "workstation-key/7")
    asideStands(app, at: "Worked for 3m 12s")
    element("work.row", holding: "Read src/serving.rs", in: app).tap()
    let detail = element("detail.screen", in: app)
    XCTAssertTrue(detail.waitForExistence(timeout: 10))
    XCTAssertGreaterThanOrEqual(detail.frame.minY, band.upperBound - 1, "below the fold")
    let backs = app.buttons.matching(identifier: "BackButton")
    waits("pushed within the Aside, and nothing in the main view", in: app) {
      backs.count == 1 && backs.firstMatch.frame.minY >= band.upperBound - 1
    }
    XCTAssertTrue(app.staticTexts["session.title"].isHittable)
    attach("An activity within the Aside")

    let composer = element("session.composer", in: app)
    composer.tap()
    fieldStandsAboveTheFold(in: app)
    composer.typeText("hi")
    XCTAssertEqual(composer.value as? String, "hi")
    dragTheKeyboardDown(in: app)
    fieldStands(22, aboveTheEdgeIn: app, "session.composer")
    XCTAssertTrue(detail.waitForExistence(timeout: 10), "still pushed")

    // Called back, the Sidebar covers both halves, and goes as the Session is chosen again.
    app.buttons["Show Sidebar"].firstMatch.tap()
    let scope = app.buttons["listing.scope"]
    XCTAssertTrue(scope.waitForExistence(timeout: 10))
    XCTAssertGreaterThan(app.collectionViews["listing.list"].frame.maxY, band.upperBound, "over both halves")
    attach("The Sidebar over both halves")
    let row = element("listing.active", holding: "Tidy zsh startup time", in: app)
    app.scrolled(to: row, over: app.collectionViews["listing.list"]).tap()
    XCTAssertTrue(detail.waitForExistence(timeout: 10), "as it was, the Session being the one open")
    fieldStands(22, aboveTheEdgeIn: app, "session.composer")
    waits("the Aside's Back alone", in: app) { backs.count == 1 && backs.firstMatch.isHittable }
    backs.firstMatch.tap()
    asideStands(app, at: "Worked for 3m 12s")
    conversationEndsAtTheFold(app)
  }

  @MainActor
  func testASubagentsSessionPushedInThePoseStandsAsAnySessionDoes() throws {
    let app = try launched("paired", "workstation-key/12")
    asideStands(app, at: "Working")
    // Its row below the fold pushes it in the main view, where it stands as its parent did.
    let row = element("work.row", holding: "Explorer", in: app)
    app.scrolled(to: row, over: element("work.list", in: app)).tap()
    let place = app.staticTexts["session.place"]
    waits("the Subagent's Session is pushed in the main view", in: app) {
      place.exists && place.label == "Explorer · haiku" && app.back.exists
    }
    let bar = app.navigationBars["Working"]
    waits("its own Aside below the fold", in: app) {
      bar.exists && bar.frame.minY >= band.upperBound - 1 && bar.buttons.count == 0
    }
    conversationEndsAtTheFold(app)
    XCTAssertFalse(app.buttons["session.aside"].exists, "no button puts it away")
    attach("A Subagent's Session sat like a laptop")
    app.back.tap()
    waits("its parent's Session again", in: app) { !app.back.exists && place.label != "Explorer · haiku" }
    asideStands(app, at: "Working")
    conversationEndsAtTheFold(app)
  }

  /// The app over the fake's `scenario` with nothing open, sat like a laptop, the main view holding the new-session
  /// screen at rest; and the Sidebar put away, where the system stands it over that screen.
  @MainActor
  private func launchedWithNothingOpen(_ scenario: String) throws -> XCUIApplication {
    let app = try Pose.laptop.launched(
      self, ["--fake", scenario, "--forget-scene", "-AutomaticMinimizationEnabled", "NO"])
    let composer = element("landing.composer", in: app)
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    if app.buttons["listing.scope"].waitForExistence(timeout: 3), app.buttons["listing.scope"].isHittable {
      // A tap beside it puts it away.
      app.point(across: 0.92, down: 0.3).tap()
      XCTAssertTrue(app.buttons["listing.scope"].waitForNonExistence(timeout: 10))
    }
    return app
  }

  @MainActor
  func testTheNewSessionScreensFieldStandsOnTheHalfBelowAndItsLineAboveTheFold() throws {
    let app = try launchedWithNothingOpen("offline")
    fieldStands(22, aboveTheEdgeIn: app, "landing.composer")
    let line = element("landing.unreachable", in: app)
    waits("the Unreachable line stands above the fold", in: app) {
      line.exists && line.frame.maxY <= band.lowerBound + 1 && line.frame.maxY > band.lowerBound - 40
    }
    let workspace = element("landing.asks", in: app)
    XCTAssertTrue(workspace.exists && workspace.frame.maxY <= line.frame.minY, "its middle above the line")
    // Nothing else below the fold: no Aside, no bar, nothing but the field.
    XCTAssertFalse(element("work.list", in: app).exists)
    XCTAssertFalse(element("tree.list", in: app).exists)
    let below = app.navigationBars.allElementsBoundByIndex.filter { $0.frame.minY >= band.upperBound - 1 }
    XCTAssertTrue(below.isEmpty, "\(below)")
    attach("Nothing open, sat like a laptop")

    let composer = element("landing.composer", in: app)
    for _ in 0..<2 {
      composer.tap()
      fieldStandsAboveTheFold(in: app, "landing.composer")
      XCTAssertTrue(holdsTheKeyboard(composer), "it keeps its focus")
      XCTAssertLessThanOrEqual(line.frame.maxY, composer.frame.midY - 20, "under its line")
      composer.typeText("hi")
      attach("The new-session screen's field above the fold")
      dragTheKeyboardDown(in: app)
      fieldStands(22, aboveTheEdgeIn: app, "landing.composer")
    }
  }

  @MainActor
  func testWithNothingOpenTheWorkspacePickerScrollsOnThroughTheFold() throws {
    let app = try launchedWithNothingOpen("composing")
    let elsewhere = app.buttons["landing.elsewhere"]
    XCTAssertTrue(elsewhere.waitForExistence(timeout: 10))
    elsewhere.tap()
    let picker = element("picker.screen", in: app)
    XCTAssertTrue(picker.waitForExistence(timeout: 10))
    XCTAssertGreaterThanOrEqual(picker.frame.maxY, app.window.frame.maxY - 1, "to the display's foot")
    let last = element("workspace/workstation/photo-archive", in: app)
    app.scrolled(to: last, over: picker)
    let rows = elements("workspace/workstation/notes", in: app).firstMatch
    waits("its rows stand on through the fold", in: app) {
      [last, rows].contains { $0.exists && $0.frame.minY >= band.upperBound }
    }
    XCTAssertFalse(element("work.list", in: app).exists, "no Aside with nothing open")
    attach("The Workspace Picker sat like a laptop")
  }

  @MainActor
  func testWithNothingOpenTheSidebarCalledBackRunsOnBelowTheFoldOverTheNewSessionScreen() throws {
    let app = try launchedWithNothingOpen("composing")
    app.buttons["Show Sidebar"].firstMatch.tap()
    let list = app.collectionViews["listing.list"]
    XCTAssertTrue(app.buttons["listing.scope"].waitForExistence(timeout: 10))
    XCTAssertGreaterThan(list.frame.maxY, band.upperBound, "over both halves")
    attach("The Sidebar over the new-session screen")
    // Chosen from it, a Session stands as the first did.
    let row = element("listing.active", holding: "Tidy zsh startup time", in: app)
    app.scrolled(to: row, over: list).tap()
    asideStands(app, at: "Worked for 3m 12s")
    conversationEndsAtTheFold(app)
  }

  @MainActor
  func testLaunchedInThePoseTheArrangementStandsWithNothingSaidTwice() throws {
    for arguments in [[], ["--aside", "work"]] {
      let app = try launched("releases", "workstation-key/7", ["--notice-stands", "60"] + arguments)
      asideStands(app, at: "Worked for 3m 12s")
      conversationEndsAtTheFold(app)
      XCTAssertFalse(app.back.exists, "the Session the main view's root")
      XCTAssertFalse(app.buttons["listing.scope"].isHittable, "the main view alone")
      let notices = elements("notice", in: app)
      XCTAssertTrue(notices.firstMatch.waitForExistence(timeout: 10))
      XCTAssertEqual(notices.count, 1, "said once")
      XCTAssertLessThanOrEqual(notices.firstMatch.frame.maxY, band.lowerBound, "beneath the main view's bar")
      attach("Launched sat like a laptop")
      app.terminate()
    }
  }

  /// Pairing at the root of a phone holding no Remote, over the fake scanner, which holds up the workstation's Invite
  /// twenty seconds after it starts, so that it is still looking while the test looks at it: the scanner's picture
  /// stands whole in the upper half, its words over it at its foot, and the forms scroll through the fold. What holds
  /// that it folded is the picture ending at the fold, as open flat in portrait it runs on to the window's foot.
  @MainActor
  func testPairingsScannerStandsWholeInTheUpperHalfAndItsFormsScrollThroughTheFold() throws {
    let app = try Pose.laptop.launched(
      self,
      [
        "--fake", "fresh", "--forget-scene", "--scan", "shaman-v1-fake-workstation", "--scan-every", "20",
      ])
    XCTAssertTrue(app.staticTexts["Pair with a server"].waitForExistence(timeout: 10), "pairing alone is offered")
    let window = app.window.frame
    let form = app.collectionViews.firstMatch
    waits("where pairing begins runs through the fold", in: app) {
      form.exists && form.frame.minY < band.lowerBound && form.frame.maxY > band.upperBound + 300
    }
    attach("Where pairing begins, sat like a laptop")

    app.buttons["pairing.scan"].tap()
    let picture = element("scan.picture", in: app)
    let looking = app.staticTexts["scan.looking"]
    waits("the scanner's picture ends at the fold", in: app) {
      picture.exists && abs(picture.frame.maxY - band.lowerBound) <= 1
    }
    XCTAssertGreaterThan(picture.frame.minY, 0, "below the title's bar")
    XCTAssertEqual(picture.frame.width, window.width, accuracy: 1, "\(picture.frame)")
    XCTAssertTrue(looking.exists)
    XCTAssertLessThan(looking.frame.maxY, band.lowerBound, "its words over its picture")
    XCTAssertGreaterThan(looking.frame.minY, picture.frame.midY, "at its foot")
    XCTAssertEqual(looking.frame.midX, window.midX, accuracy: 1, "in the middle")
    attach("The scanner, sat like a laptop")

    // Scanning as ever, it takes the Invite it is shown.
    XCTAssertTrue(app.navigationBars["Pair"].waitForExistence(timeout: 60), "the confirmation of the Invite read")
    XCTAssertTrue(element("confirm.hostname", in: app).waitForExistence(timeout: 10))
    waits("the confirmation runs through the fold", in: app) {
      form.exists && form.frame.minY < band.lowerBound && form.frame.maxY > band.upperBound + 300
    }
    attach("The confirmation, sat like a laptop")
  }
}
