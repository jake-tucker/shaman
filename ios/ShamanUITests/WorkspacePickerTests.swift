import XCTest

/// Choosing where a Session begins over the fake: "Begin somewhere else" pushes one searchable list of every Remote's
/// Workspaces, choosing one turns the Outlook and leaves the listing's scope, and the new-session screen offers the
/// main Worktree or a new one, which is made before the Session starts.
final class WorkspacePickerTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(_ scenario: String, _ arguments: [String] = []) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", scenario] + arguments
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

  /// The picker's row of the Workspace `name` on the fake's Remote `remote`.
  @MainActor
  private func row(_ remote: String, _ name: String, in app: XCUIApplication) -> XCUIElement {
    app.buttons["workspace/\(remote)/\(name)"]
  }

  /// Pushes the new-session screen from the listing's compose button.
  @MainActor
  private func compose(_ app: XCUIApplication) {
    let compose = app.buttons["home.compose"]
    XCTAssertTrue(compose.waitForExistence(timeout: 10))
    compose.tap()
    XCTAssertTrue(element("landing.title", in: app).waitForExistence(timeout: 5))
  }

  /// Pushes the Workspace Picker from the new-session screen.
  @MainActor
  private func beginElsewhere(_ app: XCUIApplication) {
    let elsewhere = app.buttons["landing.elsewhere"]
    XCTAssertTrue(elsewhere.waitForExistence(timeout: 5))
    XCTAssertEqual(elsewhere.label, "Begin somewhere else")
    elsewhere.tap()
    XCTAssertTrue(app.navigationBars["Begin in"].waitForExistence(timeout: 5))
  }

  /// Chooses the Workspace `name` on `remote` from the new-session screen, and waits on the screen standing there.
  @MainActor
  private func choose(_ remote: String, _ name: String, in app: XCUIApplication) {
    beginElsewhere(app)
    let chosen = row(remote, name, in: app)
    XCTAssertTrue(chosen.waitForExistence(timeout: 5))
    chosen.tap()
    let workspace = element("landing.workspace", in: app)
    let named = NSPredicate(format: "label == %@", name)
    expectation(for: named, evaluatedWith: workspace)
    waitForExpectations(timeout: 5)
  }

  @MainActor
  private func back(_ app: XCUIApplication) {
    app.navigationBars.buttons.firstMatch.tap()
  }

  @MainActor
  func testThePickerListsEveryRemotesWorkspacesUnderItsHeaderAndChoosingOneTurnsTheOutlookAlone() {
    let app = launched("several")
    // The listing is narrowed to the home lab's home-assistant, which turns the Outlook there, and stays so.
    app.buttons["listing.scope"].tap()
    let lab = app.buttons["home-assistant"].firstMatch
    XCTAssertTrue(lab.waitForExistence(timeout: 10))
    lab.tap()
    let narrowed = app.navigationBars["homelab · home-assistant"]
    XCTAssertTrue(narrowed.waitForExistence(timeout: 10))

    compose(app)
    beginElsewhere(app)
    let headers = app.descendants(matching: .any).matching(identifier: "picker.header")
    XCTAssertTrue(headers.firstMatch.waitForExistence(timeout: 5))
    XCTAssertEqual(
      headers.allElementsBoundByIndex.map(\.label), ["workstation", "laptop", "homelab"], "a header for each Remote")
    XCTAssertTrue(row("workstation", "shaman", in: app).exists)
    XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Sidekick")).firstMatch.exists)
    XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Browse")).firstMatch.exists)
    let archive = row("workstation", "photo-archive", in: app)
    XCTAssertFalse(archive.isEnabled, "one its Server cannot read cannot be chosen")
    XCTAssertTrue(archive.label.contains("Unavailable: its directory is gone"), archive.label)
    attach(app, "Every remote's workspaces")

    // The home lab's stand below the fold.
    app.swipeUp()
    XCTAssertTrue(row("homelab", "relay-infra", in: app).waitForExistence(timeout: 5))
    XCTAssertTrue(
      row("homelab", "home-assistant", in: app).isSelected, "the Outlook's Workspace, turned by narrowing, is ticked")
    attach(app, "The outlook's workspace ticked")
    XCTAssertTrue(row("laptop", "billing-api", in: app).exists)
    row("laptop", "billing-api", in: app).tap()
    XCTAssertTrue(element("landing.title", in: app).waitForExistence(timeout: 5))
    XCTAssertEqual(element("landing.place", in: app).label, "laptop · billing-api")
    XCTAssertEqual(element("landing.workspace", in: app).label, "billing-api")
    XCTAssertFalse(element("landing.agent", in: app).exists, "the laptop is too old to say which Agent")

    beginElsewhere(app)
    XCTAssertTrue(row("laptop", "billing-api", in: app).waitForExistence(timeout: 5))
    XCTAssertTrue(row("laptop", "billing-api", in: app).isSelected)
    back(app)
    XCTAssertTrue(element("landing.title", in: app).waitForExistence(timeout: 5))
    back(app)
    XCTAssertTrue(narrowed.waitForExistence(timeout: 5), "the listing's scope stays where it was")
    XCTAssertEqual(
      element("home.beginsIn", in: app).label, "New sessions begin in laptop · billing-api", "the Outlook turned")
  }

  @MainActor
  func testThePickerIsSearchedByNameAndSaysWhereItFindsNothing() {
    let app = launched("several")
    compose(app)
    beginElsewhere(app)
    let search = app.searchFields.firstMatch
    XCTAssertTrue(search.waitForExistence(timeout: 5))
    XCTAssertEqual(search.placeholderValue, "Search workspaces")
    search.tap()
    search.typeText("home")
    let gone = NSPredicate(format: "exists == false")
    expectation(for: gone, evaluatedWith: row("workstation", "shaman", in: app))
    waitForExpectations(timeout: 5)
    XCTAssertTrue(row("homelab", "home-assistant", in: app).exists)
    attach(app, "A search")

    search.typeText("kernel")
    let empty = element("picker.empty", in: app)
    XCTAssertTrue(empty.waitForExistence(timeout: 5))
    XCTAssertEqual(empty.label, "No workspaces found")
    attach(app, "A search that finds nothing")
  }

  @MainActor
  func testAnUnreachableRemotesHeaderSaysSoAndItsWorkspacesStayChoosable() {
    let app = launched("unreachable")
    compose(app)
    beginElsewhere(app)
    let header = app.descendants(matching: .any).matching(identifier: "picker.header")
      .matching(NSPredicate(format: "label CONTAINS %@", "homelab")).firstMatch
    XCTAssertTrue(header.waitForExistence(timeout: 5))
    XCTAssertTrue(header.label.contains("Unreachable"), header.label)
    attach(app, "An unreachable remote's workspaces")
    let assistant = row("homelab", "home-assistant", in: app)
    XCTAssertTrue(assistant.isEnabled)
    assistant.tap()
    let line = element("landing.unreachable", in: app)
    XCTAssertTrue(line.waitForExistence(timeout: 5), "turned toward it, where nothing begins until it answers")
    XCTAssertEqual(element("landing.agent", in: app).label, "Agent unavailable", "what the home lab says, read anew")
  }

  @MainActor
  func testAPickerSaysWhereARemoteHasYetToListItsWorkspacesAndWhereOneListsNone() {
    let reaching = launched("reaching")
    compose(reaching)
    beginElsewhere(reaching)
    let loading = element("picker.line", in: reaching)
    XCTAssertTrue(loading.waitForExistence(timeout: 5))
    XCTAssertEqual(loading.label, "Loading workspaces…")
    attach(reaching, "A remote yet to list its workspaces")
    reaching.terminate()

    let empty = launched("emptyServer")
    compose(empty)
    beginElsewhere(empty)
    let none = element("picker.line", in: empty)
    XCTAssertTrue(none.waitForExistence(timeout: 5))
    XCTAssertEqual(none.label, "No workspaces")
    XCTAssertFalse(element("picker.header", in: empty).exists, "with one Remote, no header")
    attach(empty, "A remote that lists no workspaces")
  }

  @MainActor
  func testANewWorktreeIsMadeBeforeTheSessionStartsAndTheChoiceStartsOnTheMainOneEveryTime() {
    let app = launched("composing")
    compose(app)
    let choice = app.segmentedControls["landing.worktree"]
    XCTAssertTrue(choice.waitForExistence(timeout: 5))
    XCTAssertTrue(choice.buttons["On main"].isSelected, "it starts on the main Worktree")
    choice.buttons["In a new worktree"].tap()
    XCTAssertTrue(choice.buttons["In a new worktree"].isSelected)
    attach(app, "In a new worktree")

    let composer = element("landing.composer", in: app)
    composer.tap()
    composer.typeText("Trim the quiet zone")
    app.buttons["landing.send"].tap()
    let bar = element("session.barWords", in: app)
    XCTAssertTrue(bar.waitForExistence(timeout: 5))
    XCTAssertEqual(bar.label, "Creating worktree")
    XCTAssertFalse(element("session.stop", in: app).exists, "no Stop while it is made")
    attach(app, "Creating worktree")
    expectation(for: NSPredicate(format: "label == %@", "Starting"), evaluatedWith: bar)
    waitForExpectations(timeout: 10)
    attach(app, "Starting, once the worktree is made")
    expectation(for: NSPredicate(format: "label == %@", "Working"), evaluatedWith: bar)
    waitForExpectations(timeout: 10)

    back(app)
    compose(app)
    XCTAssertTrue(choice.waitForExistence(timeout: 5))
    XCTAssertTrue(choice.buttons["On main"].isSelected, "and on the main Worktree again, as the screen opens again")
  }

  @MainActor
  func testAWorkspaceOutsideSourceControlOffersNoWorktreeAndABareRepositoryNoSend() {
    let app = launched("composing")
    compose(app)
    choose("workstation", "notes", in: app)
    XCTAssertFalse(app.segmentedControls["landing.worktree"].exists, "outside source control")
    XCTAssertFalse(element("landing.workingCopy", in: app).exists)
    attach(app, "A workspace outside source control")

    choose("workstation", "mirror.git", in: app)
    XCTAssertFalse(app.segmentedControls["landing.worktree"].exists, "a bare repository")
    let line = element("landing.workingCopy", in: app)
    XCTAssertTrue(line.waitForExistence(timeout: 5))
    XCTAssertTrue(line.label.hasPrefix("This workspace is a bare repository."), line.label)
    let composer = element("landing.composer", in: app)
    composer.tap()
    composer.typeText("Mirror the tags")
    XCTAssertFalse(app.buttons["landing.send"].isEnabled, "it has no main worktree to begin on")
    attach(app, "A bare repository")
  }
}
