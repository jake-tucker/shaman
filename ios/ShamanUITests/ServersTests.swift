import XCTest

/// The Servers section of the sheet of Servers, Relays and This device, over the fake: a Remote is renamed from its row
/// and removed by a swipe with a confirmation; "Pair another server" opens the pairing screen; where the Pairing under
/// the Outlook ends, by removal on either side, the Outlook turns to another Remote saying why, or the phone, left with
/// none, offers pairing alone; and "This device" says what a change of name reaches.
final class ServersTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String, _ arguments: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--forget-scene"] + arguments
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

  /// Waits for `element` to go, answering whether it went within `timeout`.
  private func gone(_ element: XCUIElement, within timeout: TimeInterval = 10) -> Bool {
    let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element)
    return XCTWaiter.wait(for: [expectation], timeout: timeout) == .completed
  }

  /// Opens the sheet of Servers, Relays and This device from the listing's bottom bar.
  @MainActor
  private func openSheet(_ app: XCUIApplication) {
    let servers = app.buttons["home.servers"]
    XCTAssertTrue(servers.waitForExistence(timeout: 10))
    servers.tap()
    XCTAssertTrue(app.navigationBars["Servers & device"].waitForExistence(timeout: 10))
  }

  /// Closes the sheet, as a swipe down does.
  @MainActor
  private func closeSheet(_ app: XCUIApplication) {
    let bar = app.navigationBars["Servers & device"]
    bar.swipeDown(velocity: .fast)
    XCTAssertTrue(gone(bar), "the sheet closed")
  }

  /// Writes `name` over the name of the Remote `remote` in its row, and keeps it.
  @MainActor
  private func rename(_ remote: String, to name: String, in app: XCUIApplication) {
    let field = app.textFields["server.\(remote)"]
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    // At its trailing end, where the name ends, so every character of it is deleted.
    field.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
    let written = field.value as? String ?? ""
    field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: written.count) + name + "\n")
  }

  /// What the core says of its own accord, beginning `words`, wherever it stands.
  @MainActor
  private func notice(beginning words: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", words)).firstMatch
  }

  /// Swipes the row of the Remote `remote` and confirms its removal, checking what is asked first.
  @MainActor
  private func remove(_ remote: String, in app: XCUIApplication) {
    let row = app.textFields["server.\(remote)"]
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    row.swipeLeft()
    let swiped = app.buttons["server.\(remote).remove"]
    XCTAssertTrue(swiped.waitForExistence(timeout: 5))
    swiped.tap()
    let confirm = app.buttons["server.remove.confirm"].firstMatch
    XCTAssertTrue(confirm.waitForExistence(timeout: 5))
    let asked = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "This phone forgets \(remote)"))
    XCTAssertTrue(asked.firstMatch.exists, "what removing ends is said first")
    attach(app, "Removing a server asks first")
    confirm.tap()
  }

  @MainActor
  func testARemoteIsRenamedFromItsRowARefusedNameStayingBesideWhy() {
    let app = launched("several")
    openSheet(app)
    attach(app, "The servers section")

    rename("workstation", to: "laptop", in: app)
    let refusal = app.staticTexts["server.workstation.refusal"]
    XCTAssertTrue(refusal.waitForExistence(timeout: 10))
    XCTAssertEqual(refusal.label, "This phone holds another server as laptop; choose another name")
    XCTAssertEqual(app.textFields["server.workstation"].value as? String, "laptop", "what was written stays")
    attach(app, "A refused server name")

    rename("workstation", to: "studio", in: app)
    let renamed = app.textFields["server.studio"]
    XCTAssertTrue(renamed.waitForExistence(timeout: 10))
    XCTAssertTrue(gone(refusal), "nothing refused stands")
    XCTAssertEqual(renamed.value as? String, "studio")
    attach(app, "A renamed server")

    closeSheet(app)
    XCTAssertEqual(element("home.beginsIn", in: app).label, "New sessions begin in studio", "the Outlook goes with it")
    attach(app, "The listing after a rename")
  }

  @MainActor
  func testARemoteRemovedByASwipeTurnsTheOutlookToAnotherSayingSo() {
    let app = launched("several")
    openSheet(app)
    remove("workstation", in: app)

    XCTAssertTrue(gone(app.textFields["server.workstation"]))
    XCTAssertTrue(app.textFields["server.laptop"].exists)
    let said = notice(beginning: "workstation was removed", in: app)
    XCTAssertTrue(said.waitForExistence(timeout: 10))
    XCTAssertEqual(said.label, "workstation was removed. New sessions begin in laptop")
    attach(app, "A server removed")

    closeSheet(app)
    XCTAssertEqual(element("home.beginsIn", in: app).label, "New sessions begin in laptop")
    attach(app, "The listing after a removal")
  }

  @MainActor
  func testAPhoneLeftWithNoRemoteOffersPairingAlone() {
    let app = launched("paired")
    openSheet(app)
    remove("workstation", in: app)

    XCTAssertTrue(app.buttons["pairing.paste"].waitForExistence(timeout: 10))
    XCTAssertFalse(app.buttons["home.servers"].exists, "no listing stands")
    attach(app, "A phone left with no server")
  }

  @MainActor
  func testPairAnotherServerOpensThePairingScreen() {
    let app = launched("paired")
    openSheet(app)
    let pairAnother = app.buttons["servers.pairAnother"]
    XCTAssertTrue(pairAnother.waitForExistence(timeout: 10))
    pairAnother.tap()
    XCTAssertTrue(app.buttons["pairing.paste"].waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["Pair with a server"].exists)
    attach(app, "Pairing another server")
  }

  @MainActor
  func testAServerThatRemovesThisPhoneTurnsTheOutlookToTheRemoteTurnedTowardBefore() {
    let app = launched("removedThere", ["--removed-after", "4"])
    let beginsIn = element("home.beginsIn", in: app)
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn.label, "New sessions begin in workstation")

    let said = notice(beginning: "workstation removed this phone", in: app)
    XCTAssertTrue(said.waitForExistence(timeout: 15))
    XCTAssertEqual(
      said.label, "workstation removed this phone; pair with it again to reach it. New sessions begin in laptop")
    XCTAssertEqual(beginsIn.label, "New sessions begin in laptop", "the laptop was turned toward before it")
    attach(app, "A server removed this phone")
  }

  @MainActor
  func testAPhoneWhoseOneServerRemovesItOffersPairingAloneSayingWhy() {
    let app = launched("lastRemovedThere", ["--removed-after", "3"])
    XCTAssertTrue(app.buttons["home.servers"].waitForExistence(timeout: 10))

    XCTAssertTrue(app.buttons["pairing.paste"].waitForExistence(timeout: 15))
    let said = notice(beginning: "workstation removed this phone", in: app)
    XCTAssertTrue(said.waitForExistence(timeout: 10))
    XCTAssertEqual(said.label, "workstation removed this phone; pair with it again to reach it")
    attach(app, "The one server removed this phone")
  }

  @MainActor
  func testThisDeviceSaysANewNameReachesServersPairedFromThenOn() {
    let app = launched("paired")
    openSheet(app)
    let note = app.staticTexts["device.name.note"]
    var swipes = 0
    while !(note.exists && note.isHittable) && swipes < 6 {
      app.swipeUp()
      swipes += 1
    }
    XCTAssertEqual(
      note.label,
      "Servers paired from now on know this phone by this name. Those already paired keep the name they were given.")
    attach(app, "This device")
  }
}
