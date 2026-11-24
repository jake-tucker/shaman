import XCTest

/// Pairing by scanning an Invite or pasting it, over the fake.
final class PairingTests: XCTestCase {
  /// The Invite the fake reads as the workstation's.
  private let invite = "shaman-v1-fake-workstation"

  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String, invite: String? = nil, arguments: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario] + arguments
    if let invite {
      app.launchEnvironment["SHAMAN_INVITE"] = invite
    }
    app.launch()
    return app
  }

  /// Pastes `text` as the user would, through the paste button.
  @MainActor
  private func paste(_ text: String, in app: XCUIApplication) {
    UIPasteboard.general.string = text
    let paste = app.buttons["pairing.paste"]
    XCTAssertTrue(paste.waitForExistence(timeout: 10))
    paste.tap()
  }

  @MainActor
  private func replace(_ field: XCUIElement, with text: String) {
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    field.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
    let written = field.value as? String ?? ""
    field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: written.count) + text + "\n")
  }

  /// The hostname the confirmation screen shows.
  @MainActor
  private func hostname(in app: XCUIApplication) -> String? {
    let hostname = app.descendants(matching: .any)["confirm.hostname"]
    XCTAssertTrue(hostname.waitForExistence(timeout: 10))
    return hostname.value as? String
  }

  /// The element the confirmation screen's Form shows under `identifier`, scrolled to. The Form makes a row as it nears
  /// the screen, so one beneath it is scrolled to before it is waited for.
  @MainActor
  private func shown(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    XCTAssertTrue(app.navigationBars["Pair"].waitForExistence(timeout: 10), "the confirmation screen")
    return app.scrolled(to: app.descendants(matching: .any)[identifier])
  }

  @MainActor
  func testAPastedInviteIsConfirmedAndPairedAndTheRemoteIsListedWithItsWays() {
    let app = launched("fresh")
    XCTAssertTrue(app.staticTexts["Pair with a server"].waitForExistence(timeout: 10), "pairing alone is offered")
    paste(invite, in: app)

    XCTAssertEqual(hostname(in: app), "workstation")
    let fingerprint = app.descendants(matching: .any)["confirm.fingerprint"]
    XCTAssertEqual((fingerprint.value as? String)?.count, 64)
    XCTAssertTrue(app.descendants(matching: .any)["confirm.way.0"].label.contains("192.168.1.20:7791"))
    XCTAssertTrue(app.descendants(matching: .any)["confirm.way.2"].label.contains("not dialled from a phone"))
    XCTAssertFalse(app.descendants(matching: .any)["confirm.held"].exists)
    XCTAssertEqual(app.textFields["confirm.remoteName"].value as? String, "workstation", "offered from the hostname")
    XCTAssertEqual(app.textFields["confirm.deviceName"].value as? String, "iPhone")
    attach("Confirming a pasted invite")

    replace(app.textFields["confirm.remoteName"], with: "desk")
    replace(app.textFields["confirm.deviceName"], with: "Pocket")
    shown("confirm.pair", in: app).tap()

    let beginsIn = app.descendants(matching: .any)["home.beginsIn"]
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10), "the Outlook turned toward the new Remote")
    XCTAssertEqual(beginsIn.label, "New sessions begin in desk")
    attach("Landed on the new Remote")

    app.buttons["home.servers"].tap()
    XCTAssertTrue(app.navigationBars["Servers & device"].waitForExistence(timeout: 10))
    XCTAssertFalse(app.navigationBars["Settings"].exists)
    XCTAssertTrue(app.descendants(matching: .any)["server.desk"].waitForExistence(timeout: 10))
    let answering = app.descendants(matching: .any)["server.desk.way.0"]
    XCTAssertTrue(answering.label.contains("192.168.1.20:7791"), answering.label)
    XCTAssertTrue(answering.label.hasSuffix("answering"), answering.label)
    let notAnswering = app.descendants(matching: .any)["server.desk.way.1"]
    XCTAssertTrue(notAnswering.label.hasSuffix("not answering"), notAnswering.label)
    XCTAssertEqual(app.textFields["device.name"].value as? String, "Pocket", "the name chosen is kept")
    attach("The Remote with its ways")
  }

  @MainActor
  func testAMalformedInviteShowsItsParseErrorOnTheConfirmationScreen() {
    let app = launched("fresh")
    paste("not an invite", in: app)
    let unreadable = app.staticTexts["confirm.unreadable"]
    XCTAssertTrue(unreadable.waitForExistence(timeout: 10))
    XCTAssertEqual(unreadable.label, "Invite must start with `shaman-`; was all of it copied?")
    XCTAssertFalse(app.buttons["confirm.pair"].exists)
    attach("A malformed invite")

    app.buttons["confirm.scanAgain"].tap()
    XCTAssertTrue(app.buttons["pairing.paste"].waitForExistence(timeout: 10))
  }

  @MainActor
  func testAPairingNoWayAnsweredStaysWithTryAgain() {
    let app = launched("fresh")
    paste("shaman-v1-unanswered", in: app)
    XCTAssertEqual(hostname(in: app), "workstation")
    shown("confirm.pair", in: app).tap()
    let failure = app.staticTexts["confirm.failure"]
    XCTAssertTrue(failure.waitForExistence(timeout: 10))
    XCTAssertTrue(failure.label.hasPrefix("No way this invite offers answered"), failure.label)
    XCTAssertEqual(hostname(in: app), "workstation", "still on the confirmation screen")
    XCTAssertTrue(shown("confirm.tryAgain", in: app).exists)
    attach("No way answered")
  }

  @MainActor
  func testADeadInviteStaysWithTheServersWordsAndScanAgain() {
    let app = launched("fresh")
    paste("shaman-v1-dead", in: app)
    XCTAssertEqual(hostname(in: app), "workstation")
    shown("confirm.pair", in: app).tap()
    let failure = app.staticTexts["confirm.failure"]
    XCTAssertTrue(failure.waitForExistence(timeout: 10))
    XCTAssertEqual(failure.label, "Invite has expired")
    shown("confirm.scanAgain", in: app).tap()
    XCTAssertTrue(app.buttons["pairing.paste"].waitForExistence(timeout: 10))
  }

  @MainActor
  func testAnInviteFromAServerAlreadyHeldSaysUnderWhichNameAndReplacesIt() {
    let app = launched("paired")
    XCTAssertTrue(app.buttons["home.servers"].waitForExistence(timeout: 10))
    app.buttons["home.servers"].tap()
    let pairAnother = app.buttons["servers.pairAnother"]
    XCTAssertTrue(pairAnother.waitForExistence(timeout: 10))
    pairAnother.tap()
    paste(invite, in: app)

    let held = app.descendants(matching: .any)["confirm.held"]
    XCTAssertTrue(held.waitForExistence(timeout: 10))
    XCTAssertTrue(held.label.contains("already paired with this server as workstation"), held.label)
    XCTAssertEqual(shown("confirm.remoteName", in: app).value as? String, "workstation", "pre-filled")
    attach("An invite from a server already held")

    replace(app.textFields["confirm.remoteName"], with: "studio")
    shown("confirm.pair", in: app).tap()
    // The listing stands in the hierarchy beneath the sheet, so it is read once the sheet has gone.
    XCTAssertTrue(app.navigationBars["Pair"].waitForNonExistence(timeout: 10), "paired, the sheet goes")
    let beginsIn = app.descendants(matching: .any)["home.beginsIn"]
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn.label, "New sessions begin in studio")
    app.buttons["home.servers"].tap()
    XCTAssertTrue(app.descendants(matching: .any)["server.studio"].waitForExistence(timeout: 10))
    XCTAssertFalse(app.descendants(matching: .any)["server.workstation"].exists, "one Remote for each Server")
  }

  /// The fake scanner's launch arguments, reading `codes` in turn.
  private func scanning(_ codes: String...) -> [String] {
    codes.flatMap { ["--scan", $0] }
  }

  @MainActor
  func testTheScannerPassesOverWhatIsNoInviteAndTheFirstInviteGoesToItsConfirmation() {
    let app = launched(
      "fresh", arguments: scanning("https://example.com", invite, "shaman-v1-fake-another") + ["--scan-every", "3"])
    let scan = app.buttons["pairing.scan"]
    XCTAssertTrue(scan.waitForExistence(timeout: 10))
    XCTAssertTrue(app.buttons["pairing.paste"].exists, "paste is offered beside scan")
    attach("Scan and paste")
    scan.tap()

    XCTAssertTrue(app.staticTexts["scan.looking"].waitForExistence(timeout: 10))
    XCTAssertEqual(app.staticTexts["scan.looking"].label, "Looking for a Shaman invite")
    let passedOver = app.staticTexts["scan.passedOver"]
    XCTAssertTrue(passedOver.waitForExistence(timeout: 10), "what is no invite is passed over")
    XCTAssertTrue(app.staticTexts["scan.looking"].exists, "and it keeps scanning")
    attach("Scanning past a code that is no invite")

    XCTAssertEqual(hostname(in: app), "workstation", "the first invite read goes to its confirmation")
    XCTAssertTrue(app.staticTexts["scan.looking"].waitForNonExistence(timeout: 10), "the scanner is covered")
    attach("Confirming a scanned invite")
    shown("confirm.pair", in: app).tap()
    let beginsIn = app.descendants(matching: .any)["home.beginsIn"]
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn.label, "New sessions begin in workstation")
  }

  @MainActor
  func testScanAgainAfterADeadScannedInviteReturnsToTheScanner() {
    let app = launched("fresh", arguments: scanning("shaman-v1-dead", invite))
    let scan = app.buttons["pairing.scan"]
    XCTAssertTrue(scan.waitForExistence(timeout: 10))
    scan.tap()
    XCTAssertEqual(hostname(in: app), "workstation")
    shown("confirm.pair", in: app).tap()
    let failure = app.staticTexts["confirm.failure"]
    XCTAssertTrue(failure.waitForExistence(timeout: 10))
    XCTAssertEqual(failure.label, "Invite has expired")
    shown("confirm.scanAgain", in: app).tap()

    XCTAssertTrue(app.staticTexts["scan.looking"].waitForExistence(timeout: 10), "back at the scanner")
    XCTAssertEqual(hostname(in: app), "workstation", "which read the next invite")
    XCTAssertFalse(app.staticTexts["confirm.failure"].exists)
    shown("confirm.pair", in: app).tap()
    XCTAssertTrue(app.descendants(matching: .any)["home.beginsIn"].waitForExistence(timeout: 10))
  }

  /// A back swipe begun from a scanned Invite's confirmation and given up shows the scanner for a moment beneath it,
  /// while the confirmation stays pushed: the scanner, which would see another Invite were it scanning, takes nothing
  /// then. Returned to, it scans again and takes that one.
  @MainActor
  func testABackSwipeGivenUpOverAScannedInvitePushesNoSecondConfirmation() {
    let app = launched("fresh", arguments: scanning(invite, "shaman-v1-fake-another") + ["--scan-every", "1"])
    let scan = app.buttons["pairing.scan"]
    XCTAssertTrue(scan.waitForExistence(timeout: 10))
    scan.tap()
    XCTAssertEqual(hostname(in: app), "workstation")

    let edge = app.point(across: 0.01, down: 0.5)
    edge.press(
      forDuration: 0.05, thenDragTo: app.point(across: 0.3, down: 0.5),
      withVelocity: .slow, thenHoldForDuration: 2.5)
    XCTAssertTrue(app.descendants(matching: .any)["confirm.hostname"].waitForExistence(timeout: 5), "swipe given up")
    // The fake scanner, were it scanning while the swipe held, would have held up the next Invite a second after it
    // saw this one again.

    app.back.tap()
    XCTAssertTrue(app.staticTexts["scan.looking"].waitForExistence(timeout: 10), "one back reaches the scanner")
    XCTAssertFalse(app.descendants(matching: .any)["confirm.hostname"].exists, "no second confirmation was pushed")
    XCTAssertEqual(hostname(in: app), "workstation", "the next Invite is taken")
  }

  @MainActor
  func testACameraThatFailsSaysSoAndOffersPaste() {
    let app = launched("fresh", arguments: ["--camera", "fails"])
    let scan = app.buttons["pairing.scan"]
    XCTAssertTrue(scan.waitForExistence(timeout: 10))
    scan.tap()
    let failed = app.staticTexts["scan.failed"]
    XCTAssertTrue(failed.waitForExistence(timeout: 10))
    XCTAssertEqual(failed.label, "The camera stopped scanning. Paste the invite instead.")
    XCTAssertFalse(app.staticTexts["scan.looking"].exists)
    attach("The camera failed")

    UIPasteboard.general.string = invite
    app.buttons["scan.paste"].tap()
    XCTAssertEqual(hostname(in: app), "workstation")
  }

  @MainActor
  func testARefusedCameraSaysSoAndPointsAtSettingsAndPasteStillWorks() {
    let app = launched("fresh", arguments: ["--camera", "refused"])
    let scan = app.buttons["pairing.scan"]
    XCTAssertTrue(scan.waitForExistence(timeout: 10))
    XCTAssertFalse(app.staticTexts["pairing.scan.refused"].exists, "nothing is said before scan is pressed")
    scan.tap()
    let refused = app.staticTexts["pairing.scan.refused"]
    XCTAssertTrue(refused.waitForExistence(timeout: 10))
    XCTAssertEqual(
      refused.label, "Shaman may not use the camera. Allow it in the Settings app to scan, or paste the invite.")
    XCTAssertTrue(app.buttons["pairing.openSettings"].exists)
    XCTAssertFalse(app.staticTexts["scan.looking"].exists, "no scanner opened")
    attach("The camera refused")

    paste(invite, in: app)
    XCTAssertEqual(hostname(in: app), "workstation")
  }

  /// The simulator has no camera, so the camera's own scanner finds none, asking for nothing.
  @MainActor
  func testAPhoneWithNoCameraSaysSoAndPasteStillWorks() {
    let app = launched("fresh")
    let scan = app.buttons["pairing.scan"]
    XCTAssertTrue(scan.waitForExistence(timeout: 10))
    scan.tap()
    let unavailable = app.staticTexts["pairing.scan.unavailable"]
    XCTAssertTrue(unavailable.waitForExistence(timeout: 10))
    XCTAssertEqual(unavailable.label, "This phone has no camera that can scan. Paste the invite instead.")
    attach("No camera to scan with")

    paste(invite, in: app)
    XCTAssertEqual(hostname(in: app), "workstation")
  }

  @MainActor
  func testAnotherServerIsPairedByScanningFromTheServersSheet() {
    let app = launched("paired", arguments: scanning(invite))
    XCTAssertTrue(app.buttons["home.servers"].waitForExistence(timeout: 10))
    app.buttons["home.servers"].tap()
    let pairAnother = app.buttons["servers.pairAnother"]
    XCTAssertTrue(pairAnother.waitForExistence(timeout: 10))
    pairAnother.tap()
    let scan = app.buttons["pairing.scan"]
    XCTAssertTrue(scan.waitForExistence(timeout: 10))
    scan.tap()
    let held = app.descendants(matching: .any)["confirm.held"]
    XCTAssertTrue(held.waitForExistence(timeout: 10))
    XCTAssertTrue(held.label.contains("already paired with this server as workstation"), held.label)
  }

  @MainActor
  func testAnInviteByTheLaunchEnvironmentIsConfirmedAtOnce() {
    let app = launched("fresh", invite: invite)
    XCTAssertEqual(hostname(in: app), "workstation")
  }
}
