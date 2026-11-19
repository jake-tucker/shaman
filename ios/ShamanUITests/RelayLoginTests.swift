import XCTest

/// Logging in at a Relay a Remote is reached through, over the fake: a Relay way with no login reads "login needed"
/// with "Log in", which shows the code, copies it and opens GitHub in the system's authentication sheet, and closes
/// itself once the relay confirms; dismissing it or leaving the app gives the login up; and the Relays the phone holds
/// a login at are listed with their account and removed.
final class RelayLoginTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(
    _ scenario: String, _ arguments: [String] = [], confirmsAfter seconds: Int = 4, invite: String? = nil
  ) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario, "--login-confirms-after", String(seconds)] + arguments
    if let invite {
      app.launchEnvironment["SHAMAN_INVITE"] = invite
    }
    app.launch()
    return app
  }

  @MainActor
  private func attach(_ app: XCUIApplication, _ name: String) {
    let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    screenshot.name = name
    screenshot.lifetime = .keepAlways
    add(screenshot)
  }

  /// The system's prompt to sign in through github.com, which the authentication sheet opens with.
  private var prompt: XCUIElement {
    XCUIApplication(bundleIdentifier: "com.apple.springboard").alerts.firstMatch
  }

  @MainActor
  private func element(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any)[identifier]
  }

  /// Waits for `element` to go, answering whether it went within `timeout`.
  private func gone(_ element: XCUIElement, within timeout: TimeInterval = 15) -> Bool {
    let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: element)
    return XCTWaiter.wait(for: [expectation], timeout: timeout) == .completed
  }

  /// `element`, scrolled to where it can be tapped above the bottom bar.
  @MainActor
  private func scrolled(to element: XCUIElement, in app: XCUIApplication) -> XCUIElement {
    // The listing is lazy: a row far down stands only once scrolled to.
    _ = app.buttons["home.servers"].waitForExistence(timeout: 10)
    let bar = app.buttons["home.servers"]
    let clear = { element.exists && element.isHittable && (!bar.exists || element.frame.maxY <= bar.frame.minY) }
    var swipes = 0
    while !clear() && swipes < 12 {
      app.swipeUp()
      swipes += 1
    }
    XCTAssertTrue(element.isHittable, "scrolled to \(element)")
    return element
  }

  /// Taps the login offer `identifier` names and waits for the sheet's code, which it copies.
  @MainActor
  private func logIn(_ identifier: String, in app: XCUIApplication) {
    let login = element(identifier, in: app)
    XCTAssertTrue(login.waitForExistence(timeout: 10), identifier)
    login.tap()
    XCTAssertTrue(app.staticTexts["login.code"].waitForExistence(timeout: 10))
    XCTAssertEqual(app.staticTexts["login.code"].label, "WDJB-MJHT")
    XCTAssertTrue(app.descendants(matching: .any)["login.copied"].exists, "the code is copied by itself")
  }

  @MainActor
  func testARelayWayNeedingALoginLogsInInASheetThatClosesItselfOnceTheRelayConfirms() {
    let app = launched("relayLogin", ["--servers-and-device"], confirmsAfter: 6)
    let way = element("server.workstation.way.2", in: app)
    XCTAssertTrue(way.waitForExistence(timeout: 10))
    XCTAssertTrue(way.label.contains("login needed"), way.label)
    XCTAssertFalse(element("relay.relay.example.com", in: app).exists, "no relay is held before a login")
    attach(app, "A relay way needing a login")

    logIn("server.workstation.way.2.login", in: app)
    XCTAssertTrue(app.staticTexts["login.address"].label.contains("github.com/login/device"))
    XCTAssertTrue(app.buttons["login.open"].exists)
    // GitHub opens in the system's authentication sheet, beginning with its prompt.
    XCTAssertTrue(prompt.waitForExistence(timeout: 10), "the authentication sheet opens")
    attach(app, "GitHub opened in the authentication sheet")

    // The relay confirms: the authentication sheet is cancelled and the login's sheet closes itself.
    XCTAssertTrue(gone(prompt), "the authentication sheet is cancelled once the relay confirms")
    XCTAssertTrue(gone(app.staticTexts["login.code"]), "the login's sheet closes itself")
    XCTAssertTrue(way.label.contains("answering"), way.label)
    let relay = element("relay.relay.example.com", in: app)
    XCTAssertTrue(relay.waitForExistence(timeout: 10))
    XCTAssertTrue(relay.label.contains("octocat (github)"), relay.label)
    XCTAssertTrue(relay.label.contains("logged in"), relay.label)
    attach(app, "Logged in, the relay listed with its account")
  }

  @MainActor
  func testTheCodeAndAddressStayOnScreenOnceTheAuthenticationSheetIsClosed() {
    let app = launched("relayLogin", ["--servers-and-device"], confirmsAfter: 60)
    logIn("server.workstation.way.2.login", in: app)
    XCTAssertTrue(prompt.waitForExistence(timeout: 10))
    prompt.buttons["Cancel"].tap()
    XCTAssertTrue(gone(prompt, within: 5))

    XCTAssertTrue(app.staticTexts["login.code"].exists, "the code stays, for approving from another device")
    XCTAssertTrue(app.staticTexts["login.address"].exists)
    XCTAssertTrue(app.staticTexts["login.words"].label.contains("from another device"))
    attach(app, "The code and address, for another device")
  }

  @MainActor
  func testDismissingTheLoginsSheetGivesTheLoginUp() {
    let app = launched("relayLogin", ["--servers-and-device"], confirmsAfter: 60)
    logIn("server.workstation.way.2.login", in: app)
    XCTAssertTrue(prompt.waitForExistence(timeout: 10))
    prompt.buttons["Cancel"].tap()
    XCTAssertTrue(gone(prompt, within: 5))
    let relay = element("relay.relay.example.com", in: app)
    XCTAssertTrue(relay.label.contains("logging in"), relay.label)

    app.buttons["login.cancel"].tap()
    XCTAssertTrue(gone(app.staticTexts["login.code"], within: 5))
    XCTAssertTrue(gone(relay, within: 5), "nothing is kept of a login given up")
    XCTAssertTrue(element("server.workstation.way.2", in: app).label.contains("login needed"))
  }

  @MainActor
  func testLeavingTheAppGivesTheLoginUp() {
    let app = launched("relayLogin", confirmsAfter: 60)
    let servers = app.buttons["home.servers"]
    XCTAssertTrue(servers.waitForExistence(timeout: 10))
    servers.tap()
    logIn("server.workstation.way.2.login", in: app)
    XCTAssertTrue(prompt.waitForExistence(timeout: 10))
    prompt.buttons["Cancel"].tap()
    XCTAssertTrue(gone(prompt, within: 5))

    XCUIDevice.shared.press(.home)
    app.activate()
    let failed = app.staticTexts["login.failed"]
    XCTAssertTrue(failed.waitForExistence(timeout: 10))
    XCTAssertTrue(failed.label.contains("left the screen"), failed.label)
    XCTAssertTrue(app.buttons["login.again"].exists)
    attach(app, "A login given up as the app was left")
  }

  @MainActor
  func testARelayIsRemovedByASwipeOnceConfirmedWhichGivesUpItsLogin() {
    let app = launched("relayed", ["--servers-and-device"])
    let relay = element("relay.relay.example.com", in: app)
    XCTAssertTrue(relay.waitForExistence(timeout: 10))
    XCTAssertTrue(relay.label.contains("octocat (github)"), relay.label)
    XCTAssertTrue(element("server.workstation.way.2", in: app).label.contains("answering"))
    attach(app, "The relays section")

    relay.swipeLeft()
    let remove = app.buttons["relay.relay.example.com.remove"]
    XCTAssertTrue(remove.waitForExistence(timeout: 5))
    remove.tap()
    let confirm = app.buttons["relay.remove.confirm"]
    XCTAssertTrue(confirm.firstMatch.waitForExistence(timeout: 5))
    let asked = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "This phone gives up its login at"))
    XCTAssertTrue(asked.firstMatch.exists, "what removing gives up is said first")
    attach(app, "Removing a relay asks first")
    confirm.firstMatch.tap()

    XCTAssertTrue(gone(relay, within: 5))
    let way = element("server.workstation.way.2", in: app)
    XCTAssertTrue(way.label.contains("login needed"), way.label)
  }

  @MainActor
  func testARemoteOutOfReachForWantOfALoginLeadsToItAndAnswersOnceLoggedIn() {
    let app = launched("loginNeeded")
    let row = scrolled(to: app.buttons["listing.unreachable"], in: app)
    row.tap()
    let explanation = app.staticTexts["unreachable.explanation"]
    XCTAssertTrue(explanation.waitForExistence(timeout: 5), "the sheet explains")
    XCTAssertTrue(explanation.label.contains("needs to log in"), explanation.label)
    let offer = app.buttons["unreachable.tryAgain"]
    XCTAssertEqual(offer.label, "Log in", "trying again leads to the login")
    attach(app, "Unreachable for want of a login")

    offer.tap()
    XCTAssertTrue(app.staticTexts["login.code"].waitForExistence(timeout: 10), "the login's sheet shows its code")
    XCTAssertTrue(gone(app.staticTexts["login.code"]), "the login's sheet closes itself once confirmed")
    XCTAssertTrue(gone(row, within: 10), "the server answers through the relay")
    XCTAssertFalse(explanation.exists, "the sheet goes with the row")
  }

  @MainActor
  func testALoginUnderAnotherAccountReadsAsThePhoneAndTheServerUnderDifferentAccounts() {
    let app = launched("otherAccount")
    let row = scrolled(to: app.buttons["listing.unreachable"], in: app)
    row.tap()
    let explanation = app.staticTexts["unreachable.explanation"]
    XCTAssertTrue(explanation.waitForExistence(timeout: 5), "the sheet explains")
    XCTAssertTrue(
      explanation.label.contains("as hubot (github), and the server under another account"), explanation.label)
    XCTAssertEqual(app.buttons["unreachable.tryAgain"].label, "Log in again")
    attach(app, "Logged in under another account")
  }

  @MainActor
  func testARelayWayNeedingALoginIsPairedPastByADirectWay() {
    let app = launched("fresh", invite: "shaman-v1-fake-workstation-relay")
    let way = element("confirm.way.3", in: app)
    XCTAssertTrue(way.waitForExistence(timeout: 10))
    XCTAssertTrue(way.label.contains("login needed"), way.label)
    XCTAssertTrue(app.buttons["confirm.way.3.login"].exists, "the way offers the login")
    attach(app, "A relay way needing a login beside direct ways")

    app.swipeUp()
    let pair = app.buttons["confirm.pair"]
    XCTAssertTrue(pair.waitForExistence(timeout: 5))
    pair.tap()
    XCTAssertTrue(app.buttons["home.servers"].waitForExistence(timeout: 15), "paired by a direct way, no login asked")
    app.buttons["home.servers"].tap()
    let relayed = element("server.workstation.way.2", in: app)
    XCTAssertTrue(relayed.waitForExistence(timeout: 10))
    XCTAssertTrue(relayed.label.contains("login needed"), relayed.label)
  }

  @MainActor
  func testAPairingThroughARelayAloneAsksForTheLoginAndGoesOnByItselfOnceItIsDone() {
    let app = launched("fresh", invite: "shaman-v1-fake-workstation-relayonly")
    let way = element("confirm.way.0", in: app)
    XCTAssertTrue(way.waitForExistence(timeout: 10))
    XCTAssertTrue(way.label.contains("login needed"), way.label)
    XCTAssertTrue(app.buttons["confirm.way.0.login"].exists, "the way offers the login")
    attach(app, "A relay way needing a login on the confirmation screen")

    app.buttons["confirm.pair"].tap()
    let failure = app.staticTexts["confirm.failure"]
    XCTAssertTrue(failure.waitForExistence(timeout: 10))
    XCTAssertTrue(failure.label.contains("pairing goes on by itself"), failure.label)
    attach(app, "Pairing asks for the login")
    app.buttons["confirm.login"].tap()
    XCTAssertTrue(app.staticTexts["login.code"].waitForExistence(timeout: 10))
    XCTAssertTrue(gone(app.staticTexts["login.code"]), "the login's sheet closes itself once confirmed")

    // Logged in, the same Invite pairs by itself and the app lands on the listing.
    XCTAssertTrue(app.buttons["home.servers"].waitForExistence(timeout: 15), "paired, the listing stands")
  }
}
