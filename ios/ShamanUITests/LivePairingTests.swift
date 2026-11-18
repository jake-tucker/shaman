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
  func testAnInviteFromAScratchServerPairsReadsAndSendsAndThePairingSurvivesARelaunch() throws {
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

    // A Prompt sent from the composer goes over the Pairing, and stands as the Server holds it.
    let again = "Reply with the single word again, and nothing else. Use no tools."
    let composer = app.descendants(matching: .any)["session.composer"]
    XCTAssertTrue(composer.waitForExistence(timeout: 10))
    composer.tap()
    composer.typeText(again)
    app.descendants(matching: .any)["session.send"].tap()
    let sent = app.descendants(matching: .any).matching(identifier: "session.userMessage")
      .matching(NSPredicate(format: "label == %@", again)).firstMatch
    XCTAssertTrue(sent.waitForExistence(timeout: 30), "sent, it stands as the Server holds it")
    XCTAssertFalse(app.descendants(matching: .any)["banner"].exists, "and nothing failed")
    attach(app, "A prompt sent to the scratch Server")
    app.navigationBars.buttons.firstMatch.tap()
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn.label, "\(paired) · workspace", "opening it turned the Outlook toward its Workspace")

    // The title's menu and the search are the live core's: narrowing to the Session's Workspace lists it alone.
    let menu = app.buttons["listing.scope"]
    XCTAssertEqual(menu.label, "All workspaces")
    menu.tap()
    let workspace = app.buttons["workspace"].firstMatch
    XCTAssertTrue(workspace.waitForExistence(timeout: 10), "the scratch Server's Workspace is offered")
    attach(app, "The scratch Server's title menu")
    workspace.tap()
    XCTAssertTrue(app.navigationBars["workspace"].waitForExistence(timeout: 10))
    XCTAssertTrue(row.waitForExistence(timeout: 10))
    let field = app.searchFields.firstMatch
    XCTAssertEqual(field.placeholderValue, "Search by title")
    field.tap()
    field.typeText("nothing like it")
    let empty = app.descendants(matching: .any)["listing.empty"]
    XCTAssertTrue(empty.waitForExistence(timeout: 10))
    XCTAssertEqual(empty.label, "No sessions match")
    attach(app, "A search of the scratch Server")
    app.buttons["Close"].firstMatch.tap()
    XCTAssertEqual(beginsIn.label, "\(paired) · workspace")
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

    try unreachableAndBack(app, signals: environment["SHAMAN_LIVE_SIGNALS"])
  }

  /// The scratch Server stopped mid-run reads Unreachable, its rows dimmed beneath a slim row, and once started again
  /// it answers, trying it again where it has not yet by itself; and a switch away and back keeps it answering. The
  /// script stops and starts the Server as the test asks through files in `signals`.
  @MainActor
  private func unreachableAndBack(_ app: XCUIApplication, signals: String?) throws {
    guard let signals else { return }
    let slim = app.buttons["listing.unreachable"]
    XCTAssertTrue(app.descendants(matching: .any).matching(identifier: "listing.active").firstMatch.exists)
    try signal("stop", in: signals, awaiting: "stopped")
    XCTAssertTrue(slim.waitForExistence(timeout: 30), "the stopped Server reads Unreachable")
    XCTAssertTrue(slim.label.contains("Server unreachable"), slim.label)
    XCTAssertTrue(
      app.descendants(matching: .any).matching(identifier: "listing.active").firstMatch.exists, "its rows stay")
    attach(app, "The scratch Server stopped, read Unreachable")

    try signal("resume", in: signals, awaiting: "resumed")
    if slim.exists {
      slim.tap()
      let tryAgain = app.buttons["unreachable.tryAgain"]
      if tryAgain.waitForExistence(timeout: 5) {
        attach(app, "The scratch Server's offer to try again")
        tryAgain.tap()
      }
    }
    XCTAssertTrue(slim.waitForNonExistence(timeout: 30), "the Server started again answers")
    attach(app, "The scratch Server answering again")

    XCUIDevice.shared.press(.home)
    app.activate()
    XCTAssertTrue(app.descendants(matching: .any)["home.beginsIn"].waitForExistence(timeout: 10))
    XCTAssertFalse(slim.waitForExistence(timeout: 3), "a switch away and back reads no Remote Unreachable")
  }

  /// Asks the script for `asked` by a file in `signals`, and waits for it to say `done` the same way.
  private func signal(_ asked: String, in signals: String, awaiting done: String) throws {
    let directory = URL(filePath: signals, directoryHint: .isDirectory)
    try Data().write(to: directory.appending(path: asked))
    let answered = directory.appending(path: done)
    let deadline = Date().addingTimeInterval(60)
    while !FileManager.default.fileExists(atPath: answered.path(percentEncoded: false)) {
      guard Date() < deadline else {
        XCTFail("the script never said \(done)")
        return
      }
      Thread.sleep(forTimeInterval: 0.2)
    }
  }
}
