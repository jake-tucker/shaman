import XCTest

/// Pairing the live core with a real Serving Server: run only by `scripts/ios-pair-scratch-server.sh --test`, which
/// starts a scratch Server and hands the test its Invite, and never in CI, where no Server runs.
final class LivePairingTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ run: String, invite: String? = nil, arguments: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchEnvironment["SHAMAN_CORE_DIRECTORY"] = run
    app.launchEnvironment["SHAMAN_KEY_ITEM"] = run
    if let invite {
      app.launchEnvironment["SHAMAN_INVITE"] = invite
    }
    app.launchArguments = arguments
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
  func testAnInviteFromAScratchServerPairsAndThePairingSurvivesARelaunch() throws {
    let environment = ProcessInfo.processInfo.environment
    guard let invite = environment["SHAMAN_LIVE_INVITE"], let run = environment["SHAMAN_LIVE_RUN"],
      let prompt = environment["SHAMAN_LIVE_PROMPT"]
    else {
      throw XCTSkip("Run by scripts/ios-pair-scratch-server.sh --test, with a scratch Server's invite")
    }
    addTeardownBlock { @MainActor in
      let removing = self.launched(run, arguments: ["--remove-core-directory"])
      XCTAssertTrue(removing.staticTexts["core.removed"].waitForExistence(timeout: 10))
      removing.terminate()
    }

    var app = launched(run, invite: invite)
    let hostname = app.descendants(matching: .any)["confirm.hostname"]
    XCTAssertTrue(hostname.waitForExistence(timeout: 10))
    attach(app, "A scratch Server's invite")
    let pair = app.buttons["confirm.pair"]
    var swipes = 0
    while !pair.isHittable && swipes < 5 {
      app.swipeUp()
      swipes += 1
    }
    pair.tap()

    let beginsIn = app.descendants(matching: .any)["home.beginsIn"]
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 30), "paired, and the Outlook turned toward the Server")
    let row = app.descendants(matching: .any).matching(identifier: "listing.active").firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 15), "the scratch Server's listing was read over the Pairing")
    XCTAssertTrue(app.navigationBars["All workspaces"].exists, "one Remote held, so no Origin is named")
    attach(app, "Paired with the scratch Server")

    // The Session the script began there opens, and is read live over the Pairing.
    let paired = beginsIn.label
    row.tap()
    XCTAssertTrue(app.descendants(matching: .any)["session.title"].waitForExistence(timeout: 10))
    let bubble = app.descendants(matching: .any).matching(identifier: "session.userMessage").firstMatch
    XCTAssertTrue(bubble.waitForExistence(timeout: 30), "the conversation was read")
    XCTAssertEqual(bubble.label, prompt)
    let place = app.descendants(matching: .any)["session.place"]
    XCTAssertEqual(place.label, "workspace", "its Workspace, and no Origin")
    let answered = app.descendants(matching: .any).matching(
      NSPredicate(format: "identifier IN {'session.agentMessage', 'session.error', 'session.turn'}")
    ).firstMatch
    XCTAssertTrue(answered.waitForExistence(timeout: 90), "its Turn was read as it went")
    attach(app, "A session on the scratch Server, read live")
    app.navigationBars.buttons.firstMatch.tap()
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn.label, "\(paired) · workspace", "opening it turned the Outlook toward its Workspace")
    let shown = beginsIn.label
    app.buttons["home.servers"].tap()
    let way = app.descendants(matching: .any).matching(NSPredicate(format: "identifier ENDSWITH '.way.0'")).firstMatch
    XCTAssertTrue(way.waitForExistence(timeout: 10))
    let answering = NSPredicate(format: "label ENDSWITH ', answering'")
    expectation(for: answering, evaluatedWith: way)
    waitForExpectations(timeout: 15)
    attach(app, "The scratch Server with its way answering")

    app.terminate()
    app = launched(run)
    XCTAssertTrue(
      app.descendants(matching: .any)["home.beginsIn"].waitForExistence(timeout: 10), "the Pairing survives a relaunch")
    XCTAssertEqual(app.descendants(matching: .any)["home.beginsIn"].label, shown)
  }
}
