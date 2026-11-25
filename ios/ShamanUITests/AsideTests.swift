import XCTest

/// The Aside beside the main view, on a turned iPhone 18 Pro Max: what shows it and what it shows, that its own button
/// alone takes it down, that while it is shown it stands in the Sidebar's place, and what becomes of what it holds as
/// the Session or its latest Turn changes; and the chooser beneath it, and the card it marks.
final class AsideTests: TurningTestCase {
  /// The Session in `paired` whose Agent delegated to Subagents, and so has a tree.
  private let delegating = "workstation-key/12"
  private let delegatingTitle = "Find what holds the session lock"

  /// Waits until the Aside stands beside the main view under a bar reading `title`, with Back in it where it `pushed`
  /// and nothing else, and the Sidebar behind its button.
  @MainActor
  private func asideStands(
    _ app: XCUIApplication, at title: String, pushed: Bool = false, file: StaticString = #filePath, line: UInt = #line
  ) {
    let bar = app.navigationBars[title]
    let main = element("session.conversation", in: app)
    let button = app.buttons["session.aside"]
    for _ in 0..<40 {
      if bar.exists, main.exists, main.frame.maxX <= bar.frame.minX + 1, button.exists, button.label == "Hide aside",
        bar.buttons.count == (pushed ? 1 : 0), !sidebarStands(app)
      {
        if pushed {
          XCTAssertTrue(bar.buttons["BackButton"].isHittable, "its own Back", file: file, line: line)
        }
        return
      }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail("the Aside stands at \(title)", file: file, line: line)
  }

  /// Waits until no Aside stands and the Sidebar is back beside the main view.
  @MainActor
  private func noAsideStands(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    columnsStand(app, file: file, line: line)
    waitForOne("session.aside", "label == 'Show aside'", in: app, file: file, line: line)
    XCTAssertFalse(element("work.list", in: app).exists, file: file, line: line)
    XCTAssertFalse(element("tree.row", in: app).exists, file: file, line: line)
  }

  @MainActor
  private func sidebarStands(_ app: XCUIApplication) -> Bool {
    let scope = app.buttons["listing.scope"]
    return scope.exists && scope.isHittable
  }

  /// The card of the Turn whose line reads `line`, where a tap reaches it.
  @MainActor @discardableResult
  private func card(_ line: String, in app: XCUIApplication) -> XCUIElement {
    app.scrolled(to: element("session.turn", holding: line, in: app), over: element("session.conversation", in: app))
  }

  /// Opens the Session the listing's row `title` names from the Sidebar, called back over the Aside.
  @MainActor
  private func openFromTheSidebar(_ title: String, in app: XCUIApplication) {
    app.buttons["Show Sidebar"].firstMatch.tap()
    open(title, in: app)
  }

  /// Sends `text` to the open Session, at rest, which begins a Turn a moment later, as its bar says by offering Stop, and
  /// puts the keyboard down where one is up, as it leaves the conversation no room for a card.
  @MainActor
  private func beginTurn(_ text: String, in app: XCUIApplication) {
    let composer = element("session.composer", in: app)
    composer.tap()
    composer.typeText(text)
    app.buttons["session.send"].tap()
    XCTAssertTrue(app.buttons["session.stop"].waitForExistence(timeout: 20), "a Turn began")
    let keyboard = app.keyboards.firstMatch
    guard keyboard.exists, keyboard.frame.minY < app.window.frame.maxY - 100 else { return }
    // From the foot of the conversation, above the bar, down over the keyboard.
    element("session.bar", in: app).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0))
      .withOffset(CGVector(dx: 0, dy: -20))
      .press(forDuration: 0.05, thenDragTo: app.point(across: 0.3, down: 0.98))
    XCTAssertTrue(keyboard.waitForNonExistence(timeout: 10), "the keyboard is put down")
  }

  /// Stops the open Session's Turn.
  @MainActor
  private func stopTurn(in app: XCUIApplication) {
    app.buttons["session.stop"].tap()
    XCTAssertTrue(app.buttons["session.stop"].waitForNonExistence(timeout: 10), "stopped")
  }

  /// Waits until the Aside stands saying `words` where its work would be, under a bar that says nothing.
  @MainActor
  private func asideSays(
    _ words: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    waitForOne("work.empty", "label == '\(words)'", in: app, file: file, line: line)
    waitForOne("session.aside", "label == 'Hide aside'", in: app, file: file, line: line)
    XCTAssertLessThanOrEqual(
      element("session.conversation", in: app).frame.maxX, element("work.empty", in: app).frame.minX, file: file,
      line: line)
    XCTAssertFalse(sidebarStands(app), file: file, line: line)
  }

  /// Waits until the chooser stands beneath the Aside's list, `list`, between "Work" and the tree titled `tree`, and
  /// marks the one `list` is of.
  @MainActor
  private func chooserStands(
    _ tree: String = "Subagents", beneath list: String, in app: XCUIApplication, file: StaticString = #filePath,
    line: UInt = #line
  ) {
    let (kind, other) = list == "tree.list" ? ("aside.tree", "aside.work") : ("aside.work", "aside.tree")
    waitForOne(kind, "selected == true", in: app, file: file, line: line)
    waitForOne(other, "selected == false", in: app, file: file, line: line)
    let work = app.buttons["aside.work"]
    let side = app.buttons["aside.tree"]
    XCTAssertEqual(work.label, "Work", file: file, line: line)
    XCTAssertEqual(side.label, tree, file: file, line: line)
    let above = element(list, in: app)
    XCTAssertTrue(above.waitForExistence(timeout: 10), file: file, line: line)
    XCTAssertLessThanOrEqual(above.frame.maxY, work.frame.minY + 1, "beneath it", file: file, line: line)
    XCTAssertGreaterThanOrEqual(work.frame.minX, above.frame.minX - 1, file: file, line: line)
    XCTAssertLessThanOrEqual(work.frame.maxX, side.frame.minX, file: file, line: line)
    XCTAssertTrue(work.isHittable && side.isHittable, file: file, line: line)
  }

  /// Waits until the card reading `line`, once it is on screen, is marked and no other is; or with none, that none is.
  @MainActor
  private func marked(
    _ line: String?, in app: XCUIApplication, file: StaticString = #filePath, at: UInt = #line
  ) {
    if let line { card(line, in: app) }
    let cards = elements("session.turn", in: app)
    let selected = cards.matching(NSPredicate(format: "selected == true"))
    for _ in 0..<40 {
      if selected.count == (line == nil ? 0 : 1), line == nil || selected.firstMatch.label.contains(line!) { return }
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTFail(
      "\(line ?? "nothing") marked, and not \(selected.allElementsBoundByIndex.map(\.label))", file: file, line: at)
  }

  @MainActor
  func testTheChooserStandsBeneathTheListOfASessionWithATreeAndGivesEitherKind() {
    let app = launched("paired", ["--open", delegating, "--aside", "work"], turned: true)
    titled(delegatingTitle, in: app)
    for _ in 0..<2 {
      // Its bar holds nothing but a title, which `asideStands` looks to.
      asideStands(app, at: "Working")
      chooserStands(beneath: "work.list", in: app)
      attach("The chooser, at the work")
      app.buttons["aside.tree"].tap()
      asideStands(app, at: "Subagents")
      chooserStands(beneath: "tree.list", in: app)
      attach("The chooser, at the tree")
      app.buttons["aside.work"].tap()
    }
    // The tree's last row is reached above it.
    app.buttons["aside.tree"].tap()
    asideStands(app, at: "Subagents")
    let last = element("tree.row", holding: "Review the lock's tests", in: app)
    let list = element("tree.list", in: app)
    app.scrolled(to: last, over: list)
    // On to its foot, the whole of the row above the chooser.
    list.swipeUp()
    for _ in 0..<20 where last.frame.maxY > app.buttons["aside.work"].frame.minY + 1 {
      RunLoop.current.run(until: Date().addingTimeInterval(0.5))
    }
    XCTAssertLessThanOrEqual(last.frame.maxY, app.buttons["aside.work"].frame.minY + 1)
    attach("The tree's last row above the chooser")
    last.tap()
    titled("Review the lock's tests", in: app)
    asideStands(app, at: "Subagents")
    chooserStands(beneath: "tree.list", in: app)

    // None where there is no tree, and then again, the kind being the tree still.
    openFromTheSidebar("Draw the invite as a QR code in /serve", in: app)
    asideStands(app, at: "Working")
    XCTAssertTrue(app.buttons["aside.tree"].waitForNonExistence(timeout: 10))
    XCTAssertFalse(app.buttons["aside.work"].exists)
    attach("No chooser")
    openFromTheSidebar(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    chooserStands(beneath: "tree.list", in: app)
  }

  @MainActor
  func testWorkOnTheChooserIsTheOneTapBackToTheLatestTurnWhichItFollows() {
    let app = launched("paired", ["--open", delegating, "--aside", "work"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Working")
    app.buttons["session.stop"].tap()
    let stopped = element("session.turn", holding: "Stopped after 3m 10s", in: app)
    XCTAssertTrue(stopped.waitForExistence(timeout: 10))
    beginTurn("Now the other lock", in: app)
    waitForOne("work.empty", "label == 'Nothing done yet'", in: app)
    let earlier = element("work.row", holding: "Explorer", in: app)
    XCTAssertFalse(earlier.exists)
    // From an earlier Turn, and from the tree with an earlier one held.
    for tree in [false, true] {
      card("Stopped after 3m 10s", in: app).tap()
      XCTAssertTrue(earlier.waitForExistence(timeout: 10))
      marked("Stopped after 3m 10s", in: app)
      chooserStands(beneath: "work.list", in: app)
      if tree {
        app.buttons["aside.tree"].tap()
        asideStands(app, at: "Subagents")
      }
      app.buttons["aside.work"].tap()
      waitForOne("work.empty", "label == 'Nothing done yet'", in: app)
      marked("Working", in: app)
      XCTAssertFalse(earlier.exists)
    }
  }

  @MainActor
  func testTheChoosersOtherSideReadsTheTreesOwnTitle() {
    let app = launched("sidekick", ["--open", "workstation-key/20", "--aside", "work"], turned: true)
    XCTAssertTrue(app.buttons["aside.tree"].waitForExistence(timeout: 10))
    chooserStands("Sessions", beneath: "work.list", in: app)
    app.buttons["aside.tree"].tap()
    asideStands(app, at: "Sessions")
    chooserStands("Sessions", beneath: "tree.list", in: app)
    attach("The chooser of a Sidekick's tree")
    app.terminate()
    // Also where the tree is not read, or cannot be.
    for scenario in ["unreadTree", "unreadableTree"] {
      let unread = launched(scenario, ["--open", delegating, "--aside", "tree"], turned: true)
      let said = scenario == "unreadTree" ? "tree.loading" : "tree.failed"
      XCTAssertTrue(element(said, in: unread).waitForExistence(timeout: 10))
      chooserStands(beneath: "tree.list", in: unread)
      unread.terminate()
    }
  }

  @MainActor
  func testTheCardWhoseWorkItShowsIsMarkedWhileItIsOnScreen() {
    let app = launched("paired", ["--open", "workstation-key/7"], turned: true)
    titled("Tidy zsh startup time", in: app)
    card("Worked for 3m 12s", in: app)
    marked(nil, in: app)
    let button = app.buttons["session.aside"]
    button.tap()
    asideStands(app, at: "Worked for 3m 12s")
    marked("Worked for 3m 12s", in: app)
    attach("The marked card")
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    marked("Worked for 1m 23s", in: app)
    // With an Activity's whole pushed over that Turn's work.
    app.scrolled(to: element("session.compaction", in: app), over: element("session.conversation", in: app)).tap()
    asideStands(app, at: "Compaction", pushed: true)
    XCTAssertTrue(card("Worked for 1m 23s", in: app).isSelected)
    // Not while it is not on screen.
    button.tap()
    noAsideStands(app)
    card("Worked for 1m 23s", in: app)
    marked(nil, in: app)
    button.tap()
    waitForOne("session.aside", "label == 'Hide aside'", in: app)
    marked("Worked for 1m 23s", in: app)
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    card("Worked for 1m 23s", in: app)
    marked(nil, in: app)
    turn(.landscapeLeft, app)
    asideStands(app, at: "Compaction", pushed: true)
    XCTAssertTrue(card("Worked for 1m 23s", in: app).isSelected)
    card("Worked for 3m 12s", in: app).tap()
    asideStands(app, at: "Worked for 3m 12s")
    // It moves with the Aside to a Turn that begins.
    beginTurn("Now the other shell", in: app)
    asideStands(app, at: "Working")
    marked("Working", in: app)
  }

  @MainActor
  func testNoCardIsMarkedWhileTheTreeShowsAndEachSessionsOwnIs() {
    let app = launched("paired", ["--open", delegating, "--aside", "tree"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    XCTAssertTrue(element("session.turn", in: app).waitForExistence(timeout: 10))
    marked(nil, in: app)
    app.buttons["aside.work"].tap()
    asideStands(app, at: "Working")
    marked("2 tools", in: app)
    attach("The marked card of a Session with a tree")
    // Over its parent a Subagent's Session marks its own, and Back the parent's again.
    element("work.row", holding: "Explorer", in: app).tap()
    titled("Find what held the lock", in: app)
    marked("1 tool", in: app)
    app.back.tap()
    titled(delegatingTitle, in: app)
    marked("2 tools", in: app)
    app.buttons["session.tree"].tap()
    asideStands(app, at: "Subagents")
    marked(nil, in: app)
    // Nor for a Session with no tree, whose work shows though the kind is the tree.
    openFromTheSidebar("Draw the invite as a QR code in /serve", in: app)
    asideStands(app, at: "Working")
    XCTAssertTrue(
      elements("session.turn", in: app).waitForOne(NSPredicate(format: "selected == true")), "its work shows")
  }

  @MainActor
  func testTappingTheMarkedCardReturnsItToTheTopOfThatTurnsWork() {
    let app = launched("paired", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    let latest = "Worked for 3m 12s"
    asideStands(app, at: latest)
    marked(latest, in: app)
    let list = element("work.list", in: app)
    let head = element("work.row", holding: "Read src/tui/serve_panel.rs", in: app)
    XCTAssertTrue(head.waitForExistence(timeout: 10))
    let stood = head.frame.minY
    let atItsTop = { () -> Bool in
      for _ in 0..<40 {
        if head.exists, abs(head.frame.minY - stood) < 1 { return true }
        RunLoop.current.run(until: Date().addingTimeInterval(0.5))
      }
      return false
    }
    let point = { (down: Double) in list.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: down)) }
    point(0.85).press(forDuration: 0.05, thenDragTo: point(0.2), withVelocity: .slow, thenHoldForDuration: 0.3)
    XCTAssertTrue(!head.exists || head.frame.minY < stood - 20, "scrolled from its top")
    attach("Scrolled from its top")
    card(latest, in: app).tap()
    XCTAssertTrue(atItsTop(), "returned from further down it")
    asideStands(app, at: latest)
    marked(latest, in: app)
    // From an Activity's whole, pushed from further down it.
    app.scrolled(to: element("work.row", holding: "cargo check", in: app), over: list).tap()
    asideStands(app, at: "Command", pushed: true)
    card(latest, in: app).tap()
    asideStands(app, at: latest)
    XCTAssertTrue(atItsTop(), "returned from an Activity's whole")

    // And at an earlier Turn, which it stays at.
    let earlier = "Worked for 1m 23s"
    card(earlier, in: app).tap()
    asideStands(app, at: earlier)
    app.scrolled(to: element("work.row", holding: "cargo run", in: app), over: list).tap()
    asideStands(app, at: "Command", pushed: true)
    marked(earlier, in: app)
    card(earlier, in: app).tap()
    asideStands(app, at: earlier)
    marked(earlier, in: app)
    XCTAssertTrue(element("work.row", holding: "Read src/tui/serve_overlay.rs", in: app).isHittable)
  }

  @MainActor
  func testTheKindCarriesFromOneSessionToTheNextAndASessionWithNoTreeShowsItsWork() {
    let invite = "Draw the invite as a QR code in /serve"
    let app = launched("paired", ["--open", delegating, "--aside", "work"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Working")
    app.buttons["session.tree"].tap()
    asideStands(app, at: "Subagents")

    // One with no tree.
    openFromTheSidebar(invite, in: app)
    asideStands(app, at: "Working")
    XCTAssertTrue(element("work.row", holding: "cargo nextest run qr", in: app).waitForExistence(timeout: 10))
    XCTAssertFalse(element("tree.row", in: app).exists)
    attach("A session with no tree, the kind still the tree")
    openFromTheSidebar(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    waitForOne("tree.row", "label CONTAINS '\(delegatingTitle)' AND selected == true", in: app)

    // A card turns the kind to work, which carries in its turn, and while no Aside is on screen.
    element("session.turn", in: app).tap()
    asideStands(app, at: "Working")
    openFromTheSidebar(invite, in: app)
    asideStands(app, at: "Working")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
    open(delegatingTitle, in: app)
    app.buttons["session.aside"].tap()
    asideStands(app, at: "Working")
    XCTAssertTrue(element("work.row", holding: "Explorer", in: app).waitForExistence(timeout: 10))
    app.buttons["session.tree"].tap()
    asideStands(app, at: "Subagents")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
    open(invite, in: app)
    open(delegatingTitle, in: app)
    app.buttons["session.aside"].tap()
    asideStands(app, at: "Subagents")
  }

  @MainActor
  func testWithinOneTreeTheTreeStandsAndOnlyItsHighlightMoves() {
    let app = launched("paired", ["--open", delegating, "--aside", "tree"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    let last = elements("tree.row", in: app)
    XCTAssertTrue(last.firstMatch.waitForExistence(timeout: 10))
    // Scrolled a little from its head, where a tree drawn afresh would stand.
    let list = element("tree.list", in: app)
    let point = { (down: Double) in list.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: down)) }
    point(0.8).press(forDuration: 0.05, thenDragTo: point(0.55), withVelocity: .slow, thenHoldForDuration: 0.3)
    let explorer = element("tree.row", holding: "Find what held the lock", in: app)
    XCTAssertTrue(explorer.isHittable)
    let stood = explorer.frame
    explorer.tap()
    titled("Find what held the lock", in: app)
    waitForOne("tree.row", "label CONTAINS 'Find what held the lock' AND selected == true", in: app)
    XCTAssertEqual(explorer.frame, stood, "where it stood")
    attach("The same tree, scrolled, its highlight moved")
    app.back.tap()
    titled(delegatingTitle, in: app)
    waitForOne("tree.row", "label CONTAINS '\(delegatingTitle)' AND selected == true", in: app)
    XCTAssertEqual(explorer.frame, stood)
  }

  @MainActor
  func testItFollowsTheLatestTurnAndHoldsAtAnEarlierOneUntilTheLatestIsAskedFor() {
    let app = launched("paired", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    beginTurn("Now the other shell", in: app)
    asideStands(app, at: "Working")
    waitForOne("work.empty", "label == 'Nothing done yet'", in: app)
    attach("The Aside at a Turn just begun")
    stopTurn(in: app)
    asideStands(app, at: "Stopped after 4m 12s")

    // Held at an earlier one as another begins.
    card("Worked for 3m 12s", in: app).tap()
    asideStands(app, at: "Worked for 3m 12s")
    beginTurn("And fish", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    XCTAssertTrue(element("work.row", holding: "Read src/tui/serve_panel.rs", in: app).exists)
    attach("The Aside held at an earlier Turn")
    // The latest Turn's card resumes following.
    card("Working", in: app).tap()
    asideStands(app, at: "Working")
    stopTurn(in: app)
    asideStands(app, at: "Stopped after 4m 12s")
    beginTurn("And nushell", in: app)
    asideStands(app, at: "Working")

    // Held at an earlier one, another Session opened and this one again begins at its latest.
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    openFromTheSidebar("Draw the invite as a QR code in /serve", in: app)
    asideStands(app, at: "Working")
    openFromTheSidebar("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
  }

  @MainActor
  func testItFollowsAndHoldsByTheSameRulesWhileItIsOffScreen() {
    let app = launched("paired", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
    beginTurn("Now the other shell", in: app)
    app.buttons["session.aside"].tap()
    asideStands(app, at: "Working")
    stopTurn(in: app)
    asideStands(app, at: "Stopped after 4m 12s")
    // Held, with no room for it.
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    beginTurn("And fish", in: app)
    turn(.landscapeLeft, app)
    asideStands(app, at: "Worked for 1m 23s")
  }

  @MainActor
  func testWithAnActivityPushedANewTurnChangesNothingAndBackReturnsToItsTurn() {
    let app = launched("paired", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    let check = element("work.row", holding: "cargo check", in: app)
    app.scrolled(to: check, over: element("work.list", in: app)).tap()
    asideStands(app, at: "Command", pushed: true)
    beginTurn("Now the other shell", in: app)
    asideStands(app, at: "Command", pushed: true)
    attach("An Activity's whole as a Turn began")
    app.navigationBars["Command"].buttons["BackButton"].tap()
    asideStands(app, at: "Worked for 3m 12s")
    XCTAssertTrue(check.waitForExistence(timeout: 10))
    // Held there, as at any earlier Turn.
    stopTurn(in: app)
    beginTurn("And fish", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    card("Working", in: app).tap()
    asideStands(app, at: "Working")
  }

  @MainActor
  func testASessionWithNoTurnSaysSoAndFillsInPlaceAsTheFirstBegins() {
    // Working only for a Prompt not yet delivered, which a stop gives back.
    let app = launched("paired", ["--open", "workstation-key/11", "--aside", "work"], turned: true)
    titled("Note the floor in the pairing docs", in: app)
    asideSays("Nothing done yet", in: app)
    XCTAssertFalse(app.buttons["session.tree"].exists)
    attach("A session with no Turn, and the Aside")
    let words = element("work.empty", in: app).frame
    app.buttons["session.stop"].tap()
    waitForOne("session.composer", "value == 'Note the floor in the pairing docs'", in: app)
    asideSays("Nothing done yet", in: app)
    app.buttons["session.send"].tap()
    asideStands(app, at: "Working")
    waitForOne("work.empty", "label == 'Nothing done yet'", in: app)
    XCTAssertEqual(element("work.empty", in: app).frame, words, "in place")
  }

  @MainActor
  func testASessionThatCannotBeReadSaysWhyInIt() {
    let app = launched("releases", ["--open", "laptop-key/4", "--aside", "tree", "--notice-stands", "1"], turned: true)
    let unread = element("session.unread", in: app)
    XCTAssertTrue(unread.waitForExistence(timeout: 10))
    asideSays(unread.label, in: app)
  }

  @MainActor
  func testItIsAbsentWhileNoSessionIsOpenAndReturnsAsAFirstPromptIsSent() {
    let app = launched("composing", ["--open", "workstation-key/7", "--aside", "work"], turned: true)
    titled("Tidy zsh startup time", in: app)
    asideStands(app, at: "Worked for 3m 12s")
    app.buttons["Show Sidebar"].firstMatch.tap()
    app.buttons["home.compose"].tap()
    columnsStand(app, title: "landing.title")
    let button = app.buttons["session.aside"]
    XCTAssertFalse(button.exists)
    XCTAssertFalse(element("work.list", in: app).exists)
    let composer = element("landing.composer", in: app)
    composer.tap()
    composer.typeText("Map the workspace")
    columnsStand(app, title: "landing.title")
    app.buttons["landing.send"].tap()
    // Over the Provisional Session, whose work is none yet, and then over the Session begun.
    titled("Map the workspace", in: app)
    asideSays("Nothing done yet", in: app)
    attach("The Aside over a Provisional Session")
    asideStands(app, at: "Working")
    attach("The Aside over the Session begun")
  }

  @MainActor
  func testItBeginsHiddenAndItsButtonShowsItAtTheLatestTurnsWorkAndTakesItDown() {
    let app = launched("paired", ["--open", "workstation-key/7"], turned: true)
    titled("Tidy zsh startup time", in: app)
    noAsideStands(app)
    let button = app.buttons["session.aside"]
    for _ in 0..<2 {
      button.tap()
      asideStands(app, at: "Worked for 3m 12s")
      XCTAssertTrue(element("work.row", holding: "Read src/tui/serve_panel.rs", in: app).isHittable)
      XCTAssertFalse(app.back.exists, "nothing is pushed")
      attach("The Aside beside a session")
      button.tap()
      noAsideStands(app)
    }
    // None while no Session is open.
    app.buttons["home.compose"].tap()
    columnsStand(app, title: "landing.title")
    XCTAssertFalse(button.exists)
    // And none in one column.
    open("Tidy zsh startup time", in: app)
    XCTAssertTrue(button.waitForExistence(timeout: 10))
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    XCTAssertTrue(button.waitForNonExistence(timeout: 10))
  }

  @MainActor
  func testATurnsCardShowsThatTurnsWorkAndAnActivityIsPushedWithinIt() {
    let app = launched("paired", ["--open", "workstation-key/7"], turned: true)
    titled("Tidy zsh startup time", in: app)
    columnsStand(app)
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    attach("An earlier Turn's work in the Aside")
    card("Worked for 3m 12s", in: app).tap()
    asideStands(app, at: "Worked for 3m 12s")

    // An Activity's whole, under the Aside's own Back, the main view standing as it was.
    let list = element("work.list", in: app)
    app.scrolled(to: element("work.row", holding: "cargo check", in: app), over: list).tap()
    asideStands(app, at: "Command", pushed: true)
    titled("Tidy zsh startup time", in: app)
    attach("An Activity's whole in the Aside")
    app.navigationBars["Command"].buttons["BackButton"].tap()
    asideStands(app, at: "Worked for 3m 12s")

    // A card tapped with one pushed shows its Turn's work, and a Compaction's whole is pushed there too.
    app.scrolled(to: element("work.row", holding: "cargo check", in: app), over: list).tap()
    asideStands(app, at: "Command", pushed: true)
    card("Worked for 1m 23s", in: app).tap()
    asideStands(app, at: "Worked for 1m 23s")
    app.scrolled(to: element("session.compaction", in: app), over: element("session.conversation", in: app)).tap()
    asideStands(app, at: "Compaction", pushed: true)
    app.navigationBars["Compaction"].buttons["BackButton"].tap()
    asideStands(app, at: "Worked for 1m 23s")
    XCTAssertFalse(app.back.exists, "nothing was pushed in the main view")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
  }

  @MainActor
  func testTheTreesButtonShowsTheTreeAndNeverTakesTheAsideDown() {
    let app = launched("paired", ["--open", delegating], turned: true)
    titled(delegatingTitle, in: app)
    columnsStand(app)
    let tree = app.buttons["session.tree"]
    XCTAssertTrue(tree.waitForExistence(timeout: 10))
    XCTAssertGreaterThan(app.buttons["session.aside"].frame.minX, tree.frame.minX, "after the tree's button")
    let entry = element("tree.row", holding: delegatingTitle, in: app)
    for _ in 0..<2 {
      tree.tap()
      asideStands(app, at: "Subagents")
      XCTAssertTrue(entry.isSelected, "the open Session's entry is highlighted")
      XCTAssertFalse(app.otherElements["tree.sheet"].exists)
    }
    attach("The tree in the Aside")
    // From work to the tree, and back.
    element("session.turn", in: app).tap()
    asideStands(app, at: "Working")
    tree.tap()
    asideStands(app, at: "Subagents")

    // An entry chosen is shown in the main view, beside the same tree.
    let explorer = element("tree.row", holding: "Find what held the lock", in: app)
    app.scrolled(to: explorer, over: element("tree.list", in: app)).tap()
    titled("Find what held the lock", in: app)
    asideStands(app, at: "Subagents")
    waitForOne("tree.row", "label CONTAINS 'Find what held the lock' AND selected == true", in: app)
    attach("A subagent's session beside the tree")
    app.back.tap()
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
  }

  @MainActor
  func testAReviewThatLeadsToTheTreeShowsItThere() {
    let app = launched("subagentsWait", ["--open", delegating], turned: true)
    titled(delegatingTitle, in: app)
    columnsStand(app)
    for _ in 0..<2 {
      let review = app.buttons["session.review"]
      XCTAssertTrue(review.waitForExistence(timeout: 10))
      review.tap()
      asideStands(app, at: "Subagents")
    }
    XCTAssertFalse(app.otherElements["tree.sheet"].exists)
  }

  @MainActor
  func testASubagentsRowInItPushesItsSessionInTheMainView() {
    let app = launched("paired", ["--open", delegating, "--aside", "work"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Working")
    element("work.row", holding: "Explorer", in: app).tap()
    titled("Find what held the lock", in: app)
    asideStands(app, at: "Working")
    XCTAssertTrue(element("work.row", holding: "Grepper", in: app).waitForExistence(timeout: 10), "its own work")
    attach("A subagent's session beside its work")
    app.back.tap()
    titled(delegatingTitle, in: app)
    XCTAssertTrue(element("work.row", holding: "Explorer", in: app).waitForExistence(timeout: 10))
    asideStands(app, at: "Working")
  }

  @MainActor
  func testTheSidebarCalledBackStandsOverBothAndASessionOpenedFromItTakesItsPlace() {
    let app = launched("paired", ["--open", delegating, "--aside", "work"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Working")
    let aside = element("work.list", in: app)
    let stood = aside.frame
    app.buttons["Show Sidebar"].firstMatch.tap()
    let scope = app.buttons["listing.scope"]
    XCTAssertTrue(scope.waitForExistence(timeout: 10))
    XCTAssertTrue(app.navigationBars["Working"].exists, "over the main view and the Aside, which stand beneath it")
    XCTAssertEqual(aside.frame, stood)
    XCTAssertLessThan(element("session.conversation", in: app).frame.minX, scope.frame.maxX)
    // The Aside is on screen beneath it, and its card marked.
    waitForOne("session.turn", "selected == true", in: app)
    attach("The Sidebar over the main view and the Aside")
    // Put away by a tap beside it.
    app.point(across: 0.5, down: 0.65).tap()
    asideStands(app, at: "Working")
    marked("2 tools", in: app)
    app.buttons["Show Sidebar"].firstMatch.tap()
    XCTAssertTrue(scope.waitForExistence(timeout: 10))
    waitForOne("session.turn", "selected == true", in: app)

    element("listing.active", holding: "Draw the invite as a QR code in /serve", in: app).tap()
    titled("Draw the invite as a QR code in /serve", in: app)
    // Shown still, at that Session's work.
    asideStands(app, at: "Working")
    XCTAssertTrue(element("work.row", holding: "cargo nextest run qr", in: app).waitForExistence(timeout: 10))
    XCTAssertFalse(app.back.exists)
    attach("Another session, the Aside still shown")
    app.buttons["session.aside"].tap()
    noAsideStands(app)
  }

  @MainActor
  func testAnInterventionPresentsItselfOverItAndItStandsAsItWas() {
    let app = launched(
      "subagentWaits", ["--open", delegating, "--aside", "tree", "--arming-moment", "0.3"], turned: true)
    let sheet = app.otherElements["approval.sheet"]
    XCTAssertTrue(sheet.waitForExistence(timeout: 10), "presented over the tree")
    attach("An Approval over the Aside")
    // At full height, as a short display has it, a sheet has no grabber.
    let top = app.staticTexts["approval.title"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    top.press(forDuration: 0.1, thenDragTo: top.withOffset(CGVector(dx: 0, dy: 360)))
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
    asideStands(app, at: "Subagents")
    XCTAssertTrue(element("tree.row", holding: delegatingTitle, in: app).isSelected)
    // And over a Subagent's Session and its work, where its own does, and is answered.
    element("session.turn", in: app).tap()
    XCTAssertTrue(element("work.row", holding: "Explorer", in: app).waitForExistence(timeout: 10))
    app.buttons["session.review"].tap()
    XCTAssertTrue(sheet.waitForExistence(timeout: 10))
    waitForOne("approval.acceptOnce", "isEnabled == true", in: app)
    app.buttons["approval.acceptOnce"].tap()
    XCTAssertTrue(sheet.waitForNonExistence(timeout: 10))
    titled("Find what held the lock", in: app)
    XCTAssertTrue(element("work.row", holding: "Grepper", in: app).waitForExistence(timeout: 10))
    asideStands(app, at: "Working")
    attach("The Aside after an Approval")
  }

  @MainActor
  func testNoNoticeIsSaidInIt() {
    let app = launched(
      "releases", ["--open", "workstation-key/7", "--aside", "work", "--notice-stands", "60"], turned: true)
    let notices = elements("notice", in: app)
    XCTAssertTrue(notices.firstMatch.waitForExistence(timeout: 10))
    let bar = app.navigationBars["Worked for 3m 12s"]
    XCTAssertTrue(bar.waitForExistence(timeout: 10))
    let aside = bar.frame.minX
    for notice in notices.allElementsBoundByIndex {
      XCTAssertLessThanOrEqual(notice.frame.maxX, aside + 1, "\(notice)")
    }
    attach("A notice beside the Aside")
  }

  @MainActor
  func testItIsOfferedOnlyOnceTurned() {
    let app = launched("paired", ["--open", delegating, "--aside", "tree"])
    oneColumnStands(app, showing: "session.title")
    XCTAssertTrue(app.buttons["session.tree"].waitForExistence(timeout: 10))
    let button = app.buttons["session.aside"]
    XCTAssertFalse(button.exists, "upright one column stands")
    XCTAssertFalse(element("tree.row", in: app).exists)
    turn(.landscapeLeft, app)
    asideStands(app, at: "Subagents")
    turn(.portrait, app)
    oneColumnStands(app, showing: "session.title")
    XCTAssertTrue(button.waitForNonExistence(timeout: 10))
  }

  @MainActor
  func testItIsShownAtLaunchAtItsTreeAndNeverWithoutASessionToOpen() {
    let app = launched("paired", ["--open", delegating, "--aside", "tree"], turned: true)
    titled(delegatingTitle, in: app)
    asideStands(app, at: "Subagents")
    app.terminate()
    let again = launched("paired", ["--aside", "tree"], turned: true)
    columnsStand(again, title: "landing.title")
    open("Draw the invite as a QR code in /serve", in: again)
    noAsideStands(again)
  }
}
