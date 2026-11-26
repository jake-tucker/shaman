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
    // As a new scene opens, so that the Outlook is the scenario's, and not one an earlier test's scene kept.
    app.launchArguments = ["--fake", scenario, "--forget-scene"] + arguments
    app.launch()
    return app
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
    XCTAssertTrue(app.cells.firstMatch.waitForExistence(timeout: 5))
  }

  /// Chooses the Workspace `name` on `remote` from the new-session screen, and waits on the screen standing there.
  @MainActor
  private func choose(_ remote: String, _ name: String, in app: XCUIApplication) {
    beginElsewhere(app)
    app.scrolled(to: row(remote, name, in: app)).tap()
    let workspace = element("landing.workspace", in: app)
    let named = NSPredicate(format: "label == %@", name)
    expectation(for: named, evaluatedWith: workspace)
    waitForExpectations(timeout: 5)
  }

  @MainActor
  private func back(_ app: XCUIApplication) {
    app.back.tap()
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
    XCTAssertTrue(row("workstation", "shaman", in: app).exists)
    XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Sidekick")).firstMatch.exists)
    XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Browse")).firstMatch.exists)
    let archive = element("workspace/workstation/photo-archive", in: app)
    XCTAssertTrue(archive.exists)
    XCTAssertTrue(archive.label.contains("Unavailable: its directory is gone"), archive.label)
    XCTAssertFalse(row("workstation", "photo-archive", in: app).exists, "one its Server cannot read is no choice")
    archive.tap()
    XCTAssertTrue(app.navigationBars["Begin in"].exists, "and a tap on it chooses nothing")
    attach("Every remote's workspaces")

    // The home lab's stand further down.
    var read: [String] = []
    for remote in ["workstation", "laptop", "homelab"] {
      app.scrolled(to: headers.matching(NSPredicate(format: "label == %@", remote)).firstMatch)
      // What is drawn now begins with what of the last was still drawn.
      let drawn = headers.allElementsBoundByIndex.map(\.label)
      let again = (0...drawn.count).last { read.suffix($0) == drawn.prefix($0) } ?? 0
      read += drawn.dropFirst(again)
    }
    XCTAssertEqual(read, ["workstation", "laptop", "homelab"], "a header for each Remote")
    app.scrolled(to: row("homelab", "relay-infra", in: app))
    XCTAssertTrue(
      row("homelab", "home-assistant", in: app).isSelected, "the Outlook's Workspace, turned by narrowing, is ticked")
    attach("The outlook's workspace ticked")
    app.scrolled(to: row("laptop", "billing-api", in: app)).tap()
    XCTAssertTrue(element("landing.title", in: app).waitForExistence(timeout: 5))
    XCTAssertEqual(element("landing.place", in: app).label, "laptop · billing-api")
    XCTAssertEqual(element("landing.workspace", in: app).label, "billing-api")
    XCTAssertFalse(element("landing.agent", in: app).exists, "the laptop is too old to say which Agent")

    beginElsewhere(app)
    XCTAssertTrue(app.scrolled(to: row("laptop", "billing-api", in: app)).isSelected)
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
    attach("A search")

    search.typeText("kernel")
    let empty = element("picker.empty", in: app)
    XCTAssertTrue(empty.waitForExistence(timeout: 5))
    XCTAssertEqual(empty.label, "No workspaces found")
    attach("A search that finds nothing")
  }

  @MainActor
  func testAnUnreachableRemotesHeaderSaysSoAndItsWorkspacesStayChoosable() {
    let app = launched("unreachable")
    compose(app)
    beginElsewhere(app)
    let header = app.descendants(matching: .any).matching(identifier: "picker.header")
      .matching(NSPredicate(format: "label CONTAINS %@", "homelab")).firstMatch
    app.scrolled(to: header)
    XCTAssertTrue(header.label.contains("Unreachable"), header.label)
    attach("An unreachable remote's workspaces")
    let assistant = app.scrolled(to: row("homelab", "home-assistant", in: app))
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
    reaching.scrolled(to: loading)
    XCTAssertEqual(loading.label, "Loading workspaces…")
    attach("A remote yet to list its workspaces")
    reaching.terminate()

    let empty = launched("emptyServer")
    compose(empty)
    beginElsewhere(empty)
    let none = element("picker.line", in: empty)
    XCTAssertTrue(none.waitForExistence(timeout: 5))
    XCTAssertEqual(none.label, "No workspaces")
    XCTAssertFalse(element("picker.header", in: empty).exists, "with one Remote, no header")
    attach("A remote that lists no workspaces")
  }

  @MainActor
  func testANewWorktreeIsMadeBeforeTheSessionStartsAndTheChoiceStartsOnTheMainOneEveryTime() {
    let app = launched("composing")
    compose(app)
    let choice = app.segmentedControls["landing.worktree"]
    XCTAssertTrue(choice.waitForExistence(timeout: 5))
    XCTAssertTrue(choice.buttons["On main"].isSelected, "it starts on the main Worktree")
    app.scrolled(to: choice.buttons["In a new worktree"]).tap()
    XCTAssertTrue(choice.buttons["In a new worktree"].isSelected)
    attach("In a new worktree")

    let composer = element("landing.composer", in: app)
    composer.tap()
    composer.typeText("Trim the quiet zone")
    app.buttons["landing.send"].tap()
    let bar = app.descendants(matching: .any).matching(identifier: "session.barWords")
    XCTAssertTrue(bar.waitForOne(NSPredicate(format: "label == %@", "Creating worktree")), "it is made first")
    XCTAssertFalse(element("session.stop", in: app).exists, "no Stop while it is made")
    attach("Creating worktree")
    XCTAssertTrue(bar.waitForOne(NSPredicate(format: "label == %@", "Starting")), "it starts once it is made")
    attach("Starting, once the worktree is made")
    XCTAssertTrue(bar.waitForOne(NSPredicate(format: "label == %@", "Working")), "and then works")

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
    attach("A workspace outside source control")

    choose("workstation", "mirror.git", in: app)
    XCTAssertFalse(app.segmentedControls["landing.worktree"].exists, "a bare repository")
    let line = element("landing.workingCopy", in: app)
    XCTAssertTrue(line.waitForExistence(timeout: 5))
    XCTAssertTrue(line.label.hasPrefix("This workspace is a bare repository."), line.label)
    let composer = element("landing.composer", in: app)
    composer.tap()
    composer.typeText("Mirror the tags")
    XCTAssertFalse(app.buttons["landing.send"].isEnabled, "it has no main worktree to begin on")
    attach("A bare repository")
  }
}
