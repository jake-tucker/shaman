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
    guard let invite = environment["SHAMAN_LIVE_INVITE"], let run = environment["SHAMAN_LIVE_RUN"] else {
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

    let beginsIn = app.staticTexts["home.beginsIn"]
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 30), "paired, and the Outlook turned toward the Server")
    let shown = beginsIn.label
    let empty = app.descendants(matching: .any)["listing.empty"]
    XCTAssertTrue(empty.waitForExistence(timeout: 15), "the scratch Server's listing was read over the Pairing")
    XCTAssertTrue(empty.label.contains("No sessions yet"), empty.label)
    XCTAssertTrue(app.navigationBars["All workspaces"].exists, "one Remote held, so no Origin is named")
    attach(app, "Paired with the scratch Server")
    app.buttons["home.servers"].tap()
    let way = app.descendants(matching: .any).matching(NSPredicate(format: "identifier ENDSWITH '.way.0'")).firstMatch
    XCTAssertTrue(way.waitForExistence(timeout: 10))
    let answering = NSPredicate(format: "label ENDSWITH ', answering'")
    expectation(for: answering, evaluatedWith: way)
    waitForExpectations(timeout: 15)
    attach(app, "The scratch Server with its way answering")

    app.terminate()
    app = launched(run)
    XCTAssertTrue(app.staticTexts["home.beginsIn"].waitForExistence(timeout: 10), "the Pairing survives a relaunch")
    XCTAssertEqual(app.staticTexts["home.beginsIn"].label, shown)
  }
}
