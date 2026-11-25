import XCTest

/// The app's sheets beside a vertical bar, over the fake: the five that push nothing turn it off, so that what they
/// hold spans the display, under a title in its middle and with "Decline" and "Cancel" in words, and the two that push
/// screens keep it. Where the bar is horizontal every sheet spans the display anyway.
final class SheetTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String, _ arguments: [String] = [], invite: String? = nil) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario] + arguments
    if let invite {
      app.launchEnvironment["SHAMAN_INVITE"] = invite
    }
    app.launch()
    return app
  }

  @MainActor
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any)[identifier]
  }

  /// The cell of a sheet's list that holds the element `identifier` names.
  @MainActor
  private func cell(holding identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.cells.containing(.any, identifier: identifier).firstMatch
  }

  /// How far the first of what a sheet holds across the display stands from the display's left edge, and how far the
  /// last does from its right, which one element alone may be both of.
  @MainActor
  private func gaps(
    beside first: XCUIElement, _ last: XCUIElement? = nil, in app: XCUIApplication, file: StaticString, line: UInt
  ) -> (left: CGFloat, right: CGFloat) {
    let last = last ?? first
    XCTAssertTrue(first.waitForExistence(timeout: 10), file: file, line: line)
    XCTAssertTrue(last.waitForExistence(timeout: 10), file: file, line: line)
    let display = app.window.frame
    return (first.frame.minX - display.minX, display.maxX - last.frame.maxX)
  }

  /// Holds that what stands from `first` to `last` spans the display, with a margin at either side and no inset.
  @MainActor
  private func spansTheDisplay(
    _ first: XCUIElement, to last: XCUIElement? = nil, in app: XCUIApplication, file: StaticString = #filePath,
    line: UInt = #line
  ) {
    let (left, right) = gaps(beside: first, last, in: app, file: file, line: line)
    XCTAssertEqual(left, right, accuracy: 8, "no inset at one side", file: file, line: line)
    XCTAssertLessThan(max(left, right), 48, "nor more than a margin at either", file: file, line: line)
  }

  /// Holds that `title` stands in the middle of the display, under the sheet's grabber.
  @MainActor
  private func isCentred(
    _ title: XCUIElement, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    XCTAssertTrue(title.waitForExistence(timeout: 10), file: file, line: line)
    XCTAssertEqual(title.frame.midX, app.window.frame.midX, accuracy: 2, title.label, file: file, line: line)
  }

  /// The title in a sheet's own bar that begins with `words`.
  @MainActor
  private func title(_ words: String, in app: XCUIApplication) -> XCUIElement {
    app.navigationBars.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", words)).firstMatch
  }

  /// Holds that a sheet's `button` stands in its bar across the head as `words`, which are wider than a symbol's
  /// circle.
  @MainActor
  private func isInWords(
    _ button: XCUIElement, _ words: String, file: StaticString = #filePath, line: UInt = #line
  ) {
    XCTAssertTrue(button.exists, "across the head, and not down the side", file: file, line: line)
    XCTAssertEqual(button.label, words, file: file, line: line)
    let frame = button.frame
    XCTAssertGreaterThan(frame.width, frame.height * 1.2, "\(frame) is as wide as words", file: file, line: line)
  }

  @MainActor
  func testAnApprovalSpansTheDisplay() {
    let app = launched("paired", ["--open", "workstation-key/3"])
    spansTheDisplay(element("approval.decline", in: app), to: element("approval.acceptOnce", in: app), in: app)
    isCentred(element("approval.title", in: app), in: app)
    attach("An approval spanning the display")
  }

  @MainActor
  func testAQuestionnaireSpansTheDisplayWithDeclineInWords() {
    let app = launched("paired", ["--open", "workstation-key/13"])
    spansTheDisplay(
      app.descendants(matching: .any).matching(identifier: "questionnaire.choice").firstMatch, in: app)
    isCentred(title("Question ", in: app), in: app)
    let decline = app.navigationBars.buttons["questionnaire.decline"]
    isInWords(decline, "Decline")
    attach("A questionnaire spanning the display")
  }

  @MainActor
  func testTheTreeSpansTheDisplay() {
    let app = launched("paired", ["--open", "workstation-key/12"])
    XCTAssertTrue(
      app.descendants(matching: .any).matching(identifier: "session.turn").firstMatch.waitForExistence(timeout: 10))
    let button = app.buttons["session.tree"]
    XCTAssertTrue(button.waitForExistence(timeout: 10))
    button.tap()
    // From its Marker to how long it has worked, as an entry's ground runs on beneath a bar at its side.
    let entry = app.buttons.matching(identifier: "tree.row").firstMatch
    spansTheDisplay(
      entry.activityIndicators.firstMatch,
      to: entry.staticTexts.matching(NSPredicate(format: "label MATCHES '([0-9]+m )?[0-9]+s'")).firstMatch, in: app)
    isCentred(app.staticTexts["tree.count"], in: app)
    attach("The tree spanning the display")
  }

  @MainActor
  func testAnUnreachableRemotesSheetSpansTheDisplay() {
    let app = launched("unreachable")
    XCTAssertTrue(app.navigationBars["Everywhere"].waitForExistence(timeout: 10))
    app.scrolled(to: app.buttons["listing.unreachable"]).tap()
    spansTheDisplay(element("unreachable.tryAgain", in: app), in: app)
    attach("An unreachable remote's sheet spanning the display")
  }

  @MainActor
  func testTheRelayLoginSpansTheDisplayWithCancelInWords() {
    let app = launched("relayLogin", ["--servers-and-device", "--login-confirms-after", "120"])
    let login = element("server.workstation.way.2.login", in: app)
    XCTAssertTrue(login.waitForExistence(timeout: 10))
    login.tap()
    XCTAssertTrue(app.staticTexts["login.code"].waitForExistence(timeout: 10))
    // The system's prompt to sign in through github.com, which covers the sheet.
    let prompt = XCUIApplication(bundleIdentifier: "com.apple.springboard").alerts.firstMatch
    XCTAssertTrue(prompt.waitForExistence(timeout: 10))
    prompt.buttons["Cancel"].tap()
    XCTAssertTrue(prompt.waitForNonExistence(timeout: 10))
    spansTheDisplay(cell(holding: "login.copy", in: app), in: app)
    let cancel = app.navigationBars.buttons["login.cancel"]
    isInWords(cancel, "Cancel")
    // Asserted on the closed Duo alone: on the Pro the title is too long to stand in the middle beside "Cancel", and
    // this asserts nothing.
    let title = title("Log in at ", in: app)
    if title.frame.width / 2 + 8 < app.window.frame.midX - cancel.frame.maxX {
      isCentred(title, in: app)
    }
    attach("The relay login spanning the display")
  }

  @MainActor
  func testTheSheetsThatPushScreensKeepTheBar() {
    let app = launched("paired")
    let servers = app.buttons["home.servers"]
    let compose = app.buttons["home.compose"]
    XCTAssertTrue(servers.waitForExistence(timeout: 10))
    XCTAssertTrue(compose.waitForExistence(timeout: 10))
    // The listing's own two, one over the other in a vertical bar and level with each other in a horizontal one.
    let vertical = abs(servers.frame.midY - compose.frame.midY) > servers.frame.height / 2
    servers.tap()
    keepsTheBar(cell(holding: "server.workstation", in: app), vertical: vertical, in: app)
    attach("Servers & device beside its bar")
    let pairAnother = app.scrolled(to: app.buttons["servers.pairAnother"])
    pairAnother.tap()
    XCTAssertTrue(app.buttons["pairing.paste"].waitForExistence(timeout: 10))
    XCTAssertEqual(app.barStandsVertically, vertical, "Back over what it pushes, in the bar")

    let confirming = launched("paired", invite: "shaman-v1-fake-workstation")
    keepsTheBar(cell(holding: "confirm.hostname", in: confirming), vertical: vertical, in: confirming)
    attach("Pairing's confirmation beside its bar")
  }

  /// Holds that what `cell` stands for gives up room at one side for a bar, where the bar is `vertical`.
  @MainActor
  private func keepsTheBar(
    _ cell: XCUIElement, vertical: Bool, in app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line
  ) {
    guard vertical else { return spansTheDisplay(cell, in: app, file: file, line: line) }
    let (left, right) = gaps(beside: cell, in: app, file: file, line: line)
    XCTAssertGreaterThan(abs(left - right), 40, "room for its bar", file: file, line: line)
  }
}
