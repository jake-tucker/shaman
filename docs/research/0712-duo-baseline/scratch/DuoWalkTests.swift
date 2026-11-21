// SCRATCH for #712. Never commit: walks the screens a launch cannot reach and asks a watcher on the Mac for a
// screenshot of each, by a file under /tmp/duo712/req the watcher removes once it has taken it.
import XCTest

final class DuoWalkTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = true
  }

  @MainActor
  private func launched(_ arguments: [String], environment: [String: String] = [:]) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = arguments
    app.launchEnvironment = environment
    app.launch()
    return app
  }

  @MainActor
  private func capture(_ name: String, settle: TimeInterval = 1.5) {
    Thread.sleep(forTimeInterval: settle)
    let request = "/tmp/duo712/req/\(name)"
    FileManager.default.createFile(atPath: request, contents: Data())
    let deadline = Date().addingTimeInterval(30)
    while FileManager.default.fileExists(atPath: request), Date() < deadline {
      Thread.sleep(forTimeInterval: 0.2)
    }
  }

  @MainActor
  private func note(_ name: String, _ text: String) {
    try? text.write(toFile: "/tmp/duo712/notes/\(name).txt", atomically: true, encoding: .utf8)
  }

  @MainActor
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any).matching(identifier: identifier).firstMatch
  }

  @MainActor
  @discardableResult
  private func tap(_ identifier: String, in app: XCUIApplication, timeout: TimeInterval = 10) -> Bool {
    let found = element(identifier, in: app)
    guard found.waitForExistence(timeout: timeout) else {
      XCTFail("no \(identifier)")
      return false
    }
    found.tap()
    return true
  }

  @MainActor
  func testListingSearchAndScope() {
    let app = launched(["--fake", "several", "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    note("listing-frames", frames(app, ["listing.scope", "home.servers", "home.beginsIn", "home.compose"]))
    tap("listing.scope", in: app)
    capture("listing-scope-menu")
    app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.5)).tap()
    Thread.sleep(forTimeInterval: 1.5)
    let search = app.searchFields.firstMatch
    if search.waitForExistence(timeout: 5) {
      search.tap()
      capture("listing-search-focused")
      search.typeText("zsh")
      capture("listing-search-typed")
    } else {
      XCTFail("no search field")
    }
  }

  @MainActor
  func testListingScrolled() {
    let app = launched(["--fake", "several", "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    scroll(app)
    capture("listing-scrolled")
    scroll(app)
    scroll(app)
    capture("listing-scrolled-end")
    scroll(app, up: false)
    capture("listing-scrolled-back", settle: 1)
  }

  @MainActor
  func testServersSheetPairingAndScanner() {
    let app = launched(["--fake", "several", "--forget-scene", "--camera", "granted"])
    tap("home.servers", in: app)
    capture("sheet-servers")
    tap("servers.pairAnother", in: app)
    capture("sheet-servers-pairing")
    tap("pairing.scan", in: app)
    capture("sheet-servers-scanner", settle: 3)
  }

  @MainActor
  func testScannerAtTheRoot() {
    let app = launched(["--fake", "fresh", "--scan", "https://example.com", "--scan-every", "600"])
    tap("pairing.scan", in: app)
    capture("root-scanner", settle: 3)
  }

  @MainActor
  func testUnreachableSheet() {
    let app = launched(["--fake", "unreachable", "--forget-scene"])
    let row = element("listing.unreachable", in: app)
    for _ in 0..<6 where !(row.exists && row.isHittable) { scroll(app) }
    capture("listing-unreachable-foot")
    if tap("listing.unreachable", in: app) {
      capture("sheet-unreachable", settle: 2.5)
    }
  }

  @MainActor
  func testWorkAndDetail() {
    let app = launched(["--fake", "paired", "--open", "workstation-key/7"])
    let cards = app.descendants(matching: .any).matching(identifier: "session.turn")
    guard cards.firstMatch.waitForExistence(timeout: 10) else { return XCTFail("no card") }
    cards.element(boundBy: cards.count - 1).tap()
    XCTAssertTrue(element("work.list", in: app).waitForExistence(timeout: 5))
    capture("work")
    let rows = app.descendants(matching: .any).matching(identifier: "work.row")
    if rows.firstMatch.waitForExistence(timeout: 5) {
      rows.firstMatch.tap()
      capture("work-detail")
    }
  }

  @MainActor
  func testTreeSheet() {
    let app = launched(["--fake", "paired", "--open", "workstation-key/12"])
    let tree = element("session.tree", in: app)
    guard tree.waitForExistence(timeout: 10) else { return XCTFail("no tree button") }
    note("tree-button-frame", frames(app, ["session.tree", "session.composer", "session.bar", "session.header"]))
    tree.tap()
    XCTAssertTrue(element("tree.sheet", in: app).waitForExistence(timeout: 5))
    capture("sheet-tree", settle: 3)
  }

  @MainActor
  func testSidekickTreeSheet() {
    let app = launched(["--fake", "sidekick", "--open", "workstation-key/20"])
    let tree = element("session.tree", in: app)
    guard tree.waitForExistence(timeout: 10) else { return XCTFail("no tree button") }
    capture("session-sidekick")
    tree.tap()
    capture("sheet-tree-sidekick", settle: 3)
  }

  @MainActor
  func testRelayLoginSheet() {
    let app = launched(["--fake", "relayLogin", "--servers-and-device", "--login-confirms-after", "600"])
    let login = app.buttons.matching(NSPredicate(format: "identifier ENDSWITH %@", ".login")).firstMatch
    guard login.waitForExistence(timeout: 10) else { return XCTFail("no login") }
    login.tap()
    XCTAssertTrue(element("login.code", in: app).waitForExistence(timeout: 10))
    capture("sheet-relay-login")
  }

  @MainActor
  func testWorkspacePicker() {
    let app = launched(["--fake", "composing", "--compose"])
    tap("landing.elsewhere", in: app)
    XCTAssertTrue(element("picker.screen", in: app).waitForExistence(timeout: 5))
    capture("picker")
    let search = app.searchFields.firstMatch
    if search.waitForExistence(timeout: 3) {
      search.tap()
      capture("picker-search-focused")
    }
  }

  @MainActor
  func testQuestionnaireFields() {
    let app = launched(["--fake", "paired", "--open", "workstation-key/13"])
    guard element("questionnaire.sheet", in: app).waitForExistence(timeout: 10) else { return XCTFail("no sheet") }
    Thread.sleep(forTimeInterval: 1.5)
    let choices = app.descendants(matching: .any).matching(identifier: "questionnaire.choice")
    choices.firstMatch.tap()
    let field = element("questionnaire.field", in: app)
    if field.exists {
      field.tap()
      capture("sheet-questionnaire-field-focused")
    }
    tap("questionnaire.next", in: app)
    capture("sheet-questionnaire-2")
    let second = element("questionnaire.field", in: app)
    if second.waitForExistence(timeout: 3) {
      second.tap()
      second.typeText("yes")
    }
    tap("questionnaire.next", in: app)
    capture("sheet-questionnaire-3")
  }

  @MainActor
  func testApprovalLarge() {
    let app = launched(["--fake", "paired", "--open", "workstation-key/3"])
    let sheet = element("approval.title", in: app)
    guard sheet.waitForExistence(timeout: 10) else { return XCTFail("no approval") }
    Thread.sleep(forTimeInterval: 1.5)
    note("approval-frames", frames(app, ["approval.title", "approval.sheet", "approval.decline", "approval.accept"]))
    sheet.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.1)))
    capture("sheet-approval-large")
  }

  @MainActor
  func testComposerKeyboardAndBanner() {
    let app = launched(["--fake", "paired", "--open", "workstation-key/7"])
    let composer = element("session.composer", in: app)
    guard composer.waitForExistence(timeout: 10) else { return XCTFail("no composer") }
    composer.tap()
    capture("session-keyboard")
    composer.typeText(
      "One line after another to see the composer grow as it does on a phone, until it is several lines tall and has to scroll within itself. And then some more words to make sure of it, fail"
    )
    capture("session-composer-long")
    tap("session.send", in: app)
    capture("session-banner", settle: 3)
  }

  @MainActor
  func testLongSessionScrolls() {
    let app = launched(["--fake", "longSession", "--open", "workstation-key/30"])
    guard element("session.conversation", in: app).waitForExistence(timeout: 15) else { return XCTFail("no list") }
    capture("long-foot", settle: 3)
    let list = element("session.conversation", in: app)
    list.swipeDown()
    capture("long-scrolled-1")
    list.swipeDown()
    list.swipeDown()
    capture("long-scrolled-3")
  }

  /// Holds a long Session open at its foot, or scrolled away from it, for as long as the watcher wants: a file under
  /// /tmp/duo712/req named "hold" ends it. The watcher poses the device meanwhile.
  @MainActor
  func testHoldLongSession() {
    let session = ProcessInfo.processInfo.environment["DUO_HOLD_SESSION"] ?? "workstation-key/30"
    let app = launched(["--fake", "longSession", "--open", session])
    guard element("session.conversation", in: app).waitForExistence(timeout: 15) else { return XCTFail("no list") }
    if ProcessInfo.processInfo.environment["DUO_HOLD_SCROLLED"] != nil {
      Thread.sleep(forTimeInterval: 3)
      element("session.conversation", in: app).swipeDown()
    }
    capture("hold-ready", settle: 3)
    FileManager.default.createFile(atPath: "/tmp/duo712/req/hold", contents: Data())
    let deadline = Date().addingTimeInterval(600)
    while FileManager.default.fileExists(atPath: "/tmp/duo712/req/hold"), Date() < deadline {
      Thread.sleep(forTimeInterval: 0.5)
    }
  }

  /// Split View multitasking, by the drag #710 found: the app to the left half, then what it shows there.
  @MainActor
  func testSplitView() {
    let app = launched(["--fake", "several", "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    let window = app.windows.firstMatch
    let indicator = window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.995))
    let dx = Double(ProcessInfo.processInfo.environment["DUO_SPLIT_DX"] ?? "0.03") ?? 0.03
    let dy = Double(ProcessInfo.processInfo.environment["DUO_SPLIT_DY"] ?? "0.5") ?? 0.5
    let side = window.coordinate(withNormalizedOffset: CGVector(dx: dx, dy: dy))
    indicator.press(forDuration: 0.6, thenDragTo: side, withVelocity: .slow, thenHoldForDuration: 1.0)
    capture("split-listing", settle: 4)
    note("split-window", "window=\(app.windows.firstMatch.frame) app=\(app.frame)")
    let row = element("listing.active", in: app)
    if row.exists {
      row.tap()
      capture("split-session", settle: 3)
      let composer = element("session.composer", in: app)
      if composer.waitForExistence(timeout: 5) {
        composer.tap()
        capture("split-session-keyboard")
      }
    }
    hold()
  }

  /// A second window, asked of the Home Screen: Shaman in one half, then its icon tapped in the other.
  @MainActor
  func testSecondWindowFromHomeScreen() {
    let app = launched(["--fake", "several", "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    let window = app.windows.firstMatch
    let indicator = window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.995))
    let side = window.coordinate(withNormalizedOffset: CGVector(dx: 0.03, dy: 0.5))
    indicator.press(forDuration: 0.6, thenDragTo: side, withVelocity: .slow, thenHoldForDuration: 1.0)
    capture("second-split", settle: 4)
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let icon = springboard.icons["Shaman"].firstMatch
    var lines = ["icon exists=\(icon.exists)"]
    if icon.exists {
      lines.append("icon frame=\(icon.frame) hittable=\(icon.isHittable)")
      icon.tap()
      capture("second-after-icon-tap", settle: 4)
      lines.append("app windows=\(app.windows.count) frames=\(app.windows.allElementsBoundByIndex.map { $0.frame })")
    }
    note("second-home", lines.joined(separator: "\n"))
    hold()
  }

  /// A second window, asked of the icon's own menu on the Home Screen.
  @MainActor
  func testSecondWindowIconMenu() {
    let app = launched(["--fake", "several", "--forget-scene"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    XCUIDevice.shared.press(.home)
    Thread.sleep(forTimeInterval: 2)
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let icon = springboard.icons["Shaman"].firstMatch
    guard icon.waitForExistence(timeout: 5) else {
      capture("second-no-icon")
      return XCTFail("no icon")
    }
    icon.press(forDuration: 1.3)
    capture("second-icon-menu", settle: 1)
    let labels = springboard.buttons.allElementsBoundByIndex.map { $0.label }
    note("second-icon-menu", labels.joined(separator: "\n"))
  }

  /// A second window, asked by the app itself: the scratch probe asks UIKit for one a few seconds after launch.
  @MainActor
  func testSecondWindowAsked() {
    let app = launched(["--fake", "several", "--forget-scene", "--second-window"])
    XCTAssertTrue(element("listing.active", in: app).waitForExistence(timeout: 10))
    capture("second-asked", settle: 9)
    note(
      "second-asked",
      "app windows=\(app.windows.count) frames=\(app.windows.allElementsBoundByIndex.map { $0.frame })")
    hold()
  }

  /// The tree's own banner, a Subagent's stop refused.
  @MainActor
  func testTreeBanner() {
    let app = launched(["--fake", "subagentStopRefused", "--open", "workstation-key/12"])
    let tree = element("session.tree", in: app)
    guard tree.waitForExistence(timeout: 10) else { return XCTFail("no tree button") }
    tree.tap()
    if tap("tree.stop", in: app) {
      let confirm = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Stop")).allElementsBoundByIndex
        .last
      confirm?.tap()
      capture("sheet-tree-banner", settle: 3)
    }
  }

  /// Waits, for as long as `DUO_HOLD` seconds, until the watcher removes /tmp/duo712/req/hold.
  @MainActor
  private func hold() {
    let seconds = TimeInterval(ProcessInfo.processInfo.environment["DUO_HOLD"] ?? "0") ?? 0
    guard seconds > 0 else { return }
    FileManager.default.createFile(atPath: "/tmp/duo712/req/hold", contents: Data())
    let deadline = Date().addingTimeInterval(seconds)
    while FileManager.default.fileExists(atPath: "/tmp/duo712/req/hold"), Date() < deadline {
      Thread.sleep(forTimeInterval: 0.5)
    }
    try? FileManager.default.removeItem(atPath: "/tmp/duo712/req/hold")
  }

  /// A drag up the window, or down it: the app's own frame reads the inner display unturned, so its swipes miss.
  @MainActor
  private func scroll(_ app: XCUIApplication, up: Bool = true) {
    let window = app.windows.firstMatch
    let low = window.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.75))
    let high = window.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.25))
    if up {
      low.press(forDuration: 0.05, thenDragTo: high)
    } else {
      high.press(forDuration: 0.05, thenDragTo: low)
    }
    Thread.sleep(forTimeInterval: 0.8)
  }

  @MainActor
  private func frames(_ app: XCUIApplication, _ identifiers: [String]) -> String {
    var lines = ["window=\(app.windows.firstMatch.frame)"]
    for identifier in identifiers {
      let found = element(identifier, in: app)
      lines.append("\(identifier)=\(found.exists ? "\(found.frame)" : "absent")")
    }
    return lines.joined(separator: "\n")
  }
}
