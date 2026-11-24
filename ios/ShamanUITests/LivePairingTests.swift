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
    attach("A scratch Server's invite")
    app.scrolled(to: app.buttons["confirm.pair"]).tap()

    let beginsIn = app.descendants(matching: .any)["home.beginsIn"]
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 30), "paired, and the Outlook turned toward the Server")
    let row = app.descendants(matching: .any).matching(identifier: "listing.active").firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 15), "the scratch Server's listing was read over the Pairing")
    XCTAssertTrue(app.navigationBars["All workspaces"].exists, "one Remote held, so no Origin is named")
    attach("Paired with the scratch Server")

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
    attach("A session on the scratch Server, read live")
    readWork(app)

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
    attach("A prompt sent to the scratch Server")
    app.back.tap()
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn.label, "\(paired) · workspace", "opening it turned the Outlook toward its Workspace")

    // The title's menu and the search are the live core's: narrowing to the Session's Workspace lists it alone.
    let menu = app.buttons["listing.scope"]
    XCTAssertEqual(menu.label, "All workspaces")
    menu.tap()
    let workspace = app.buttons["workspace"].firstMatch
    XCTAssertTrue(workspace.waitForExistence(timeout: 10), "the scratch Server's Workspace is offered")
    attach("The scratch Server's title menu")
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
    attach("A search of the scratch Server")
    app.buttons["Close"].firstMatch.tap()
    XCTAssertEqual(beginsIn.label, "\(paired) · workspace")
    let shown = beginsIn.label
    beginASession(app)
    app.buttons["home.servers"].tap()
    let way = app.descendants(matching: .any).matching(NSPredicate(format: "identifier ENDSWITH '.way.0'")).firstMatch
    XCTAssertTrue(way.waitForExistence(timeout: 10))
    let answering = NSPredicate(format: "label ENDSWITH ', answering'")
    expectation(for: answering, evaluatedWith: way)
    waitForExpectations(timeout: 15)
    attach("The scratch Server with its way answering")

    app.terminate()
    app = launched(run)
    XCTAssertTrue(
      app.descendants(matching: .any)["home.beginsIn"].waitForExistence(timeout: 10), "the Pairing survives a relaunch")
    XCTAssertEqual(app.descendants(matching: .any)["home.beginsIn"].label, shown)
    app = keptADraft(app, run)

    try unreachableAndBack(app, signals: environment["SHAMAN_LIVE_SIGNALS"])
    renameAndRemove(app)
  }

  /// The scratch Server renamed from its row keeps its Pairing, the Outlook going with the name; removed by a swipe, it
  /// is asked to forget this phone, which the script checks after, and the phone, holding no Remote, offers pairing
  /// alone.
  @MainActor
  private func renameAndRemove(_ app: XCUIApplication) {
    app.buttons["home.servers"].tap()
    let field = app.textFields.matching(NSPredicate(format: "identifier BEGINSWITH 'server.'")).firstMatch
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    let name = field.value as? String ?? ""
    field.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
    field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: name.count) + "renamed\n")
    let renamed = app.textFields["server.renamed"]
    XCTAssertTrue(renamed.waitForExistence(timeout: 10), "renamed from its row")
    let way = app.descendants(matching: .any)["server.renamed.way.0"]
    expectation(for: NSPredicate(format: "label ENDSWITH ', answering'"), evaluatedWith: way)
    waitForExpectations(timeout: 15)
    attach("The scratch Server renamed, still answering")
    app.navigationBars["Servers & device"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .press(
        forDuration: 0.1, thenDragTo: app.point(across: 0.5, down: 0.98), withVelocity: .fast, thenHoldForDuration: 0)
    let beginsIn = app.descendants(matching: .any)["home.beginsIn"]
    XCTAssertTrue(beginsIn.waitForExistence(timeout: 10))
    XCTAssertEqual(beginsIn.label, "New sessions begin in renamed · workspace", "the Outlook goes with the name")

    app.buttons["home.servers"].tap()
    XCTAssertTrue(renamed.waitForExistence(timeout: 10))
    renamed.swipeLeft()
    app.buttons["server.renamed.remove"].tap()
    let confirm = app.buttons["server.remove.confirm"].firstMatch
    XCTAssertTrue(confirm.waitForExistence(timeout: 5))
    confirm.tap()
    XCTAssertTrue(app.buttons["pairing.paste"].waitForExistence(timeout: 15), "holding no Remote, pairing alone")
    attach("The scratch Server removed")
  }

  /// What the new-session screen's composer held as the app left the front is in it once iOS has ended the app and it
  /// is launched again, the live core having kept it in its directory; and back leaves the listing.
  @MainActor
  private func keptADraft(_ app: XCUIApplication, _ run: String) -> XCUIApplication {
    let unsent = "Find why the relay drops idle joins"
    app.buttons["home.compose"].tap()
    var field = app.descendants(matching: .any)["landing.composer"]
    XCTAssertTrue(field.waitForExistence(timeout: 10))
    field.tap()
    field.typeText(unsent)
    app.leaveTheFront()
    // iOS writes what the scene keeps a moment after the app leaves the front.
    _ = XCTWaiter.wait(for: [XCTestExpectation(description: "the scene kept")], timeout: 2)
    app.terminate()

    let relaunched = launched(run)
    field = relaunched.descendants(matching: .any)["landing.composer"]
    XCTAssertTrue(field.waitForExistence(timeout: 10), "the new-session screen is open again")
    let holding = NSPredicate(format: "value == %@", unsent)
    XCTAssertTrue(
      XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: holding, object: field)], timeout: 10) == .completed,
      "the unsent text was kept, not \(String(describing: field.value))")
    attach("Unsent text kept by the live core after the app was ended")
    relaunched.back.tap()
    XCTAssertTrue(relaunched.descendants(matching: .any)["home.beginsIn"].waitForExistence(timeout: 10))
    return relaunched
  }

  /// The compose button pushes the new-session screen where the Outlook stands, the Workspace of the Session opened
  /// before; a first Prompt begins a Session there through the live core, the screen turning into it, and back leaves
  /// one more Session listed in that Workspace.
  @MainActor
  private func beginASession(_ app: XCUIApplication) {
    let rows = app.descendants(matching: .any).matching(identifier: "listing.active")
    XCTAssertTrue(rows.firstMatch.waitForExistence(timeout: 10))
    let listed = rows.count
    app.buttons["home.compose"].tap()
    let workspace = app.descendants(matching: .any)["landing.workspace"]
    XCTAssertTrue(workspace.waitForExistence(timeout: 10), "the Landing names where it begins")
    XCTAssertEqual(workspace.label, "workspace")
    XCTAssertEqual(app.descendants(matching: .any)["landing.place"].label, "workspace", "with one Remote no Origin")
    XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10), "the keyboard is up on arrival")
    let agent = app.descendants(matching: .any)["landing.agent"]
    XCTAssertTrue(agent.waitForExistence(timeout: 15), "the scratch Server says what a new Session would get")
    XCTAssertFalse(app.segmentedControls["landing.worktree"].exists, "its Workspace is outside source control")
    attach("The new-session screen on the scratch Server")

    // The Workspace Picker lists the scratch Server's Workspaces through the live core, the Outlook's ticked.
    app.buttons["landing.elsewhere"].tap()
    XCTAssertTrue(app.navigationBars["Begin in"].waitForExistence(timeout: 10))
    let ticked = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "workspace")).firstMatch
    XCTAssertTrue(ticked.waitForExistence(timeout: 15), "the picker lists the Workspace")
    XCTAssertTrue(ticked.isSelected, "the Outlook's Workspace is ticked")
    attach("The workspace picker on the scratch Server")
    ticked.tap()
    XCTAssertTrue(workspace.waitForExistence(timeout: 10), "chosen, back on the new-session screen")
    XCTAssertEqual(workspace.label, "workspace")

    let prompt = "Reply with the single word begun, and nothing else. Use no tools."
    let field = app.descendants(matching: .any)["landing.composer"]
    field.tap()
    field.typeText(prompt)
    app.buttons["landing.send"].tap()

    XCTAssertTrue(app.descendants(matching: .any)["session.title"].waitForExistence(timeout: 10), "the screen turned")
    let begun = app.descendants(matching: .any).matching(identifier: "session.userMessage")
      .matching(NSPredicate(format: "label == %@", prompt)).firstMatch
    XCTAssertTrue(begun.waitForExistence(timeout: 30), "the scratch Server began it, and it stands in its place")
    XCTAssertFalse(app.descendants(matching: .any)["session.refused"].exists, "and it was not refused")
    attach("A session begun on the scratch Server")

    app.back.tap()
    XCTAssertTrue(app.buttons["home.compose"].waitForExistence(timeout: 10))
    let more = NSPredicate(format: "count == %d", listed + 1)
    expectation(for: more, evaluatedWith: rows)
    waitForExpectations(timeout: 30)
  }

  /// Where the Session's Agent did some work, its card pushes that work as the live core fills it, and a row pushes
  /// the whole of its Activity. Where the Agent could not run, as where it is not signed in, its Turn failed with no
  /// work and no card stands, which is said and passed over.
  @MainActor
  private func readWork(_ app: XCUIApplication) {
    let card = app.descendants(matching: .any).matching(identifier: "session.turn").firstMatch
    let failed = app.descendants(matching: .any).matching(identifier: "session.error").firstMatch
    guard card.exists || (!failed.exists && card.waitForExistence(timeout: 60)) else {
      XCTContext.runActivity(named: "No card: the scratch Server's Agent did no work, so none is read") { _ in }
      return
    }
    card.tap()
    let row = app.descendants(matching: .any).matching(identifier: "work.row").firstMatch
    XCTAssertTrue(row.waitForExistence(timeout: 30), "the Turn's work crossed from the live core")
    attach("A turn's work on the scratch Server")
    if row.elementType == .button {
      row.tap()
      XCTAssertTrue(
        app.descendants(matching: .any)["detail.screen"].waitForExistence(timeout: 10), "its Activity's whole")
      XCTAssertTrue(
        app.descendants(matching: .any).matching(identifier: "detail.fact").firstMatch.waitForExistence(timeout: 10))
      attach("An activity on the scratch Server")
      app.back.tap()
    }
    app.back.tap()
    XCTAssertTrue(app.descendants(matching: .any)["session.composer"].waitForExistence(timeout: 10))
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
    attach("The scratch Server stopped, read Unreachable")

    // Tried again while it is still stopped, it says why in the banner and the sheet stays up.
    slim.tap()
    let tryAgain = app.buttons["unreachable.tryAgain"]
    XCTAssertTrue(tryAgain.waitForExistence(timeout: 10))
    tryAgain.tap()
    let banner = app.descendants(matching: .any)["banner"]
    XCTAssertTrue(banner.waitForExistence(timeout: 15), "a try that reached nothing says so")
    XCTAssertTrue(banner.label.contains("No way to the server answered"), banner.label)
    attach("The scratch Server tried again while stopped")

    // Started again, it answers by itself, and its row takes the sheet with it.
    try signal("resume", in: signals, awaiting: "resumed")
    XCTAssertTrue(tryAgain.waitForNonExistence(timeout: 30), "the sheet goes with the row")
    XCTAssertTrue(slim.waitForNonExistence(timeout: 30), "the Server started again answers")
    attach("The scratch Server answering again")

    app.leaveTheFront()
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
