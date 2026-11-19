import XCTest

/// A Sidekick's tree over the fake: its sheet is headed "Sessions" and stands the Sidekick's Subsessions and the
/// Sessions it acted on, on whichever Server; one on the same Server opens as the top-level Session it is, composer
/// and all; one on another Server the phone is paired with opens there under the phone's own name for it; one on a
/// Server the phone is not paired with opens nothing and says so; one whose Remote the phone cannot reach opens as any
/// Session of an Unreachable Remote does; and a Subsession's row among the Sidekick's Turn's work opens by the same
/// rule.
final class SidekickTests: XCTestCase {
  /// The Sidekick's Session in `sidekick`.
  private let sidekick = "workstation-key/20"

  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  private func launched(opening session: String) -> XCUIApplication {
    let app = XCUIApplication()
    app.launchArguments = ["--fake", "sidekick", "--open", session]
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
  private func element(_ identifier: String, holding text: String, in app: XCUIApplication) -> XCUIElement {
    app.descendants(matching: .any).matching(identifier: identifier)
      .matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
  }

  /// Waits until the open Session's Title reads `expected`.
  @MainActor
  private func titled(_ expected: String, in app: XCUIApplication) {
    let title = app.staticTexts.matching(identifier: "session.title")
      .matching(NSPredicate(format: "label == %@", expected)).firstMatch
    XCTAssertTrue(title.waitForExistence(timeout: 10), "the Session titled \(expected) shows")
  }

  /// Opens the tree's sheet from the navigation bar once the conversation has landed.
  @MainActor
  private func openTree(in app: XCUIApplication) {
    XCTAssertTrue(
      app.descendants(matching: .any).matching(identifier: "session.turn").firstMatch.waitForExistence(timeout: 10))
    let button = app.buttons["session.tree"]
    XCTAssertTrue(button.waitForExistence(timeout: 10))
    button.tap()
    XCTAssertTrue(element("tree.row", holding: "Plan the week", in: app).waitForExistence(timeout: 10))
  }

  @MainActor
  private func back(_ app: XCUIApplication) {
    app.navigationBars.buttons.element(boundBy: 0).tap()
  }

  @MainActor
  func testASidekicksTreeOpensItsSessionsHereAndOnOtherServersThroughThePhonesOwnPairing() {
    let app = launched(opening: sidekick)
    titled("Plan the week", in: app)
    openTree(in: app)
    XCTAssertTrue(app.staticTexts["Sessions"].exists, "a Sidekick's tree is headed Sessions")
    XCTAssertEqual(app.staticTexts["tree.count"].label, "8 · 1 working")
    // Drawn up to its full height, every entry stands on screen.
    element("tree.row", holding: "Plan the week", in: app).swipeUp()
    XCTAssertTrue(element("tree.row", holding: "Back up the photos", in: app).waitForExistence(timeout: 10))

    // Entries read as the Subagents Section's do.
    let crate = element("tree.row", holding: "Map the crate", in: app)
    XCTAssertTrue(crate.label.contains("open for all subagents"), crate.label)
    let unconfirmed = element("tree.row", holding: "Cover the empty input", in: app)
    XCTAssertTrue(unconfirmed.label.contains("not confirmed"), unconfirmed.label)
    let photos = element("tree.row", holding: "Back up the photos", in: app)
    XCTAssertTrue(photos.label.contains("not answering"), photos.label)

    // One on a Server the phone is not paired with is named as the workstation names it and opens nothing.
    let lights = element("tree.row", holding: "Fix the lights", in: app)
    XCTAssertTrue(lights.label.contains("attic"), lights.label)
    XCTAssertTrue(lights.label.contains("home-assistant"), lights.label)
    XCTAssertTrue(lights.label.contains("This phone is not paired with attic"), lights.label)
    XCTAssertFalse(app.buttons.containing(NSPredicate(format: "label CONTAINS %@", "Fix the lights")).firstMatch.exists)
    attach(app, "A sidekick's tree across servers")
    lights.tap()
    XCTAssertTrue(app.otherElements["tree.sheet"].exists, "it opens nothing")

    // One a Server too old to give its Remote's key names says which Server to update.
    element("tree.row", holding: "Back up the photos", in: app).swipeUp()
    let inbox = element("tree.row", holding: "Sort the inbox", in: app)
    XCTAssertTrue(inbox.waitForExistence(timeout: 10))
    XCTAssertTrue(inbox.label.contains("Update workstation to open this here"), inbox.label)

    // One on another Server the phone is paired with is named as the phone names it, and opens there.
    let parser = element("tree.row", holding: "Write the parser", in: app)
    XCTAssertTrue(parser.label.contains("laptop"), parser.label)
    XCTAssertTrue(parser.label.contains("billing-api"), parser.label)
    parser.tap()
    titled("Write the parser", in: app)
    XCTAssertTrue(app.staticTexts["session.place"].label.hasPrefix("laptop"), app.staticTexts["session.place"].label)
    XCTAssertTrue(app.textFields["session.composer"].waitForExistence(timeout: 10), "it takes Prompts")
    back(app)

    // A Subsession on the same Server opens as the top-level Session it is, composer and all, in its Sidekick's tree.
    titled("Plan the week", in: app)
    openTree(in: app)
    element("tree.row", holding: "Tidy the docs", in: app).tap()
    titled("Tidy the docs", in: app)
    XCTAssertTrue(app.textFields["session.composer"].waitForExistence(timeout: 10), "a subsession takes prompts")
    attach(app, "A subsession opened from the tree")
    openTree(in: app)
    XCTAssertTrue(element("tree.row", holding: "Tidy the docs", in: app).isSelected, "its tree is its Sidekick's")
    app.swipeDown(velocity: .fast)
    back(app)

    // One whose Remote the phone cannot reach opens as any Session of an Unreachable Remote does.
    titled("Plan the week", in: app)
    openTree(in: app)
    element("tree.row", holding: "Rotate the relay certs", in: app).tap()
    titled("Rotate the relay certs", in: app)
    let unreachable = element("session.unreachable", holding: "homelab is unreachable", in: app)
    XCTAssertTrue(unreachable.waitForExistence(timeout: 10))
    attach(app, "A session on an unreachable remote opened from the tree")
  }

  @MainActor
  func testASubsessionsRowAmongTheSidekicksWorkOpensByTheSameRule() {
    let app = launched(opening: sidekick)
    titled("Plan the week", in: app)
    let card = app.descendants(matching: .any).matching(identifier: "session.turn").firstMatch
    XCTAssertTrue(card.waitForExistence(timeout: 10))
    card.tap()
    let lights = element("work.row", holding: "Fix the lights", in: app)
    XCTAssertTrue(lights.waitForExistence(timeout: 10))
    XCTAssertTrue(lights.label.contains("This phone is not paired with attic"), lights.label)
    let crate = element("work.row", holding: "Map the crate", in: app)
    XCTAssertTrue(crate.label.contains("laptop"), crate.label)
    attach(app, "A sidekick's work with its subsessions' rows")
    crate.tap()
    titled("Map the crate", in: app)
    XCTAssertTrue(app.textFields["session.composer"].waitForExistence(timeout: 10), "it opens as a top-level session")
    back(app)
    element("work.row", holding: "Tidy the docs", in: app).tap()
    titled("Tidy the docs", in: app)
  }
}
