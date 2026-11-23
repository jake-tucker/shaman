// PROTOTYPE for #731. Throwaway: never merge. What the app's real screens do under UIKit's split view, a navigation
// controller in each column, beyond what the suite already asks of them: run on the iPhone 18 Pro, a closed Duo and an
// iPhone 18 Pro Max, which tiles the columns on its side.

import XCTest

final class PrototypeRealTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
  }

  override func tearDown() {
    XCUIDevice.shared.orientation = .portrait
  }

  @MainActor
  private func launched(_ scenario: String = "paired", _ more: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--container", "uikit", "--forget-scene"] + more
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
  private func elements(_ identifier: String, in app: XCUIApplication) -> XCUIElementQuery {
    app.descendants(matching: .any).matching(identifier: identifier)
  }

  @MainActor
  private func element(_ identifier: String, holding text: String, in app: XCUIApplication) -> XCUIElement {
    elements(identifier, in: app).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
  }

  @MainActor
  private func titled(_ expected: String, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
    let title = app.staticTexts.matching(identifier: "session.title")
      .matching(NSPredicate(format: "label == %@", expected)).firstMatch
    XCTAssertTrue(title.waitForExistence(timeout: 10), "the Session titled \(expected) shows", file: file, line: line)
  }

  @MainActor
  private func open(_ title: String, in app: XCUIApplication) {
    let row = element("listing.active", holding: title, in: app)
    XCTAssertTrue(row.waitForExistence(timeout: 10), "no row \(title)")
    row.tap()
  }

  @MainActor
  private func back(_ app: XCUIApplication) {
    let button = app.buttons["BackButton"].firstMatch
    XCTAssertTrue(button.waitForExistence(timeout: 5), "no Back")
    button.tap()
  }

  /// Turns the device on its side and says whether the columns stand: the listing beside what is open.
  @MainActor
  private func turned(_ app: XCUIApplication) -> Bool {
    XCUIDevice.shared.orientation = .landscapeLeft
    sleep(2)
    return elements("listing.active", in: app).firstMatch.exists && elements("session.turn", in: app).firstMatch.exists
  }

  /// A Turn's work, an Activity's whole and a Subagent's Session each push over the Session, and Back steps one
  /// screen at a time, only the last step returning to the listing.
  @MainActor
  func testWhatPushesAndBackStepsOneScreenAtATime() {
    let app = launched()
    open("Tidy zsh startup time", in: app)
    let cards = elements("session.turn", in: app)
    XCTAssertTrue(cards.firstMatch.waitForExistence(timeout: 10))
    cards.element(boundBy: cards.count - 1).tap()
    XCTAssertTrue(app.navigationBars["Worked for 3m 12s"].waitForExistence(timeout: 5), "a Turn's work pushed")
    element("work.row", holding: "Edited src/tui/serve_panel.rs", in: app).tap()
    XCTAssertTrue(app.navigationBars["File change"].waitForExistence(timeout: 5), "an Activity's whole pushed")
    attach(app, "An activity's whole, two deep")
    back(app)
    XCTAssertTrue(app.navigationBars["Worked for 3m 12s"].waitForExistence(timeout: 5), "back at the work")
    back(app)
    titled("Tidy zsh startup time", in: app)
    XCTAssertFalse(app.navigationBars["Worked for 3m 12s"].exists)
    back(app)
    XCTAssertTrue(app.buttons["listing.scope"].waitForExistence(timeout: 5), "back at the listing")
    XCTAssertTrue(element("listing.active", holding: "Tidy zsh startup time", in: app).isHittable)

    // And a Subagent's Session, pushed from a row of its parent's work.
    open("Find what holds the session lock", in: app)
    let card = elements("session.turn", in: app).firstMatch
    XCTAssertTrue(card.waitForExistence(timeout: 10))
    card.tap()
    let explorer = element("work.row", holding: "Explorer", in: app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 10))
    explorer.tap()
    titled("Find what held the lock", in: app)
    attach(app, "A subagent's session, pushed")
    back(app)
    XCTAssertTrue(element("work.row", holding: "Explorer", in: app).waitForExistence(timeout: 10), "back at the work")
    back(app)
    titled("Find what holds the session lock", in: app)
    back(app)
    XCTAssertTrue(app.buttons["listing.scope"].waitForExistence(timeout: 5), "back at the listing")
  }

  /// A swipe from the edge given up over a real Session leaves it open; one carried through returns to the listing.
  @MainActor
  func testASwipeGivenUpLeavesTheSessionOpen() {
    let app = launched()
    open("Tidy zsh startup time", in: app)
    titled("Tidy zsh startup time", in: app)
    let window = app.windows.firstMatch
    let edge = window.coordinate(withNormalizedOffset: CGVector(dx: 0.0, dy: 0.5))
    edge.press(
      forDuration: 0.1, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5)),
      withVelocity: .slow, thenHoldForDuration: 0.6)
    sleep(2)
    titled("Tidy zsh startup time", in: app)
    XCTAssertTrue(elements("session.turn", in: app).firstMatch.isHittable, "the conversation still stands")
    XCTAssertTrue(app.textViews["session.composer"].exists || app.textFields["session.composer"].exists)
    edge.press(
      forDuration: 0.1, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)),
      withVelocity: .slow, thenHoldForDuration: 0.1)
    XCTAssertTrue(app.buttons["listing.scope"].waitForExistence(timeout: 5), "back at the listing")
    XCTAssertTrue(element("listing.active", holding: "Tidy zsh startup time", in: app).waitForExistence(timeout: 5))
    // Opened again, it is read afresh and stands whole.
    open("Tidy zsh startup time", in: app)
    XCTAssertTrue(elements("session.turn", in: app).firstMatch.waitForExistence(timeout: 10))
  }

  /// "Begin somewhere else" pushes the Workspace Picker over the new-session screen, and a choice pops it.
  @MainActor
  func testThePickerPushesAndAChoicePops() {
    let app = launched("composing")
    let compose = app.buttons["home.compose"]
    XCTAssertTrue(compose.waitForExistence(timeout: 10), "no compose button")
    compose.tap()
    XCTAssertTrue(elements("landing.title", in: app).firstMatch.waitForExistence(timeout: 10))
    let elsewhere = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "somewhere else")).firstMatch
    XCTAssertTrue(elsewhere.waitForExistence(timeout: 5), "no offer to begin somewhere else")
    elsewhere.tap()
    XCTAssertTrue(elements("picker.screen", in: app).firstMatch.waitForExistence(timeout: 5), "the picker pushed")
    XCTAssertTrue(app.searchFields.firstMatch.exists, "its search field stands")
    attach(app, "The workspace picker, pushed")
    let row = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "dotfiles")).firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 5))
    row.tap()
    XCTAssertTrue(elements("picker.screen", in: app).firstMatch.waitForNonExistence(timeout: 5), "the choice popped it")
    XCTAssertTrue(elements("landing.title", in: app).firstMatch.exists)
    attach(app, "The new-session screen after a choice")
  }

  /// Turning changes the width class where the display is wide enough, and what is open stays open, two deep.
  @MainActor
  func testTurningKeepsWhatIsOpen() {
    let app = launched()
    open("Find what holds the session lock", in: app)
    let card = elements("session.turn", in: app).firstMatch
    XCTAssertTrue(card.waitForExistence(timeout: 10))
    card.tap()
    let explorer = element("work.row", holding: "Explorer", in: app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 10))
    explorer.tap()
    titled("Find what held the lock", in: app)
    for turn in 1...2 {
      XCUIDevice.shared.orientation = .landscapeLeft
      sleep(2)
      titled("Find what held the lock", in: app)
      attach(app, "On its side, \(turn)")
      XCUIDevice.shared.orientation = .portrait
      sleep(2)
      titled("Find what held the lock", in: app)
    }
    back(app)
    XCTAssertTrue(element("work.row", holding: "Explorer", in: app).waitForExistence(timeout: 10), "back at the work")
    back(app)
    titled("Find what holds the session lock", in: app)
    back(app)
    XCTAssertTrue(app.buttons["listing.scope"].waitForExistence(timeout: 5), "back at the listing")
  }

  /// Beside the listing a card shows its Turn's work in the Aside and is marked; the Aside waits through a turn
  /// upright with the Activity pushed within it, and a Subagent's Session pushed in the main view has a place of its
  /// own, the parent's returned by Back.
  @MainActor
  func testTheAsideHoldsRealWorkThroughATurnAndAPush() throws {
    let app = launched("paired", ["--tree-badge"])
    open("Find what holds the session lock", in: app)
    titled("Find what holds the session lock", in: app)
    try XCTSkipUnless(turned(app), "this display does not tile columns on its side")
    attach(app, "The listing beside the session")
    let card = elements("session.turn", in: app).firstMatch
    card.tap()
    let explorer = element("work.row", holding: "Explorer", in: app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 10), "the work stands in the Aside")
    XCTAssertTrue(card.exists, "and the conversation beside it")
    XCTAssertTrue(card.isSelected, "the card whose work the Aside shows is marked")
    XCTAssertTrue(app.buttons["session.aside"].exists, "the Aside's button is offered")
    XCTAssertEqual(app.buttons["session.aside"].label, "Hide aside")
    attach(app, "The aside showing a turn's work")

    // Upright there is no room: it waits, and stands again on its side.
    XCUIDevice.shared.orientation = .portrait
    sleep(2)
    XCTAssertFalse(explorer.exists && explorer.isHittable, "the Aside stands with no room")
    titled("Find what holds the session lock", in: app)
    XCUIDevice.shared.orientation = .landscapeLeft
    sleep(2)
    XCTAssertTrue(explorer.waitForExistence(timeout: 5), "the Aside stands again")
    XCTAssertTrue(explorer.isHittable)

    // A Subagent's row in the Aside pushes its Session in the main view, with a place of its own in the Aside.
    explorer.tap()
    titled("Find what held the lock", in: app)
    attach(app, "A subagent's session beside the aside")
    back(app)
    titled("Find what holds the session lock", in: app)
    XCTAssertTrue(explorer.waitForExistence(timeout: 5), "the parent's place in the Aside is returned")
    attach(app, "Back at the parent, its place in the aside returned")

    // The Aside's button takes it down, and the card brings it up again.
    app.buttons["session.aside"].tap()
    XCTAssertTrue(explorer.waitForNonExistence(timeout: 5), "taken down")
    XCTAssertFalse(card.isSelected, "no card is marked while it is down")
    XCTAssertEqual(app.buttons["session.aside"].label, "Show aside")
    XCTAssertTrue(element("listing.active", holding: "Tidy zsh", in: app).waitForExistence(timeout: 5), "the listing is back")
  }

  /// With the columns standing a notice is said once, in the main view; alone on screen the listing says it.
  @MainActor
  func testANoticeIsSaidOnce() {
    let app = launched("releases", ["--notice-stands", "120"])
    let notices = app.descendants(matching: .any).matching(identifier: "notice")
    XCTAssertTrue(notices.firstMatch.waitForExistence(timeout: 10))
    XCTAssertEqual(notices.count, 1, "said once upright")
    attach(app, "A notice upright")
    XCUIDevice.shared.orientation = .landscapeLeft
    sleep(2)
    XCTAssertTrue(notices.firstMatch.waitForExistence(timeout: 5))
    XCTAssertEqual(notices.count, 1, "said once on its side")
    attach(app, "A notice on its side")
  }

  /// A long conversation scrolled away from its foot keeps its place through a push and Back, and through turning.
  @MainActor
  func testAConversationKeepsItsPlaceThroughBackAndATurn() throws {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "longSession", "--container", "uikit", "--forget-scene", "--open", "workstation-key/30"]
    app.launch()
    let conversation = elements("session.conversation", in: app).firstMatch
    XCTAssertTrue(conversation.waitForExistence(timeout: 10))
    sleep(2)
    for _ in 0..<3 {
      conversation.swipeDown(velocity: .fast)
    }
    sleep(2)
    let all = elements("session.turn", in: app)
    let on = (0..<all.count).map { all.element(boundBy: $0) }.first { $0.isHittable }
    let card = try XCTUnwrap(on, "a card on screen away from the foot")
    let cardWords = card.label
    let before = card.frame.minY
    attach(app, "Away from the foot")
    card.tap()
    XCTAssertTrue(elements("work.list", in: app).firstMatch.waitForExistence(timeout: 5))
    back(app)
    let again = elements("session.turn", in: app).matching(NSPredicate(format: "label == %@", cardWords)).firstMatch
    XCTAssertTrue(again.waitForExistence(timeout: 5))
    sleep(1)
    XCTAssertEqual(again.frame.minY, before, accuracy: 1, "the same card stands where it stood")
    attach(app, "Back from its work")
    // Through a change of width the words at the top stay at the top.
    let texts = conversation.staticTexts
    let top = (0..<min(texts.count, 60)).map { texts.element(boundBy: $0) }
      .filter { $0.isHittable && $0.frame.minY > conversation.frame.minY + 120 && !$0.label.isEmpty }
      .min { $0.frame.minY < $1.frame.minY }
    let words = try XCTUnwrap(top, "no words at the top").label
    let kept = conversation.staticTexts.matching(NSPredicate(format: "label == %@", words)).firstMatch
    XCUIDevice.shared.orientation = .landscapeLeft
    sleep(3)
    XCTAssertTrue(kept.exists && kept.isHittable, "the words at the top are still on screen on its side: \(words)")
    attach(app, "On its side")
    XCUIDevice.shared.orientation = .portrait
    sleep(3)
    XCTAssertTrue(kept.exists && kept.isHittable, "and upright again: \(words)")
    XCTAssertTrue(again.exists && again.isHittable, "with the card that stood beneath them")
    attach(app, "Upright again")
  }

  /// What the bars carry: the scope menu, both search fields, the listing's foot, a Session's header and the tree's
  /// button, each found and, where it is a button, hittable.
  @MainActor
  func testWhatTheBarsCarry() {
    let app = launched("paired", ["--tree-badge"])
    XCTAssertTrue(app.buttons["listing.scope"].waitForExistence(timeout: 10), "the scope menu")
    XCTAssertTrue(app.buttons["listing.scope"].isHittable)
    XCTAssertTrue(app.searchFields.firstMatch.exists, "the listing's search field")
    XCTAssertTrue(app.buttons["home.servers"].isHittable, "Servers & device")
    XCTAssertTrue(app.buttons["home.compose"].isHittable, "New session")
    XCTAssertTrue(elements("home.beginsIn", in: app).firstMatch.exists, "where new Sessions begin")
    XCTAssertGreaterThan(
      app.buttons["home.compose"].frame.maxY, app.buttons["home.servers"].frame.minY, "New session is not above")
    attach(app, "The listing's bars")

    // The scope menu narrows.
    app.buttons["listing.scope"].tap()
    let dotfiles = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "dotfiles")).firstMatch
    XCTAssertTrue(dotfiles.waitForExistence(timeout: 5), "the menu opens")
    attach(app, "The scope menu open")
    dotfiles.tap()
    XCTAssertTrue(
      app.buttons.matching(identifier: "listing.scope").matching(NSPredicate(format: "label CONTAINS %@", "dotfiles"))
        .firstMatch.waitForExistence(timeout: 5), "narrowed")
    app.buttons["listing.scope"].tap()
    let everywhere = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "All workspaces")).firstMatch
    XCTAssertTrue(everywhere.waitForExistence(timeout: 5))
    everywhere.tap()

    // A Session with a tree: its header, and the tree's button wearing its count.
    open("Find what holds the session lock", in: app)
    titled("Find what holds the session lock", in: app)
    XCTAssertTrue(app.staticTexts["session.place"].exists, "its place under its title")
    let tree = app.buttons["session.tree"]
    XCTAssertTrue(tree.waitForExistence(timeout: 10), "the tree's button")
    XCTAssertTrue(tree.isHittable)
    attach(app, "A session's bars")
    tree.tap()
    XCTAssertTrue(element("tree.row", holding: "Find what holds the session lock", in: app).waitForExistence(timeout: 10))
    attach(app, "The tree")
  }

  /// Launches the app as the Mac asks (`PROTO_ARGS`) and does what it is told, one command at a time, from the file
  /// /tmp/duo731/cmd, which it removes once done: `tap <identifier>`, `label <button's label>`, `holding <words>`
  /// (taps the first element whose label holds them), `turn <left|upright>`, `back`, `swipe <from> <to>` (across the
  /// window's middle, as shares of its width), `focus`, `type <words>`, `has <identifier>`, `buttons`, `quit`.
  @MainActor
  func testDriven() {
    let environment = ProcessInfo.processInfo.environment
    let app = XCUIApplication()
    app.launchArguments = (environment["PROTO_ARGS"] ?? "--fake paired").split(separator: " ").map(String.init)
      + ["--forget-scene"]
    app.launch()
    let command = "/tmp/duo731/cmd"
    try? FileManager.default.removeItem(atPath: command)
    FileManager.default.createFile(atPath: "/tmp/duo731/ready", contents: Data())
    let deadline = Date().addingTimeInterval(2400)
    while Date() < deadline {
      guard let told = try? String(contentsOfFile: command, encoding: .utf8) else {
        Thread.sleep(forTimeInterval: 0.2)
        continue
      }
      let words = told.trimmingCharacters(in: .whitespacesAndNewlines).split(separator: " ", maxSplits: 1)
        .map(String.init)
      let rest = words.count > 1 ? words[1] : ""
      var said = "done"
      switch words.first ?? "" {
      case "tap":
        let found = app.descendants(matching: .any).matching(identifier: rest).firstMatch
        if found.waitForExistence(timeout: 5) { found.tap() } else { said = "no \(rest)" }
      case "label":
        let found = app.buttons.matching(NSPredicate(format: "label == %@", rest)).firstMatch
        if found.waitForExistence(timeout: 5) { found.tap() } else { said = "no \(rest)" }
      case "holding":
        let found = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", rest))
          .allElementsBoundByIndex.first { $0.isHittable }
        if let found { found.tap() } else { said = "no \(rest)" }
      case "turn":
        XCUIDevice.shared.orientation = rest == "left" ? .landscapeLeft : rest == "right" ? .landscapeRight : .portrait
      case "back":
        let found = app.buttons["BackButton"].firstMatch
        if found.waitForExistence(timeout: 5) { found.tap() } else { said = "no Back" }
      case "swipe":
        let shares = rest.split(separator: " ").compactMap { Double($0) }
        let window = app.windows.firstMatch
        if shares.count == 2 {
          window.coordinate(withNormalizedOffset: CGVector(dx: shares[0], dy: 0.5)).press(
            forDuration: 0.1, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: shares[1], dy: 0.5)),
            withVelocity: .slow, thenHoldForDuration: 0.4)
        }
      case "focus":
        let found = app.descendants(matching: .any).matching(identifier: "session.composer").firstMatch
        if found.waitForExistence(timeout: 5) { found.tap() } else { said = "no composer" }
      case "type":
        app.typeText(rest)
      case "has":
        let found = app.descendants(matching: .any).matching(identifier: rest)
        said = "\(found.count) " + found.allElementsBoundByIndex.prefix(6).map {
          "[\($0.label.prefix(50)) hittable=\($0.isHittable) \(Int($0.frame.minX)),\(Int($0.frame.minY)) \(Int($0.frame.width))x\(Int($0.frame.height))]"
        }.joined(separator: " ")
      case "buttons":
        said = app.buttons.allElementsBoundByIndex.map {
          "\($0.label)|\($0.identifier)|\(Int($0.frame.minX)),\(Int($0.frame.minY))"
        }.joined(separator: "\n")
      case "relaunch":
        // Ended and launched again, with these arguments: `kept` among them leaves out `--forget-scene`.
        var arguments = rest.split(separator: " ").map(String.init)
        if let kept = arguments.firstIndex(of: "kept") { arguments.remove(at: kept) } else { arguments.append("--forget-scene") }
        app.terminate()
        app.launchArguments = arguments
        app.launch()
      case "home":
        XCUIDevice.shared.press(.home)
      case "texts":
        said = app.staticTexts.allElementsBoundByIndex.prefix(40).map {
          "\($0.label.prefix(60))|\($0.identifier)|\(Int($0.frame.minX)),\(Int($0.frame.minY))"
        }.joined(separator: "\n")
      case "quit":
        try? FileManager.default.removeItem(atPath: command)
        return
      default:
        said = "unknown"
      }
      try? said.write(toFile: "/tmp/duo731/said", atomically: true, encoding: .utf8)
      try? FileManager.default.removeItem(atPath: command)
    }
  }
}
